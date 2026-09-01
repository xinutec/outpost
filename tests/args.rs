//! Command-line parsing, where every bug has been a parse that quietly did
//! something plausible.

use outpost::args::{WAIT_FOR, flag_of, take_target, timeout_of};

#[test]
fn a_target_is_lifted_out_and_the_verb_survives() {
    let (target, rest) = take_target(&["-t", "build", "wait", "--idle"]).unwrap();
    assert_eq!(target.as_deref(), Some("build"));
    assert_eq!(rest, ["wait", "--idle"]);
}

#[test]
fn the_long_spelling_works_too() {
    let (target, rest) = take_target(&["read", "--target", "dev", "20"]).unwrap();
    assert_eq!(target.as_deref(), Some("dev"));
    assert_eq!(rest, ["read", "20"]);
}

#[test]
fn no_target_leaves_every_argument_alone() {
    let (target, rest) = take_target(&["send", "yes, retry"]).unwrap();
    assert!(target.is_none());
    assert_eq!(rest, ["send", "yes, retry"]);
}

/// ⚠ **The trap this guard exists for.** Without it `-t read` takes the verb as
/// the target name, then runs `status` because nothing is left — so the command
/// does something, silently, and it is not what was asked.
#[test]
fn a_flag_cannot_be_swallowed_as_a_target_name() {
    assert!(take_target(&["-t", "--idle", "wait"]).is_err());
}

#[test]
fn a_target_at_the_end_with_no_name_is_an_error() {
    assert!(take_target(&["status", "-t"]).is_err());
}

#[test]
fn a_flag_value_is_read_back() {
    let got = flag_of(&["wait", "--match", "exit code"], "--match").unwrap();
    assert_eq!(got.as_deref(), Some("exit code"));
}

#[test]
fn an_absent_flag_is_none_rather_than_an_error() {
    assert!(flag_of(&["wait"], "--match").unwrap().is_none());
}

#[test]
fn a_flag_with_no_value_is_an_error() {
    assert!(flag_of(&["wait", "--match"], "--match").is_err());
}

#[test]
fn a_timeout_is_read_as_seconds() {
    assert_eq!(
        timeout_of(&["wait", "--timeout", "90"]).unwrap().as_secs(),
        90
    );
}

#[test]
fn no_timeout_means_the_default_ceiling() {
    assert_eq!(timeout_of(&["wait"]).unwrap(), WAIT_FOR);
}

/// A timeout that is not a number must not fall back to the default: waiting an
/// hour when 90 seconds was asked for looks exactly like a hung session.
#[test]
fn a_timeout_that_is_not_a_number_is_an_error() {
    assert!(timeout_of(&["wait", "--timeout", "soon"]).is_err());
}
