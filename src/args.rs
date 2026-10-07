//! Reading the command line.
//!
//! In the library rather than the binary, because every bug this has had was a
//! parse that quietly did something plausible: `-t read` looking for a target
//! called "read" instead of running the verb, `send "x" --wait` typing the word
//! `--wait` into the composer, `read --ful` ignored. clap refuses all three.

use std::time::Duration;

use clap::{Parser, Subcommand};

/// How long `wait` sits there when `--timeout` says nothing, in seconds.
///
/// ⚠ **A poll costs an ssh round trip and a transcript tail**, so asking often
/// is not free — and the thing being waited for is usually a build measured in
/// tens of minutes. The default ceiling is generous because the alternative,
/// returning early, reads exactly like "it never answered".
pub const WAIT_FOR_SECS: u64 = 3600;

/// Talk to a Claude Code session running on another machine.
#[derive(Parser, Debug)]
#[command(
    name = "outpost",
    after_help = "With one session running, outpost finds it. With more than one,
name the window it is in: -t <window>. `status` lists them.

The host (and any non-session windows, like a bot's log) live in
~/.config/outpost/config.toml:

    default = \"host\"

    [targets.host]
    host = \"<ssh destination>\"
    window = \"<a tmux window to read with `pane`>\""
)]
pub struct Cli {
    /// Which session, by the name of the tmux window it is in; or a config
    /// target for a non-session pane. Omit when only one session is running.
    #[arg(short = 't', long = "target", global = true, value_name = "NAME")]
    pub target: Option<String>,
    /// What to do; `status` when absent.
    #[command(subcommand)]
    pub verb: Option<Verb>,
}

#[derive(Subcommand, Debug, PartialEq, Eq)]
pub enum Verb {
    /// What the session says it is doing, and the windows.
    Status,
    /// The last n exchanges, both sides.
    Read {
        #[arg(default_value_t = 12)]
        n: usize,
        /// Do not shorten long messages.
        #[arg(long)]
        full: bool,
    },
    /// The window itself, with n lines of scrollback.
    Pane {
        #[arg(default_value_t = 0)]
        n: usize,
    },
    /// Type it and press Enter, as them; `-` reads stdin.
    Send {
        #[arg(required = true)]
        text: Vec<String>,
        /// Wait for the reply and print it.
        #[arg(long)]
        wait: bool,
        #[command(flatten)]
        until: Until,
    },
    /// Block until it says something new, then print it.
    Wait {
        /// Wait until it stops working — finished, or asking a question.
        #[arg(long, conflicts_with = "task")]
        idle: bool,
        /// Wait for a background task to END, not for prose; bare waits for any.
        #[arg(long, value_name = "TEXT", num_args = 0..=1)]
        task: Option<Option<String>>,
        #[command(flatten)]
        until: Until,
    },
}

/// How long to wait, and for what.
#[derive(clap::Args, Debug, PartialEq, Eq)]
pub struct Until {
    /// How long to sit there, in seconds.
    #[arg(long, value_name = "SECONDS", default_value_t = WAIT_FOR_SECS)]
    pub timeout: u64,
    /// Ignore turns that do not contain it.
    #[arg(long = "match", value_name = "TEXT")]
    pub needle: Option<String>,
}

impl Until {
    pub fn limit(&self) -> Duration {
        Duration::from_secs(self.timeout)
    }
}
