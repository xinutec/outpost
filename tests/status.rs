//! How `Info` classifies the session's own `status` string.
//!
//! ⚠ **The strings are the CLI's, not ours**, so they are pinned here from what
//! it actually writes. A `waiting` that silently fell through as "still working"
//! is the bug these guard: `wait --idle` sat mute through a question until it
//! timed out, which is exactly when an answer was wanted.

use outpost::remote::Info;

fn info(status: &str) -> Info {
    serde_json::from_value(serde_json::json!({
        "sessionId": "e2ae43a2-54d9-4fa2-a0cd-42d0fcd32086",
        "status": status,
    }))
    .expect("Info should deserialise from a status")
}

#[test]
fn idle_and_waiting_are_both_stops() {
    assert!(info("idle").stopped());
    assert!(info("waiting").stopped());
}

#[test]
fn busy_and_shell_are_working_not_stops() {
    assert!(!info("busy").stopped(), "the model is thinking");
    assert!(!info("shell").stopped(), "a command is running");
}

/// The distinction the caller acts on: a finished turn is read, a question is
/// answered. Only `waiting` is the second.
#[test]
fn only_waiting_is_a_question() {
    assert!(info("waiting").waiting());
    assert!(!info("idle").waiting());
    assert!(!info("busy").waiting());
}

/// An unknown status is treated as working, not as a stop: a state this does
/// not understand must not read as "go ahead, it is done".
#[test]
fn an_unknown_status_is_not_a_stop() {
    assert!(!info("reticulating").stopped());
    assert!(!info("").stopped());
}
