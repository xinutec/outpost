//! Reading a session's transcript: who said what, and when a task ended.
//!
//! Nothing in here talks to a machine. A transcript is a byte slice of jsonl by
//! the time it arrives, which is what makes these rules testable against rows
//! captured from a real one — and they need to be, because every failure mode
//! they guard against is silent. A parser that matches nothing and a session
//! that said nothing produce the same empty output.

use reader::transcript::human_turns;
use std::collections::HashSet;

/// One side of one exchange.
pub struct Line {
    /// ⚠ **The FULL ISO stamp, not the pretty one.** These get sorted, and
    /// sorting `HH:MM` across a transcript that spans days interleaves the days
    /// by time-of-day — which reads as a conversation in a plausible but wrong
    /// order, and puts the wrong messages at the end. Formatting happens at the
    /// point of printing instead.
    pub at: String,
    pub who: &'static str,
    pub text: String,
}

/// An assistant turn, with the uuid that tells a new one from a repeat.
pub struct Turn {
    pub uuid: String,
    pub at: String,
    pub text: String,
}

/// A background task that finished, however it finished.
pub struct Ending {
    pub uuid: String,
    pub at: String,
    /// `completed` or `killed`, as the harness wrote it. ⚠ Never assume the
    /// first: the distinction is the entire reason this type exists.
    pub status: String,
    pub summary: String,
}

impl Ending {
    /// Whether this is the ending a caller can treat as success.
    pub fn ok(&self) -> bool {
        self.status == "completed"
    }
}

/// Both sides of the conversation, in order.
///
/// The human side comes from `reader`, which owns five rules that callers kept
/// re-deriving — the one that matters most here is that a message typed while
/// the session is working is stored as a queued `attachment` and never as a
/// `user` row. Reading only `user` rows once produced a confident report that
/// three of Pippijn's messages had been lost when they had all been delivered.
pub fn conversation(bytes: &[u8]) -> Vec<Line> {
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
///
/// ⚠ **Anything that is not that shape comes back whole.** Taking the first
/// five characters after a `T` without checking them returns a five-character
/// slice of whatever was there — `no-T-here` printed as `-here`, which sits in
/// the timestamp column looking like a time. Giving the input back unchanged is
/// visibly not a clock; a mangled one is not.
pub fn clock(stamp: &str) -> String {
    stamp
        .split_once('T')
        .map(|(_, time)| time)
        .and_then(|time| time.get(..5))
        .filter(|hhmm| {
            let b = hhmm.as_bytes();
            b[2] == b':' && b[..2].iter().chain(&b[3..]).all(u8::is_ascii_digit)
        })
        .unwrap_or(stamp)
        .to_string()
}

/// Every assistant turn, with the uuid that tells a NEW one from a repeat.
///
/// ⚠ **Dedupe by uuid and keep the first.** The CLI rewrites earlier stretches
/// of the file back into it, so a linear read sees the same turn twice — and a
/// waiter that keyed on text or position would announce an old message as the
/// answer it was waiting for.
pub fn spoken(bytes: &[u8]) -> Vec<Turn> {
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
            out.push(Turn { uuid, at, text });
        }
    }
    out
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
pub fn endings(bytes: &[u8]) -> Vec<Ending> {
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
            (!status.is_empty()).then(|| Ending {
                uuid: turn.uuid,
                at: turn.at,
                status,
                summary: field("summary"),
            })
        })
        .collect()
}
