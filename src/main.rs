//! Talk to a Claude Code session running on another machine.
//!
//!     outpost status
//!     outpost read 20
//!     outpost pane
//!     outpost send "yes, retry"
//!     echo "..." | outpost send -
//!
//! Where it points comes from `OUTPOST_HOST` and `OUTPOST_WINDOW`; nothing
//! about any particular machine is written down here.
//!
//! **This is deliberately the whole interface.** Driving a remote container by
//! hand means an ssh session with everything in it — the process table, the
//! caches, the logs — and the standing instruction is to talk to the session
//! rather than go and look. An instruction like that rests on restraint for
//! exactly as long as the blunt instrument is still in reach. Four verbs that
//! read a conversation and type into a composer are the whole job, so they are
//! the whole tool.
//!
//! ⚠ **A message sent from here arrives as the account's owner**, with nothing
//! marking it as drafted. Never send unasked, and match how they actually
//! write rather than composing fresh prose in their name.

use anyhow::{Context, Result, bail};
use outpost::args::{flag_of, take_target, timeout_of};
use outpost::remote::Remote;
use outpost::transcript::{Turn, clock, conversation, endings, spoken};
use reader::transcript::human_turns;
use std::collections::HashSet;
use std::io::Read;

/// How much of a message to show before saying how much was kept back.
///
/// ⚠ **Never truncate silently.** A cut that leaves no mark cannot be told from
/// a message that was short.
const WIDTH: usize = 700;

/// How long to wait for the CLI to pick a sent message up, and how often to ask.
///
/// ⚠ **Bounded, because the failure it is watching for is silence.** A session
/// mid-turn queues the message instead of reading it, and a session that has
/// stopped reading its input looks exactly the same from out here — for the
/// first few seconds.
const PATIENCE: std::time::Duration = std::time::Duration::from_secs(20);
const INTERVAL: std::time::Duration = std::time::Duration::from_millis(750);

/// How often `wait` asks. The ceiling it stops at is `args::WAIT_FOR`.
///
/// ⚠ **A poll costs an ssh round trip and a transcript tail**, so asking often
/// is not free — and the thing being waited for is usually a build measured in
/// tens of minutes.
const WAIT_EVERY: std::time::Duration = std::time::Duration::from_secs(15);

const USAGE: &str = "usage:
  outpost status            what the session says it is doing, and the windows
  outpost read [n]          the last n exchanges, both sides   (default 12)
  outpost pane [n]          the window itself, with n lines of scrollback
  outpost send <text>       type it and press Enter, as them; `-` reads stdin
  outpost wait              block until it says something new, then print it

  -t <name>       which target, from the config file
  --full          with `read`, do not shorten long messages
  --wait          with `send`, wait for the reply and print it
  --timeout <s>   with `wait`, how long to sit there (default 3600)
  --match <text>  with `wait`, ignore turns that do not contain it
  --task [text]   with `wait`, wait for a background task to END, not for prose
  --idle          with `wait`, wait until it stops working, not for what it says

targets live in ~/.config/outpost/config.toml:

    default = \"dev\"

    [targets.dev]
    host = \"<ssh destination>\"
    window = \"<tmux session:window>\"";

fn main() -> Result<()> {
    // ⚠ **Rust ignores SIGPIPE, and `println!` then PANICS on a closed pipe.**
    // `outpost read 50 | head` is the ordinary way to use this, and it died with
    // a backtrace instead of stopping quietly. Restoring the default disposition
    // is what every other command-line program does.
    //
    // Safe because it runs before any thread exists and only resets a signal to
    // the behaviour the process would have had without Rust's startup code.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    let all: Vec<&str> = args.iter().map(String::as_str).collect();
    // `-t` is pulled out before dispatch so every verb accepts it in the same
    // place, and so a verb's own argument parsing never has to know about it.
    let (target, rest) = take_target(&all)?;
    let target = target.as_deref();
    match rest.split_first() {
        None | Some((&"status", [])) => status(target),
        Some((&"read", tail)) => read(target, tail),
        Some((&"pane", tail)) => pane(target, tail),
        Some((&"send", tail)) => send(target, tail),
        Some((&"wait", tail)) => wait(target, tail),
        Some((&("-h" | "--help" | "help"), _)) => {
            println!("{USAGE}");
            Ok(())
        }
        Some((other, _)) => bail!("no such command {other:?}\n\n{USAGE}"),
    }
}

