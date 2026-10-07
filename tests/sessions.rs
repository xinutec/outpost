//! Choosing between running sessions, and locating a session's tmux pane.
//!
//! The selection rule is the whole point: one running session is unambiguous,
//! more than one is not, and an earlier outpost that picked "the newest state
//! file" flipped between two live Claudes mid-task. These pin that it refuses
//! to guess.

use outpost::remote::{Info, choose};

/// An `Info` as it is after `sessions()` has filled in the window name.
fn session(window: &str, status: &str, tmux: &str) -> Info {
    let mut info: Info = serde_json::from_value(serde_json::json!({
        "sessionId": format!("id-of-{window}"),
        "status": status,
        "tmux": tmux,
    }))
    .expect("Info should deserialise");
    info.window = (!window.is_empty()).then(|| window.to_string());
    info
}

#[test]
fn one_running_session_needs_no_name() {
    let sessions = [session("security", "idle", "tox:@2.%2")];
    assert_eq!(
        choose(&sessions, None).unwrap().window.as_deref(),
        Some("security")
    );
}

#[test]
fn no_session_is_an_error_not_a_panic() {
    let sessions: [Info; 0] = [];
    assert!(choose(&sessions, None).is_err());
}

/// ⚠ **The bug this prevents.** Two live sessions and no name must refuse and
/// list them, not silently pick one.
#[test]
fn more_than_one_without_a_name_refuses_and_lists_them() {
    let sessions = [
        session("security", "idle", "tox:@2.%2"),
        session("development", "busy", "tox:@3.%3"),
    ];
    let err = choose(&sessions, None).unwrap_err().to_string();
    assert!(err.contains("security"), "lists the first: {err}");
    assert!(err.contains("development"), "lists the second: {err}");
}

#[test]
fn a_name_picks_the_session_in_that_window() {
    let sessions = [
        session("security", "idle", "tox:@2.%2"),
        session("development", "busy", "tox:@3.%3"),
    ];
    assert_eq!(
        choose(&sessions, Some("development")).unwrap().status,
        "busy"
    );
}

#[test]
fn a_name_that_matches_nothing_is_an_error_that_lists_what_there_is() {
    let sessions = [session("security", "idle", "tox:@2.%2")];
    let err = choose(&sessions, Some("development"))
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("development"),
        "names what was asked for: {err}"
    );
    assert!(err.contains("security"), "lists what is available: {err}");
}

#[test]
fn the_pane_id_is_what_addresses_the_session() {
    // A pane id is unique across the tmux server, so it survives the window
    // being renamed — which is exactly what we do to these windows.
    let info = session("security", "idle", "tox:@2.%2");
    assert_eq!(info.window_id(), Some("@2"));
    assert_eq!(info.pane(), Some("%2"));
    assert_eq!(info.target().unwrap(), "%2");
}

#[test]
fn a_session_not_under_tmux_has_no_target_rather_than_a_wrong_one() {
    let info = session("security", "idle", "");
    assert!(info.pane().is_none());
    assert!(info.target().is_err());
}
