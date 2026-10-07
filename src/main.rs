//! Talk to a Claude Code session running on another machine.
//!
//!     outpost status
//!     outpost read 20
//!     outpost pane
//!     outpost send "yes, retry"
//!     echo "..." | outpost send -
//!
//! The host comes from the config file or `OUTPOST_HOST`; nothing about any
//! particular machine is written down here. The window is not configured — a
//! session records its own tmux pane, so with one session running `outpost`
//! finds it, and with more than one you name the window (`-t security`).
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

use anyhow::{Result, bail};
use clap::Parser;
use outpost::args::{Cli, Until, Verb};
use outpost::remote::{Info, Remote, choose};
use outpost::transcript::{Turn, clock, conversation, day, endings, spoken};
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

/// How often `wait` asks. The ceiling it stops at is `args::WAIT_FOR_SECS`.
///
/// ⚠ **A poll costs an ssh round trip and a transcript tail**, so asking often
/// is not free — and the thing being waited for is usually a build measured in
/// tens of minutes.
const WAIT_EVERY: std::time::Duration = std::time::Duration::from_secs(15);

#[expect(
    unsafe_code,
    reason = "libc::signal before any thread exists, to restore SIGPIPE's default"
)]
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

    let cli = Cli::parse();
    let target = cli.target.as_deref();
    match cli.verb.unwrap_or(Verb::Status) {
        Verb::Status => status(target),
        Verb::Read { n, full } => read(target, n, full),
        Verb::Pane { n } => pane(target, n),
        Verb::Send { text, wait, until } => send(target, &text, wait.then_some(until)),
        Verb::Wait { idle, task, until } => self::wait(target, idle, task, until),
    }
}

fn status(target: Option<&str>) -> Result<()> {
    let far = Remote::resolve(target)?;
    let sessions = far.sessions()?;
    // ⚠ **With more than one session, status is where you SEE them** — so with
    // no name it lists every running session rather than refusing to choose, and
    // with a name it shows just that one. This is the surface `choose`'s error
    // tells you to come to.
    let shown: Vec<&Info> = match target {
        Some(_) => vec![choose(&sessions, target)?],
        None if sessions.is_empty() => {
            bail!("no running session on the far side — is Claude running?")
        }
        None => sessions.iter().collect(),
    };
    for info in shown {
        let bridged = match &info.bridge {
            Some(_) => "bridged",
            None => "not bridged",
        };
        println!(
            "{} — {} — claude {} — pid {} — {bridged}",
            info.label(),
            info.status,
            info.version,
            info.pid,
        );
        println!("  {}", info.session_id);
    }
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

fn pane(target: Option<&str>, back: usize) -> Result<()> {
    let far = Remote::resolve(target)?;
    // A config target names a plain window (a bot's log, the build shell); those
    // have no session, so read the window directly. Anything else is a session's
    // window name, so find the session and read its own pane.
    if let Some(window) = outpost::config::window_target(target)? {
        print!("{}", far.pane(&window, back)?);
        return Ok(());
    }
    let sessions = far.sessions()?;
    let info = choose(&sessions, target)?;
    print!("{}", far.pane(info.target()?, back)?);
    Ok(())
}

fn read(target: Option<&str>, want: usize, full: bool) -> Result<()> {
    let far = Remote::resolve(target)?;
    let sessions = far.sessions()?;
    let info = choose(&sessions, target)?;
    let bytes = far.transcript(&info.session_id)?;
    let lines = conversation(&bytes);

    let shown = want.min(lines.len());
    // ⚠ **`14:31` alone does not say which day.** Inside one sitting that is
    // fine; across a gap it is not, and the failure is silent — a conversation
    // last touched a fortnight ago reads exactly like one from this morning.
    // The header goes in wherever the day changes, and always before the first
    // line shown, because a reader starting mid-transcript has nothing earlier
    // to have inferred it from.
    let mut dated: Option<&str> = None;
    for line in &lines[lines.len() - shown..] {
        if let Some(today) = day(&line.at)
            && dated != Some(today)
        {
            println!("{today}");
            dated = Some(today);
        }
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

/// `then` is where to wait for the reply, when asked to.
fn send(target: Option<&str>, words: &[String], then: Option<Until>) -> Result<()> {
    let text = if words == ["-"] {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        buf
    } else {
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
    let sessions = far.sessions()?;
    let info = choose(&sessions, target)?;
    let pane = info.target()?;
    let before: HashSet<String> = human_turns(&far.transcript(&info.session_id)?)
        .into_iter()
        .map(|turn| turn.uuid)
        .collect();

    far.type_text(pane, &text)?;
    // Look before pressing Enter. If the keystrokes went to another window, or
    // the composer was not focused, this is the last moment at which nothing has
    // been sent yet.
    let composer = far.pane(pane, 0)?;
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
    far.press_enter(pane)?;

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
            if let Some(until) = then {
                return watch(&far, &info.session_id, until.limit(), until.needle);
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

fn wait(
    target: Option<&str>,
    idle: bool,
    task: Option<Option<String>>,
    until: Until,
) -> Result<()> {
    let far = Remote::resolve(target)?;
    let sessions = far.sessions()?;
    let info = choose(&sessions, target)?;
    let limit = until.limit();
    if idle {
        return watch_idle(&far, info.pid, limit);
    }
    // Bare `--task` waits for ANY task to end, which is right when only one is
    // running.
    if let Some(want) = task {
        return watch_task(&far, &info.session_id, limit, want);
    }
    watch(&far, &info.session_id, limit, until.needle)
}

/// How many consecutive idle readings count as actually idle.
///
/// ⚠ **One is not enough.** A session between turns reports `idle` for a moment
/// — reading the gap between two turns as "it has finished" is the same class of
/// error as reading a quiet log as a finished build. Two readings a poll apart
/// cost 15 seconds and remove it.
const SETTLED: usize = 2;

/// Block until the session stops working — whether it finished or is asking.
///
/// The CLI writes its own `status`, which is a first-party signal and better
/// than anything inferrable from outside — the usual alternative is guessing
/// from a transcript that stopped growing, which is also what a wedged session
/// looks like. ⚠ `shell` is NOT idle: it means a command is running, and a long
/// build sits there for half an hour.
///
/// ⚠ **`waiting` ends the wait too, and the printed line says which stop it
/// was.** The session reports `waiting` while blocked on a question or menu;
/// treating only `idle` as done left this sitting silent through a question
/// until the timeout, which is the one moment an answer was most needed.
/// "It finished" and "it is asking you something" call for different next
/// moves, so they are not collapsed into one word.
fn watch_idle(far: &Remote, pid: u64, limit: std::time::Duration) -> Result<()> {
    let deadline = std::time::Instant::now() + limit;
    let mut settled = 0;
    let mut last = String::new();
    while std::time::Instant::now() < deadline {
        // By pid, so this watches THE session chosen, not whichever is newest —
        // with two Claudes up, the newest flips between them. If the file
        // vanishes, `status_of` errors, which is the right answer: the session
        // this was waiting on is gone.
        let info = far.status_of(pid)?;
        if info.stopped() {
            settled += 1;
            if settled >= SETTLED {
                if info.waiting() {
                    println!("waiting — it is asking something; `outpost pane` to see");
                } else {
                    println!("idle");
                }
                return Ok(());
            }
        } else {
            // Any working reading restarts the count, so a flicker to a stop in
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
