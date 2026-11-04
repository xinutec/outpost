//! What a session's window says about background work, which its state file
//! does not.
//!
//! The state file reports `shell` both while a command runs in the foreground
//! and after a turn has ended with shells left in the background. The window
//! tells them apart: the spinner line says whether the turn is `done`, and the
//! footer counts the background shells.

/// How many background shells the footer counts (`· 2 shells`), or 0.
///
/// ⚠ **Only the footer is read**, the lines below the last `❯` prompt: a count
/// in the conversation above it is about something else.
pub fn background_shells(pane: &str) -> usize {
    let lines: Vec<&str> = pane.lines().collect();
    let below = lines
        .iter()
        .rposition(|line| line.trim_start().starts_with('❯'))
        .map_or(lines.len().saturating_sub(2), |at| at + 1);
    lines[below..]
        .iter()
        .find_map(|line| count_before(line, " shell"))
        .unwrap_or(0)
}

/// Whether the latest spinner line is a finished one (`… for 46s · done …`).
///
/// ⚠ **An earlier `done` line stays in scrollback** while the next turn runs,
/// so only the lowest spinner line counts.
pub fn turn_done(pane: &str) -> bool {
    const SPINNER: [char; 7] = ['✻', '✢', '✳', '✶', '✽', '·', '*'];
    pane.lines()
        .rev()
        .map(str::trim_start)
        .find(|line| line.starts_with(SPINNER) && (line.contains('…') || line.contains(" for ")))
        .is_some_and(|line| line.contains("· done"))
}

/// The number just before `word` in `line`, if there is one.
fn count_before(line: &str, word: &str) -> Option<usize> {
    line.match_indices(word).find_map(|(at, _)| {
        let digits: String = line[..at]
            .chars()
            .rev()
            .take_while(char::is_ascii_digit)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        digits.parse().ok()
    })
}
