//! Reading the command line.
//!
//! Pure, and in the library rather than the binary, because every bug these
//! have had was a parse that quietly did something plausible: `-t read` looking
//! for a target called "read" instead of running the verb, `send "x" --wait`
//! typing the word `--wait` into the composer. Those are cheap to pin with a
//! test and expensive to notice in use.

use anyhow::{Context, Result, bail};

/// How long `wait` sits there when `--timeout` says nothing.
///
/// ⚠ **A poll costs an ssh round trip and a transcript tail**, so asking often
/// is not free — and the thing being waited for is usually a build measured in
/// tens of minutes. The default ceiling is generous because the alternative,
/// returning early, reads exactly like "it never answered".
pub const WAIT_FOR: std::time::Duration = std::time::Duration::from_secs(3600);

/// Lift `-t <name>` (or `--target <name>`) out of the arguments.
pub fn take_target<'a>(args: &[&'a str]) -> Result<(Option<String>, Vec<&'a str>)> {
    let mut target = None;
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match *arg {
            "-t" | "--target" => {
                // ⚠ A missing value would otherwise swallow the verb: `-t read`
                // would silently look for a target called "read".
                let name = it.next().context("-t needs a target name")?;
                if name.starts_with('-') {
                    bail!("-t needs a target name, got {name:?}");
                }
                target = Some((*name).to_string());
            }
            other => rest.push(other),
        }
    }
    Ok((target, rest))
}
/// The value of a `--flag value` pair, if it is there.
pub fn flag_of(args: &[&str], flag: &str) -> Result<Option<String>> {
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if *arg == flag {
            let value = it.next().with_context(|| format!("{flag} needs a value"))?;
            return Ok(Some((*value).to_string()));
        }
    }
    Ok(None)
}
/// How long to wait, from `--timeout <seconds>`.
pub fn timeout_of(args: &[&str]) -> Result<std::time::Duration> {
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if *arg == "--timeout" {
            let value = it.next().context("--timeout needs a number of seconds")?;
            let seconds: u64 = value.parse().context("--timeout wants seconds")?;
            return Ok(std::time::Duration::from_secs(seconds));
        }
    }
    Ok(WAIT_FOR)
}
