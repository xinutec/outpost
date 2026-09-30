//! Command-line parsing, where every bug has been a parse that quietly did
//! something plausible.

use clap::Parser;
use outpost::args::{Cli, Until, Verb, WAIT_FOR_SECS};

fn parse(argv: &[&str]) -> Result<Cli, clap::Error> {
    Cli::try_parse_from(std::iter::once("outpost").chain(argv.iter().copied()))
}

fn until(timeout: u64, needle: Option<&str>) -> Until {
    Until {
        timeout,
        needle: needle.map(str::to_string),
    }
}

#[test]
fn a_target_is_lifted_out_and_the_verb_survives() {
    let cli = parse(&["-t", "build", "wait", "--idle"]).unwrap();
    assert_eq!(cli.target.as_deref(), Some("build"));
    assert!(matches!(cli.verb, Some(Verb::Wait { idle: true, .. })));
}

#[test]
fn the_long_spelling_works_after_the_verb_too() {
    let cli = parse(&["read", "--target", "dev", "20"]).unwrap();
    assert_eq!(cli.target.as_deref(), Some("dev"));
    assert_eq!(cli.verb, Some(Verb::Read { n: 20, full: false }));
}

#[test]
fn no_verb_is_status() {
    assert_eq!(parse(&[]).unwrap().verb, None);
}

/// ⚠ **The trap the old guard existed for.** `-t read` must not take the verb
/// as the target name and then run `status` because nothing is left.
#[test]
fn a_flag_cannot_be_swallowed_as_a_target_name() {
    assert!(parse(&["-t", "--idle", "wait"]).is_err());
}

#[test]
fn a_target_at_the_end_with_no_name_is_an_error() {
    assert!(parse(&["status", "-t"]).is_err());
}

/// ⚠ **Flags are not message text.** `send "x" --wait` once typed the word
/// "--wait" into the composer and sent it.
#[test]
fn send_keeps_its_flags_out_of_the_text() {
    let cli = parse(&["send", "yes,", "retry", "--wait", "--match", "done"]).unwrap();
    assert_eq!(
        cli.verb,
        Some(Verb::Send {
            text: vec!["yes,".into(), "retry".into()],
            wait: true,
            until: until(WAIT_FOR_SECS, Some("done")),
        })
    );
}

#[test]
fn a_dash_is_text_for_stdin() {
    let cli = parse(&["send", "-"]).unwrap();
    assert!(matches!(cli.verb, Some(Verb::Send { text, .. }) if text == ["-"]));
}

#[test]
fn a_timeout_is_read_as_seconds() {
    let cli = parse(&["wait", "--timeout", "90"]).unwrap();
    assert!(matches!(cli.verb, Some(Verb::Wait { until: u, .. }) if u.timeout == 90));
}

/// A timeout that is not a number must not fall back to the default: waiting an
/// hour when 90 seconds was asked for looks exactly like a hung session.
#[test]
fn a_timeout_that_is_not_a_number_is_an_error() {
    assert!(parse(&["wait", "--timeout", "soon"]).is_err());
}

#[test]
fn bare_task_waits_for_any_and_a_word_names_one() {
    let any = parse(&["wait", "--task"]).unwrap();
    assert!(matches!(
        any.verb,
        Some(Verb::Wait {
            task: Some(None),
            ..
        })
    ));
    let one = parse(&["wait", "--task", "build"]).unwrap();
    assert!(matches!(one.verb, Some(Verb::Wait { task: Some(Some(w)), .. }) if w == "build"));
}

/// The silent shapes clap now refuses: an unknown flag, and a count that is
/// not a number.
#[test]
fn a_misspelt_flag_or_count_is_refused() {
    assert!(parse(&["read", "--ful"]).is_err());
    assert!(parse(&["read", "lots"]).is_err());
    assert!(parse(&["wait", "--mach", "x"]).is_err());
}
