use outpost::footer::{background_shells, turn_done};

/// A session that finished its turn with two background shells, as captured.
const DONE_TWO_SHELLS: &str = "\
✻ Churned for 46s · done 6:29 PM · 2 shells still running

──────────────────────────────────────────── development ─
❯
──────────────────────────────────────────────────────────
  ctx 518k/1M (52%)                                    /rc
  ⏵⏵ auto mode on · 2 shells · ← 1 agent
";

/// The same session mid-turn, running a foreground command.
const WORKING: &str = "\
✻ Churned for 46s · done 6:29 PM · 2 shells still running

● Bash(bazel build //foo)
  ⎿  Running…
✢ Ideating… (42s · ↓ 2.2k tokens)

──────────────────────────────────────────────────────────
❯
──────────────────────────────────────────────────────────
  ⏵⏵ auto mode on · 3 shells · ← 1 agent
";

#[test]
fn counts_the_shells_in_the_footer() {
    assert_eq!(background_shells(DONE_TWO_SHELLS), 2);
    assert_eq!(background_shells(WORKING), 3);
}

#[test]
fn one_shell_is_singular() {
    assert_eq!(background_shells("  ⏵⏵ auto mode on · 1 shell\n"), 1);
}

#[test]
fn no_shells_counts_zero() {
    assert_eq!(background_shells("  ⏵⏵ auto mode on · ← 1 agent\n"), 0);
}

#[test]
fn a_shell_count_in_the_conversation_is_not_the_footer() {
    let pane = "we have 9 shells open here\n\n❯ \n  ⏵⏵ auto mode on\n";
    assert_eq!(background_shells(pane), 0);
}

#[test]
fn a_finished_turn_is_done() {
    assert!(turn_done(DONE_TWO_SHELLS));
}

#[test]
fn a_running_turn_is_not_done_even_after_an_earlier_done_line() {
    assert!(!turn_done(WORKING));
}

#[test]
fn no_spinner_line_is_not_done() {
    assert!(!turn_done("❯ \n  ⏵⏵ auto mode on · 2 shells\n"));
}
