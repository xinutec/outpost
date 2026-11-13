//! Reading the `/login` screen of a session that has been logged out.
//!
//! The screen wraps the sign-in link across several lines at the window's
//! width, so copying it out of `outpost pane` by eye breaks it. These read the
//! screen and say which step the session is at.

/// Where the session's login screen is.
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    /// Showing the link, waiting for the code the browser gives back.
    AwaitingCode,
    /// The code was accepted and it waits for Enter.
    Done,
    /// Not on a login screen.
    Elsewhere,
}

const PROMPT: &str = "Paste code here";
const DONE: &str = "Login successful";

pub fn step(screen: &str) -> Step {
    // The screen shows only the last login attempt, but scrollback may hold an
    // earlier one, so the last marker decides.
    let prompt = screen.rfind(PROMPT);
    let done = screen.rfind(DONE);
    match (prompt, done) {
        (_, Some(d)) if prompt.is_none_or(|p| d > p) => Step::Done,
        (Some(_), _) => Step::AwaitingCode,
        _ => Step::Elsewhere,
    }
}

/// The sign-in link, rejoined into one line. `None` when no link is on screen.
///
/// ⚠ **The last link wins**: an earlier, abandoned `/login` stays in scrollback
/// with a link that no longer works.
pub fn url(screen: &str) -> Option<String> {
    let lines: Vec<&str> = screen.lines().map(str::trim).collect();
    let start = lines
        .iter()
        .rposition(|line| line.starts_with("https://"))?;
    let link: String = lines[start..]
        .iter()
        .take_while(|line| !line.is_empty())
        .copied()
        .collect();
    Some(link)
}

/// Whether the code box holds typed characters: it masks them with `*`, so the
/// text itself is never on screen, and only that something was entered can be
/// checked.
pub fn code_entered(screen: &str) -> bool {
    screen
        .lines()
        .rfind(|line| line.contains(PROMPT))
        .and_then(|line| line.split_once('>'))
        .is_some_and(|(_, typed)| typed.contains('*'))
}
