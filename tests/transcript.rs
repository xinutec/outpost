//! What a transcript means, pinned against rows taken out of a real one.
//!
//! ⚠ **These fixtures are verbatim, not invented.** A probe against a live
//! session could only ever show "nothing matched", which is the same output a
//! parser that never matches anything gives — so the shape being read has to be
//! held down by something that fails loudly when it moves.

use outpost::transcript::{clock, conversation, endings, spoken};

/// A user row as the CLI writes one, wrapping whatever text is given.
fn user(uuid: &str, at: &str, text: &str) -> String {
    serde_json::json!({
        "type": "user",
        "uuid": uuid,
        "timestamp": at,
        "message": { "role": "user", "content": text },
    })
    .to_string()
}

/// An assistant row, with the content-block array the real ones carry.
fn assistant(uuid: &str, at: &str, blocks: serde_json::Value) -> String {
    serde_json::json!({
        "type": "assistant",
        "uuid": uuid,
        "timestamp": at,
        "message": { "role": "assistant", "content": blocks },
    })
    .to_string()
}

fn text_block(text: &str) -> serde_json::Value {
    serde_json::json!([{ "type": "text", "text": text }])
}

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
    let bytes = format!(
        "{}\n{}\n",
        user("a", "2026-08-31T14:31:17.038Z", KILLED),
        user("b", "2026-08-31T14:35:02.100Z", DONE)
    );
    let found = endings(bytes.as_bytes());
    assert_eq!(found.len(), 2, "both notifications should be seen");
    assert_eq!(found[0].status, "killed");
    assert!(found[0].summary.contains("Build all targets after reboot"));
    assert_eq!(found[1].status, "completed");
}

/// The distinction the whole `wait --task` verb exists for: a killed task and a
/// completed one are both "the task ended", and only one of them is success.
#[test]
fn killed_is_not_completed() {
    let bytes = format!("{}\n", user("a", "2026-08-31T14:31:17.038Z", KILLED));
    let found = endings(bytes.as_bytes());
    assert_eq!(found[0].status, "killed");
    assert!(!found[0].ok(), "a killed task must not read as success");
}

#[test]
fn completed_is_the_one_ending_that_is_ok() {
    let bytes = format!("{}\n", user("a", "2026-08-31T14:35:02.100Z", DONE));
    assert!(endings(bytes.as_bytes())[0].ok());
}

/// Ordinary conversation must not look like a task ending.
#[test]
fn plain_text_is_not_an_ending() {
    let bytes = format!(
        "{}\n",
        user(
            "a",
            "2026-08-31T14:31:17.038Z",
            "we restarted. build //... again"
        )
    );
    assert!(endings(bytes.as_bytes()).is_empty());
}

/// A notification whose shape this does not understand is skipped rather than
/// reported as an ending with an empty status.
#[test]
fn a_notification_without_a_status_is_not_an_ending() {
    let text = "<task-notification>\n<task-id>x</task-id>\n</task-notification>";
    let bytes = format!("{}\n", user("a", "2026-08-31T14:31:17.038Z", text));
    assert!(endings(bytes.as_bytes()).is_empty());
}

/// ⚠ **The harness writes task notifications as user turns.** Showing them
/// under the person's name reports a machine event as an instruction they gave,
/// which is exactly the kind of thing a later reader acts on.
#[test]
fn a_task_notification_is_not_something_the_person_said() {
    let bytes = format!(
        "{}\n{}\n",
        user("a", "2026-08-31T14:31:17.038Z", KILLED),
        user("b", "2026-08-31T14:32:00.000Z", "yes, retry")
    );
    let lines = conversation(bytes.as_bytes());
    assert_eq!(lines.len(), 1, "only the typed message is conversation");
    assert_eq!(lines[0].text, "yes, retry");
}

/// ⚠ **Sorting on `HH:MM` interleaves days**, which reads as a plausible but
/// wrong order and puts the wrong message last. Two turns a day apart, given in
/// reverse, have to come back oldest first.
#[test]
fn a_conversation_spanning_days_is_ordered_by_date_not_time_of_day() {
    let bytes = format!(
        "{}\n{}\n",
        user("late-yesterday", "2026-08-30T23:17:00.000Z", "first"),
        user("this-morning", "2026-08-31T09:04:00.000Z", "second")
    );
    let lines = conversation(bytes.as_bytes());
    let said: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(said, ["first", "second"]);
}

/// Both sides, with the assistant's turns read out of the content blocks.
#[test]
fn a_conversation_carries_both_sides() {
    let bytes = format!(
        "{}\n{}\n",
        user("q", "2026-08-31T14:31:17.038Z", "is it still building?"),
        assistant(
            "a",
            "2026-08-31T14:31:24.717Z",
            text_block("yes, 40 minutes in")
        )
    );
    let lines = conversation(bytes.as_bytes());
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].who, "pippijn");
    assert_eq!(lines[1].who, "session");
    assert_eq!(lines[1].text, "yes, 40 minutes in");
}

/// ⚠ **Tool calls are not speech.** A turn is usually a `tool_use` block and
/// nothing else; rendering those as things the session said buries the
/// sentences in argument lists.
#[test]
fn a_tool_call_is_not_something_the_session_said() {
    let blocks = serde_json::json!([
        { "type": "tool_use", "id": "toolu_1", "name": "Bash", "input": { "command": "bazel build //..." } }
    ]);
    let bytes = format!("{}\n", assistant("a", "2026-08-31T14:31:24.717Z", blocks));
    assert!(spoken(bytes.as_bytes()).is_empty());
    assert!(conversation(bytes.as_bytes()).is_empty());
}

/// ⚠ **The CLI rewrites earlier stretches of the file back into it**, so a
/// linear read sees the same turn twice. A waiter that counted the repeat as new
/// would announce an old message as the answer it was waiting for.
#[test]
fn a_turn_written_twice_is_one_turn() {
    let row = assistant("same-uuid", "2026-08-31T14:31:24.717Z", text_block("done"));
    let bytes = format!("{row}\n{row}\n");
    assert_eq!(spoken(bytes.as_bytes()).len(), 1);
}

/// A truncated tail starts mid-line, so the first row is not valid JSON. It is
/// skipped; it does not take the rest of the read with it.
#[test]
fn a_half_line_from_a_truncated_tail_is_skipped() {
    let bytes = format!(
        "id\":\"cut-off\",\"timestamp\"}}\n{}\n",
        assistant("whole", "2026-08-31T14:31:24.717Z", text_block("intact"))
    );
    let turns = spoken(bytes.as_bytes());
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].text, "intact");
}

#[test]
fn a_stamp_prints_as_hours_and_minutes() {
    assert_eq!(clock("2026-08-31T14:31:24.717Z"), "14:31");
}

/// A stamp this does not understand comes back whole rather than as a slice of
/// itself: a mangled clock reads as a real time.
#[test]
fn an_unparseable_stamp_is_returned_untouched() {
    assert_eq!(clock("no-T-here"), "no-T-here");
}

/// ⚠ **Five characters after a `T` is not a time.** Taking them unchecked
/// returned `-here` for `no-T-here`, which sits in the timestamp column looking
/// like one.
#[test]
fn five_characters_after_a_t_are_not_a_clock_unless_they_are_one() {
    assert_eq!(clock("2026-08-31Tnot-a-time"), "2026-08-31Tnot-a-time");
    assert_eq!(clock(""), "");
}