fn status(target: Option<&str>) -> Result<()> {
    let far = Remote::resolve(target)?;
    let info = far.info()?;
    let bridged = match &info.bridge {
        Some(_) => "bridged",
        None => "not bridged",
    };
    println!(
        "{} — {} — claude {} — pid {} — {bridged}",
        info.name.as_deref().unwrap_or("(unnamed)"),
        info.status,
        info.version,
        info.pid,
    );
    println!("{}", info.session_id);
    // The windows are the other half of "is it all still up": a session that is
    // idle because the bots died is not the same as a session that is idle.
    match far.windows() {
        Ok(windows) if !windows.is_empty() => {
            for (name, command) in windows {
                println!("  {name:<20} {command}");
            }
        }
        Ok(_) => println!("  (tmux is running but has no windows)"),
        Err(err) => println!("  windows unavailable: {err}"),
    }
    Ok(())
}

fn pane(target: Option<&str>, args: &[&str]) -> Result<()> {
    let back: usize = match args.first() {
        Some(n) => n.parse().context("that is not a number of lines")?,
        None => 0,
    };
    print!("{}", Remote::resolve(target)?.pane(back)?);
    Ok(())
}

fn read(target: Option<&str>, args: &[&str]) -> Result<()> {
    let full = args.contains(&"--full");
    let want: usize = args
        .iter()
        .find(|arg| !arg.starts_with("--"))
        .map_or(Ok(12), |n| n.parse())
        .context("that is not a number of messages")?;

    let far = Remote::resolve(target)?;
    let info = far.info()?;
    let bytes = far.transcript(&info.session_id)?;
    let lines = conversation(&bytes);

    let shown = want.min(lines.len());
    for line in &lines[lines.len() - shown..] {
        let text = if full || line.text.chars().count() <= WIDTH {
            line.text.clone()
        } else {
            let kept: String = line.text.chars().take(WIDTH).collect();
            let left = line.text.chars().count() - WIDTH;
            format!("{kept}… (+{left} chars, --full for all)")
        };
        println!("{}  {}  {text}\n", clock(&line.at), line.who);
    }
    if lines.len() > shown {
        println!("({} earlier, ask for more)", lines.len() - shown);
    }
    Ok(())
}

