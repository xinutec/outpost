//! Reading a logged-out session's `/login` screen.

use outpost::login::{Step, code_entered, step, url};

/// The screen as tmux shows it: the link wrapped at the window's width.
const WAITING: &str = "\
● Please run /login · API Error: 403

❯ /login

────────────────────────────────────────────────────────
  Login

  Browser didn't open? Use the url below to  (c to
  sign in                                    copy)

https://example.test/oauth/authorize?code=true&client_
id=0000&response_type=code&scope=a+b&state=abc-def_gh
ij


  Paste code here if prompted >

  Esc to cancel
";

#[test]
fn a_wrapped_link_comes_back_as_one_line() {
    assert_eq!(
        url(WAITING).as_deref(),
        Some(
            "https://example.test/oauth/authorize?code=true&client_id=0000&response_type=code&scope=a+b&state=abc-def_ghij"
        )
    );
}

#[test]
fn the_last_link_wins() {
    let screen = format!("https://old.test/a\nb\n\n{WAITING}");
    assert!(url(&screen).unwrap().starts_with("https://example.test/"));
}

#[test]
fn no_link_is_none() {
    assert_eq!(url("❯ \n"), None);
}

#[test]
fn the_prompt_means_awaiting_the_code() {
    assert_eq!(step(WAITING), Step::AwaitingCode);
}

#[test]
fn success_means_done() {
    let screen = format!("{WAITING}\n  Login successful. Press Enter to continue…\n");
    assert_eq!(step(&screen), Step::Done);
}

/// An old success in scrollback must not hide a new login prompt.
#[test]
fn an_old_success_does_not_hide_a_new_prompt() {
    let screen = format!("Login successful. Press Enter\n\n{WAITING}");
    assert_eq!(step(&screen), Step::AwaitingCode);
}

#[test]
fn an_ordinary_screen_is_elsewhere() {
    assert_eq!(step("❯ hello\n"), Step::Elsewhere);
}

#[test]
fn masked_input_counts_as_entered() {
    assert!(code_entered("  Paste code here if prompted >*****…\n"));
}

#[test]
fn an_empty_box_is_not_entered() {
    assert!(!code_entered(WAITING));
}
