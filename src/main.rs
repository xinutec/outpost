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

const USAGE: &str = "usage:
  outpost status            what the session says it is doing, and the windows
  outpost read [n]          the last n exchanges, both sides   (default 12)
  outpost pane [n]          the window itself, with n lines of scrollback
  outpost send <text>       type it and press Enter, as Pippijn; `-` reads stdin

  --full   with `read`, do not shorten long messages";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest: Vec<&str> = args.iter().map(String::as_str).collect();
    match rest.split_first() {
        None | Some((&"status", [])) => status(),
        Some((&"read", tail)) => read(tail),
        Some((&"pane", tail)) => pane(tail),
        Some((&"send", tail)) => send(tail),
        Some((&("-h" | "--help" | "help"), _)) => {
            println!("{USAGE}");
            Ok(())
        }
        Some((other, _)) => bail!("no such command {other:?}\n\n{USAGE}"),
    }
}

fn status() -> Result<()> {
    let far = Remote::from_env()?;
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

fn pane(args: &[&str]) -> Result<()> {
    let back: usize = match args.first() {
        Some(n) => n.parse().context("that is not a number of lines")?,
        None => 0,
    };
    print!("{}", Remote::from_env()?.pane(back)?);
    Ok(())
}

/// One side of one exchange.
struct Line {
    at: String,
    who: &'static str,
    text: String,
}

fn read(args: &[&str]) -> Result<()> {
    let full = args.contains(&"--full");
    let want: usize = args
        .iter()
        .find(|arg| !arg.starts_with("--"))
        .map_or(Ok(12), |n| n.parse())
        .context("that is not a number of messages")?;

    let far = Remote::from_env()?;
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
        println!("{}  {}  {text}\n", line.at, line.who);
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
        .map(|turn| Line {
            at: clock(&turn.at),
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
                at: clock(row["timestamp"].as_str().unwrap_or_default()),
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

fn send(args: &[&str]) -> Result<()> {
    if args.is_empty() {
        bail!("usage: outpost send <text>");
    }
    let text = if args == ["-"] {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        buf
    } else {
        args.join(" ")
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

    let far = Remote::from_env()?;
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
    if !composer.contains(&text) {
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