fn send(target: Option<&str>, args: &[&str]) -> Result<()> {
    if args.is_empty() {
        bail!("usage: outpost send <text>");
    }
    let text = if args == ["-"] {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        buf
    } else {
        // ⚠ **Flags are not message text.** Without this, `send "x" --wait`
        // types the word "--wait" into the composer and sends it.
        let mut words = Vec::new();
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            match *arg {
                "--wait" => {}
                "--timeout" | "--match" => {
                    it.next();
                }
                other => words.push(other),
            }
        }
        words.join(" ")
    };
    let text = text.trim().to_string();
    if text.is_empty() {
        bail!("nothing to send");
    }
    // ⚠ **A newline in the composer is Enter.** Typing a two-line message would
    // submit the first line on its own and leave the second half in the box.
    // Refusing is the honest answer; silently joining the lines would send
    // something nobody wrote.
    if text.contains('\n') {
        bail!(
            "that is {} lines — this types into a composer, where a newline submits. send them one at a time",
            text.lines().count()
        );
    }

    let far = Remote::resolve(target)?;
    let info = far.info()?;
    let before: HashSet<String> = human_turns(&far.transcript(&info.session_id)?)
        .into_iter()
        .map(|turn| turn.uuid)
        .collect();

    far.type_text(&text)?;
    // Look before pressing Enter. If the keystrokes went to another window, or
    // the composer was not focused, this is the last moment at which nothing has
    // been sent yet.
    let composer = far.pane(0)?;
    // ⚠ **A composer WRAPS.** Anything past the pane width comes back with a
    // newline and the continuation's indentation inserted mid-sentence, so a
    // literal `contains` fails on exactly the long messages most worth checking
    // — and the failure looks like "the keystrokes went somewhere else", which
    // is the one thing this check exists to catch. Comparing with all
    // whitespace removed is indifferent to where the wrap landed.
    let squeeze =
        |value: &str| -> String { value.chars().filter(|c| !c.is_whitespace()).collect() };
    if !squeeze(&composer).contains(&squeeze(&text)) {
        bail!(
            "typed it, but it is not on screen — not pressing Enter.\n\
             the window may not be the composer. `outpost pane` to look."
        );
    }
    far.press_enter()?;

    // ⚠ **The receipt is the CLI recording the turn, not tmux accepting the
    // keys.** Reporting a send the moment the bytes leave is the defect the
    // console keeps making: a green tick that means a pipe was written to. The
    // transcript gaining a human turn is the session having actually read it.
    let deadline = std::time::Instant::now() + PATIENCE;
    while std::time::Instant::now() < deadline {
        std::thread::sleep(INTERVAL);
        let landed = human_turns(&far.transcript(&info.session_id)?)
            .into_iter()
            .find(|turn| !before.contains(&turn.uuid) && turn.text.contains(&text));
        if let Some(turn) = landed {
            let how = if turn.queued {
                "queued — it was working, so it will read this when the turn ends"
            } else {
                "read"
            };
            println!("sent, {how} ({} chars)", text.chars().count());
            if args.contains(&"--wait") {
                return watch(
                    &far,
                    &info.session_id,
                    timeout_of(args)?,
                    flag_of(args, "--match")?,
                );
            }
            return Ok(());
        }
    }
    bail!(
        "typed and submitted, but it has not appeared in the transcript in {}s.\n\
         it may be mid-turn with a slow write, or it may have stopped reading input.\n\
         `outpost pane` to see which — do NOT send it again blind.",
        PATIENCE.as_secs()
    )
}

fn wait(target: Option<&str>, args: &[&str]) -> Result<()> {
    let far = Remote::resolve(target)?;
    let info = far.info()?;
    let limit = timeout_of(args)?;
    if args.contains(&"--idle") {
        return watch_idle(&far, limit);
    }
    if let Some(index) = args.iter().position(|a| *a == "--task") {
        // The word after --task is optional: bare --task waits for ANY task to
        // end, which is right when only one is running.
        let want = args
            .get(index + 1)
            .filter(|next| !next.starts_with("--"))
            .map(|next| (*next).to_string());
        return watch_task(&far, &info.session_id, limit, want);
    }
    watch(&far, &info.session_id, limit, flag_of(args, "--match")?)
}

/// How many consecutive idle readings count as actually idle.
///
/// ⚠ **One is not enough.** A session between turns reports `idle` for a moment
/// — reading the gap between two turns as "it has finished" is the same class of
/// error as reading a quiet log as a finished build. Two readings a poll apart
/// cost 15 seconds and remove it.
const SETTLED: usize = 2;

