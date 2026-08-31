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

mod config;
mod remote;

use anyhow::{Context, Result, bail};
use reader::transcript::human_turns;
use remote::Remote;
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

/// How long `wait` will sit there, and how often it asks.
///
/// ⚠ **A poll costs an ssh round trip and a transcript tail**, so asking often
/// is not free — and the thing being waited for is usually a build measured in
/// tens of minutes. The default ceiling is generous because the alternative,
/// returning early, reads exactly like "it never answered".
const WAIT_FOR: std::time::Duration = std::time::Duration::from_secs(3600);
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

/// Lift `-t <name>` (or `--target <name>`) out of the arguments.
fn take_target<'a>(args: &[&'a str]) -> Result<(Option<String>, Vec<&'a str>)> {
    let mut target = None;
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match *arg {
            "-t" | "--target" => {
                // ⚠ A missing value would otherwise swallow the verb: `-t read`
                // would silently look for a target called "read".
                let name = it.next().context("-t needs a target name")?;
                if name.starts_with('-') {
                    bail!("-t needs a target name, got {name:?}");
                }
                target = Some((*name).to_string());
            }
            other => rest.push(other),
        }
    }
    Ok((target, rest))
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

/// One side of one exchange.
struct Line {
    /// ⚠ **The FULL ISO stamp, not the pretty one.** These get sorted, and
    /// sorting `HH:MM` across a transcript that spans days interleaves the days
    /// by time-of-day — which reads as a conversation in a plausible but wrong
    /// order, and puts the wrong messages at the end. Formatting happens at the
    /// point of printing instead.
    at: String,
    who: &'static str,
    text: String,
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

/// Both sides of the conversation, in order.
///
/// The human side comes from `reader`, which owns five rules that callers kept
/// re-deriving — the one that matters most here is that a message typed while
/// the session is working is stored as a queued `attachment` and never as a
/// `user` row. Reading only `user` rows once produced a confident report that
/// three of Pippijn's messages had been lost when they had all been delivered.
fn conversation(bytes: &[u8]) -> Vec<Line> {
    let mut lines: Vec<Line> = human_turns(bytes)
        .into_iter()
        // ⚠ **Not everything on the human side was typed by a human.** The
        // harness injects task notifications as user turns, and showing them
        // under the person's name misreports a machine event as an instruction
        // they gave — which is exactly the kind of thing a later reader acts on.
        .filter(|turn| !turn.text.trim_start().starts_with("<task-notification>"))
        .map(|turn| Line {
            at: turn.at,
            who: "pippijn",
            text: turn.text,
        })
        .collect();
    let mut seen = HashSet::new();
    for row in bytes.split(|b| *b == b'\n') {
        let Ok(row) = serde_json::from_slice::<serde_json::Value>(row) else {
            continue;
        };
        // Same dedupe rule as the human side: the CLI rewrites earlier stretches
        // back into the file, and the later copy is the degraded one.
        let uuid = row["uuid"].as_str().unwrap_or_default().to_string();
        if !uuid.is_empty() && !seen.insert(uuid) {
            continue;
        }
        if let Some(text) = said(&row) {
            lines.push(Line {
                at: row["timestamp"].as_str().unwrap_or_default().to_string(),
                who: "session",
                text,
            });
        }
    }
    lines.sort_by(|a, b| a.at.cmp(&b.at));
    lines
}

/// What an assistant turn said, or `None` for every other kind of row.
///
/// ⚠ **Tool calls are not speech.** A turn is usually a `tool_use` block and
/// nothing else, and rendering those as things the session said buries the
/// sentences in argument lists.
fn said(row: &serde_json::Value) -> Option<String> {
    if row["type"].as_str()? != "assistant" {
        return None;
    }
    let parts = row["message"]["content"].as_array()?;
    let text: Vec<&str> = parts
        .iter()
        .filter(|part| part["type"] == "text")
        .filter_map(|part| part["text"].as_str())
        .collect();
    let text = text.join("\n").trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// `2026-08-31T14:31:24.717Z` as `14:31`.
fn clock(stamp: &str) -> String {
    stamp
        .split('T')
        .nth(1)
        .and_then(|time| time.get(..5))
        .unwrap_or(stamp)
        .to_string()
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
    let squeeze = |value: &str| -> String {
        value.chars().filter(|c| !c.is_whitespace()).collect()
    };
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

/// Every assistant turn, with the uuid that tells a NEW one from a repeat.
///
/// ⚠ **Dedupe by uuid and keep the first.** The CLI rewrites earlier stretches
/// of the file back into it, so a linear read sees the same turn twice — and a
/// waiter that keyed on text or position would announce an old message as the
/// answer it was waiting for.
fn spoken(bytes: &[u8]) -> Vec<(String, String, String)> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for row in bytes.split(|b| *b == b'\n') {
        let Ok(row) = serde_json::from_slice::<serde_json::Value>(row) else {
            continue;
        };
        let uuid = row["uuid"].as_str().unwrap_or_default().to_string();
        if uuid.is_empty() || !seen.insert(uuid.clone()) {
            continue;
        }
        if let Some(text) = said(&row) {
            let at = row["timestamp"].as_str().unwrap_or_default().to_string();
            out.push((uuid, at, text));
        }
    }
    out
}

fn wait(target: Option<&str>, args: &[&str]) -> Result<()> {
    let far = Remote::resolve(target)?;
    let info = far.info()?;
    let limit = timeout_of(args)?;
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
        .map(|(uuid, _, _, _)| uuid)
        .collect();
    let deadline = std::time::Instant::now() + limit;
    while std::time::Instant::now() < deadline {
        std::thread::sleep(WAIT_EVERY);
        let ended = endings(&far.transcript(id)?)
            .into_iter()
            .find(|(uuid, _, _, summary)| !before.contains(uuid) && matches(summary));
        if let Some((_, at, status, summary)) = ended {
            println!("{}  {status}  {summary}", clock(&at));
            // ⚠ **A task that was killed must not exit 0.** That is the whole
            // point of reading the status rather than the session's account of
            // it: twice today a build was killed and reported as producing no
            // output, which reads like a build that did nothing wrong.
            if status != "completed" {
                bail!("the task did not complete — it was {status}");
            }
            return Ok(());
        }
    }
    bail!(
        "no background task ended in {}s. `outpost read` to see what it is doing.",
        limit.as_secs()
    )
}

/// The value of a `--flag value` pair, if it is there.
fn flag_of(args: &[&str], flag: &str) -> Result<Option<String>> {
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if *arg == flag {
            let value = it.next().with_context(|| format!("{flag} needs a value"))?;
            return Ok(Some((*value).to_string()));
        }
    }
    Ok(None)
}

/// How long to wait, from `--timeout <seconds>`.
fn timeout_of(args: &[&str]) -> Result<std::time::Duration> {
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if *arg == "--timeout" {
            let value = it.next().context("--timeout needs a number of seconds")?;
            let seconds: u64 = value.parse().context("--timeout wants seconds")?;
            return Ok(std::time::Duration::from_secs(seconds));
        }
    }
    Ok(WAIT_FOR)
}

