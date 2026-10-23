//! Whether what `send` typed reached the composer, judged from the screen.

use std::collections::HashSet;

/// True when `text`, typed between the `before` and `after` captures, is on
/// screen: as itself, or as a paste placeholder that wasn't there before.
///
/// ⚠ **A composer WRAPS**, so the text is compared with all whitespace removed.
/// ⚠ **A long message shows only as `[Pasted text #N]`.** A placeholder counts
/// only if its number is new: an old one in scrollback proves nothing about
/// where these keystrokes went.
pub fn shows_typed(before: &str, after: &str, text: &str) -> bool {
    let squeeze =
        |value: &str| -> String { value.chars().filter(|c| !c.is_whitespace()).collect() };
    if squeeze(after).contains(&squeeze(text)) {
        return true;
    }
    let old = placeholders(before);
    placeholders(after).iter().any(|n| !old.contains(n))
}

fn placeholders(screen: &str) -> HashSet<u64> {
    const OPEN: &str = "[Pasted text #";
    screen
        .match_indices(OPEN)
        .filter_map(|(at, _)| {
            let rest = &screen[at + OPEN.len()..];
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            let after = &rest[digits.len()..];
            let closed = after.starts_with(']') || after.starts_with(" +");
            if closed { digits.parse().ok() } else { None }
        })
        .collect()
}