/// Block until the session stops working.
///
/// The CLI writes its own `status`, which is a first-party signal and better
/// than anything inferrable from outside — the usual alternative is guessing
/// from a transcript that stopped growing, which is also what a wedged session
/// looks like. ⚠ `shell` is NOT idle: it means a command is running, and a long
/// build sits there for half an hour.
fn watch_idle(far: &Remote, limit: std::time::Duration) -> Result<()> {
    let deadline = std::time::Instant::now() + limit;
    let mut settled = 0;
    let mut last = String::new();
    while std::time::Instant::now() < deadline {
        let info = far.info()?;
        if info.status == "idle" {
            settled += 1;
            if settled >= SETTLED {
                println!("idle");
                return Ok(());
            }
        } else {
            // Any non-idle reading restarts the count, so a flicker to idle in
            // the middle of a turn cannot accumulate towards a false result.
            settled = 0;
        }
        last = info.status;
        std::thread::sleep(WAIT_EVERY);
    }
    bail!("still {last:?} after {}s", limit.as_secs())
}

/// Block until a background task ends, then say how it ended.
fn watch_task(
    far: &Remote,
    id: &str,
    limit: std::time::Duration,
    want: Option<String>,
) -> Result<()> {
    let matches = |summary: &str| {
        want.as_deref()
            .is_none_or(|w| summary.to_lowercase().contains(&w.to_lowercase()))
    };
    let before: HashSet<String> = endings(&far.transcript(id)?)
        .into_iter()
        .map(|ending| ending.uuid)
        .collect();
    let deadline = std::time::Instant::now() + limit;
    while std::time::Instant::now() < deadline {
        std::thread::sleep(WAIT_EVERY);
        let ended = endings(&far.transcript(id)?)
            .into_iter()
            .find(|e| !before.contains(&e.uuid) && matches(&e.summary));
        if let Some(ended) = ended {
            println!("{}  {}  {}", clock(&ended.at), ended.status, ended.summary);
            // ⚠ **A task that was killed must not exit 0.** That is the whole
            // point of reading the status rather than the session's account of
            // it: twice today a build was killed and reported as producing no
            // output, which reads like a build that did nothing wrong.
            if !ended.ok() {
                bail!("the task did not complete — it was {}", ended.status);
            }
            return Ok(());
        }
    }
    bail!(
        "no background task ended in {}s. `outpost read` to see what it is doing.",
        limit.as_secs()
    )
}

/// Block until the session says something it has not said before, then print it.
///
/// This exists because the alternative was a shell loop that fingerprinted the
/// last line and compared strings — which is both awkward to write each time and
/// wrong in a way that is easy to miss, since an identical message sent twice is
/// a real thing a session does.
fn watch(far: &Remote, id: &str, limit: std::time::Duration, needle: Option<String>) -> Result<()> {
    let before: HashSet<String> = spoken(&far.transcript(id)?)
        .into_iter()
        .map(|turn| turn.uuid)
        .collect();
    let deadline = std::time::Instant::now() + limit;
    while std::time::Instant::now() < deadline {
        std::thread::sleep(WAIT_EVERY);
        // ⚠ **Waiting for "anything new" is the wrong thing whenever somebody
        // else is also talking to the session.** Measured: a wait for a build
        // result returned on an unrelated answer 22 minutes in, because a
        // question had been asked in the meantime and the session replied to
        // that first. `--match` is what makes the wait about the thing wanted
        // rather than about the next thing to happen.
        let fresh: Vec<Turn> = spoken(&far.transcript(id)?)
            .into_iter()
            .filter(|turn| !before.contains(&turn.uuid))
            .filter(|turn| {
                needle
                    .as_deref()
                    .is_none_or(|want| turn.text.to_lowercase().contains(&want.to_lowercase()))
            })
            .collect();
        if !fresh.is_empty() {
            for turn in fresh {
                println!("{}  {}\n", clock(&turn.at), turn.text);
            }
            return Ok(());
        }
    }
    // Not an error in the sense of something being broken — but it must not
    // exit 0, or a script cannot tell "it answered" from "it did not".
    match needle {
        Some(want) => bail!(
            "nothing matching {want:?} in {}s. `outpost read` to see what it did say.",
            limit.as_secs()
        ),
        None => bail!(
            "nothing new in {}s. `outpost pane` to see whether it is still working.",
            limit.as_secs()
        ),
    }
}