/// Block until the session says something it has not said before, then print it.
///
/// This exists because the alternative was a shell loop that fingerprinted the
/// last line and compared strings — which is both awkward to write each time and
/// wrong in a way that is easy to miss, since an identical message sent twice is
/// a real thing a session does.
fn watch(
    far: &Remote,
    id: &str,
    limit: std::time::Duration,
    needle: Option<String>,
) -> Result<()> {
    let before: HashSet<String> = spoken(&far.transcript(id)?)
        .into_iter()
        .map(|(uuid, _, _)| uuid)
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
        let fresh: Vec<(String, String, String)> = spoken(&far.transcript(id)?)
            .into_iter()
            .filter(|(uuid, _, _)| !before.contains(uuid))
            .filter(|(_, _, text)| {
                needle
                    .as_deref()
                    .is_none_or(|want| text.to_lowercase().contains(&want.to_lowercase()))
            })
            .collect();
        if !fresh.is_empty() {
            for (_, at, text) in fresh {
                println!("{}  {text}\n", clock(&at));
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

/// Background-task endings recorded in the transcript, newest last.
///
/// ⚠ **This is the only EXACT signal that a long job finished.** Waiting on what
/// the session says is waiting on prose: it reports in whatever words it
/// chooses, it may answer an unrelated question first, and — measured twice —
/// when its own task is killed it can report "no output" without knowing why.
/// The harness writes these rows itself, with a status that distinguishes
/// `completed` from `killed`, which is precisely the distinction that cost two
/// builds today.
fn endings(bytes: &[u8]) -> Vec<(String, String, String, String)> {
    human_turns(bytes)
        .into_iter()
        .filter(|turn| turn.text.contains("<task-notification>"))
        .filter_map(|turn| {
            let field = |tag: &str| -> String {
                let open = format!("<{tag}>");
                let close = format!("</{tag}>");
                turn.text
                    .split_once(&open)
                    .and_then(|(_, rest)| rest.split_once(&close))
                    .map(|(value, _)| value.trim().to_string())
                    .unwrap_or_default()
            };
            let status = field("status");
            // A notification with no status is a shape this does not understand;
            // treating it as an ending would be inventing one.
            (!status.is_empty()).then(|| (turn.uuid, turn.at, status, field("summary")))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A user row as the CLI writes one, wrapping whatever text is given.
    fn row(uuid: &str, text: &str) -> String {
        serde_json::json!({
            "type": "user",
            "uuid": uuid,
            "timestamp": "2026-08-31T14:31:17.038Z",
            "message": { "role": "user", "content": text },
        })
        .to_string()
    }

    /// ⚠ **Verbatim from a real transcript**, not invented. A probe against the
    /// live session could only ever show "nothing matched", which is the same
    /// output a parser that never matches anything would give — so the shape
    /// this reads has to be pinned by something that fails loudly when it moves.
    const KILLED: &str = "<task-notification>\n\
        <task-id>b2jj7il5o</task-id>\n\
        <tool-use-id>toolu_014JBQeyBit3vaXrYRKhseUS</tool-use-id>\n\
        <output-file>/tmp/claude-1000/tasks/b2jj7il5o.output</output-file>\n\
        <status>killed</status>\n\
        <summary>Background command \"Build all targets after reboot\" was stopped</summary>\n\
        </task-notification>";

    const DONE: &str = "<task-notification>\n\
        <task-id>b3jbxr5cm</task-id>\n\
        <status>completed</status>\n\
        <summary>Background command \"Retry building all targets\" completed (exit code 0)</summary>\n\
        </task-notification>";

    #[test]
    fn reads_status_and_summary_out_of_a_notification() {
        let bytes = format!("{}\n{}\n", row("a", KILLED), row("b", DONE));
        let found = endings(bytes.as_bytes());
        assert_eq!(found.len(), 2, "both notifications should be seen");
        assert_eq!(found[0].2, "killed");
        assert!(found[0].3.contains("Build all targets after reboot"));
        assert_eq!(found[1].2, "completed");
    }

    /// The distinction the whole verb exists for: a killed task and a completed
    /// one are both "the task ended", and only one of them is success.
    #[test]
    fn killed_is_not_completed() {
        let bytes = format!("{}\n", row("a", KILLED));
        assert_ne!(endings(bytes.as_bytes())[0].2, "completed");
    }

    /// Ordinary conversation must not look like a task ending.
    #[test]
    fn plain_text_is_not_an_ending() {
        let bytes = format!("{}\n", row("a", "we restarted. build //... again"));
        assert!(endings(bytes.as_bytes()).is_empty());
    }

    /// A notification whose shape this does not understand is skipped rather
    /// than reported as an ending with an empty status.
    #[test]
    fn a_notification_without_a_status_is_not_an_ending() {
        let text = "<task-notification>\n<task-id>x</task-id>\n</task-notification>";
        let bytes = format!("{}\n", row("a", text));
        assert!(endings(bytes.as_bytes()).is_empty());
    }
}
