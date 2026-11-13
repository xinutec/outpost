//! When `send` may press Enter: only once what it typed is on screen.
//!
//! ⚠ **A long message never shows as text.** Claude Code collapses a large paste
//! into `[Pasted text #N]`, so the literal check refused every long message and
//! the caller had to submit it with a second, made-up message.

use outpost::composer::shows_typed;

const MSG: &str = "a question about the settings plan, long enough to be collapsed";

#[test]
fn the_text_itself_on_screen_counts() {
    let after = format!("─────\n❯ {MSG}\n─────");
    assert!(shows_typed("❯ \n", &after, MSG));
}

#[test]
fn a_wrapped_composer_still_counts() {
    let after = "❯ a question about the settings plan, long\n  enough to be collapsed";
    assert!(shows_typed("❯ \n", after, MSG));
}

#[test]
fn a_placeholder_that_appeared_while_typing_counts() {
    let before = "❯ [Pasted text #16]\n";
    let after = "❯ [Pasted text #17]\n";
    assert!(shows_typed(before, after, MSG));
}

#[test]
fn a_placeholder_with_a_line_count_counts() {
    let after = "❯ [Pasted text #3 +2 lines]\n";
    assert!(shows_typed("❯ \n", after, MSG));
}

#[test]
fn an_old_placeholder_still_in_scrollback_does_not() {
    // The keystrokes went somewhere else; the placeholder is from an earlier send.
    let screen = "❯ [Pasted text #17]\n";
    assert!(!shows_typed(screen, screen, MSG));
}

#[test]
fn nothing_visible_does_not() {
    assert!(!shows_typed("❯ \n", "❯ \n", MSG));
}

#[test]
fn a_placeholder_in_a_different_window_shape_does_not() {
    // Text mentioning the words, not Claude Code's placeholder.
    let after = "Pasted text #18 is mentioned here";
    assert!(!shows_typed("❯ \n", after, MSG));
}

// ⚠ **The composer redraws on its own schedule.** One capture right after typing
// can predate the redraw of a long message, and `send` would refuse to press
// Enter with the text sitting in the box.

use outpost::composer::appears;
use std::time::Duration;

#[test]
fn text_that_shows_on_a_later_look_counts() {
    let mut looks = vec!["❯ \n".to_string(), "❯ \n".to_string(), format!("❯ {MSG}\n")].into_iter();
    let shown = appears(
        || Ok(looks.next().unwrap_or_default()),
        "❯ \n",
        MSG,
        5,
        Duration::ZERO,
    );
    assert!(shown.unwrap());
}

#[test]
fn text_that_never_shows_does_not() {
    let shown = appears(|| Ok("❯ \n".to_string()), "❯ \n", MSG, 5, Duration::ZERO);
    assert!(!shown.unwrap());
}

use outpost::composer::appears_with;

#[test]
fn a_check_that_holds_on_a_later_look_counts() {
    let mut looks = 0;
    let held = appears_with(
        || {
            looks += 1;
            Ok(looks == 3)
        },
        5,
        Duration::ZERO,
    );
    assert!(held.unwrap());
}

#[test]
fn a_check_that_never_holds_does_not() {
    assert!(!appears_with(|| Ok(false), 5, Duration::ZERO).unwrap());
}
