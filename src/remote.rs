//! Reaching a Claude Code session that is not on this machine.
//!
//! None of the usual ways in works on a session running elsewhere:
//!
//! - **A console can only drive sessions it spawned**, and it spawns them
//!   locally.
//! - **The peer-messaging protocol is the wrong shape**, even where the session
//!   opens a socket for it (`/tmp/cc-socks/<pid>.sock`). A message sent that way
//!   arrives attributed to another *session*. What this tool sends has to
//!   arrive as the person whose keyboard it is standing in for.
//! - **Remote Control is outbound HTTPS only** — no listening socket, so the
//!   vendor's own client is the only thing at the other end of it.
//!
//! What is left is the keyboard: type into the composer the way a person would.
//! That is what this does, and it is why the far side needs a multiplexer that
//! a program can type into and read back with nobody attached.

use anyhow::{Context, Result, bail};
use std::io::Write;
use std::process::{Command, Stdio};

/// Where a session is, and how to address the window it lives in.
///
/// ⚠ **Both come from the environment and neither has a default.** A host name
/// compiled in would make this tool a record of which machines exist, which is
/// not its job and not something a repository should carry.
pub struct Remote {
    /// An ssh destination. A `Host` block in `~/.ssh/config`, never a bare
    /// address: the port and the login live there and are not this tool's to
    /// know.
    pub host: String,
    /// A tmux target, `session:window`.
    ///
    /// ⚠ **By NAME, not by index.** Window numbers are a human habit and get
    /// rearranged; a name survives that, as long as `allow-rename off` stops a
    /// shell taking the name away.
    pub target: String,
}

/// How much of a transcript's tail to read. A live one reaches gigabytes; this
/// covers hundreds of exchanges.
const TAIL: u64 = 8 << 20;

/// What the CLI itself says about a session, from `~/.claude/sessions/<pid>.json`.
///
/// A first-party liveness signal, which is worth preferring: the usual
/// alternative is inferring busy-ness from a running process and a growing
/// transcript, and the CLI writes its own `status` down.
#[derive(serde::Deserialize)]
pub struct Info {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(default)]
    pub pid: u64,
    /// `idle`, `shell`, `busy`. ⚠ `shell` means a command is running, which is
    /// NOT the model thinking and NOT idle.
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, rename = "updatedAt")]
    pub updated_at: u64,
    /// Present only while Remote Control is bridged.
    #[serde(default, rename = "bridgeSessionId")]
    pub bridge: Option<String>,
}

impl Remote {
    /// From `OUTPOST_HOST` and `OUTPOST_WINDOW`.
    pub fn from_env() -> Result<Self> {
        let host = std::env::var("OUTPOST_HOST")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .context("set OUTPOST_HOST to an ssh destination")?;
        let target = std::env::var("OUTPOST_WINDOW")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .context("set OUTPOST_WINDOW to a tmux session:window")?;
        Ok(Self { host, target })
    }

    /// Run a shell snippet on the far side and return its stdout.
    ///
    /// ⚠ **ssh reports two different failures in the same channel.** A remote
    /// command that exits non-zero gives its own status; a connection that never
    /// got there gives 255. Conflating them turns "the host is unreachable" into
    /// "the session said no", so 255 is named separately.
    fn run(&self, script: &str, stdin: Option<&str>) -> Result<String> {
        let mut child = Command::new("ssh")
            .arg("-o")
            .arg("BatchMode=yes")
            .arg(&self.host)
            .arg(script)
            .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("running ssh — is it on PATH?")?;
        if let Some(text) = stdin {
            child
                .stdin
                .take()
                .expect("stdin was piped")
                .write_all(text.as_bytes())
                .context("writing to ssh")?;
        }
        let out = child.wait_with_output().context("waiting for ssh")?;
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        match out.status.code() {
            Some(0) => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
            Some(255) => bail!("could not reach {}: {stderr}", self.host),
            Some(code) => bail!("{} answered {code}: {stderr}", self.host),
            None => bail!("ssh to {} was killed by a signal", self.host),
        }
    }

    /// The window's visible content plus `back` lines of scrollback.
    ///
    /// `capture-pane -p` writes to stdout. The equivalent under screen needed
    /// `hardcopy` to a file on the far side and a second command to read it
    /// back, and could only ever return the visible window — anything that
    /// scrolled past between two polls was gone.
    pub fn pane(&self, back: usize) -> Result<String> {
        self.run(&format!("tmux capture-pane -p -S -{back} -t {} 2>&1", self.target), None)
    }

    /// What the CLI says about itself.
    ///
    /// ⚠ **The file is named by PID, so its name changes on every restart** and
    /// stale siblings accumulate. The newest wins rather than the only one.
    pub fn info(&self) -> Result<Info> {
        // ⚠ **`cat` alone is wrong here.** These files are written without a
        // trailing newline, so two of them concatenate into a single line that
        // parses as nothing — and the loop below skips what it cannot parse, so
        // the failure arrives as "no session registered" rather than as an
        // error. Measured: a restart leaves the previous pid's file behind, so
        // having two is the normal case, not the edge case.
        let raw = self.run(
            "for f in ~/.claude/sessions/*.json; do cat \"$f\"; echo; done 2>/dev/null || true",
            None,
        )?;
        let mut best: Option<Info> = None;
        for line in raw.lines().filter(|line| !line.trim().is_empty()) {
            let Ok(info) = serde_json::from_str::<Info>(line) else {
                continue;
            };
            if best.as_ref().is_none_or(|old| info.updated_at > old.updated_at) {
                best = Some(info);
            }
        }
        best.context("no session registered on the far side — is Claude running?")
    }

    /// The tail of a session's transcript, as raw jsonl.
    ///
    /// Found by looking rather than by rebuilding the path: Claude Code encodes
    /// the working directory into the project directory's name, and that
    /// encoding is undocumented — a guess fails silently and reads as an empty
    /// conversation.
    pub fn transcript(&self, id: &str) -> Result<Vec<u8>> {
        if !id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
            bail!("{id:?} is not a session id");
        }
        let script = format!(
            r#"for d in ~/.claude/projects/*/; do
                 f="${{d}}{id}.jsonl"
                 if [ -f "$f" ]; then tail -c {TAIL} "$f"; exit 0; fi
               done
               echo "no transcript for {id}" >&2; exit 3"#
        );
        Ok(self.run(&script, None)?.into_bytes())
    }

    /// Put text in the composer without pressing Enter.
    ///
    /// ⚠ **Two shells stand between this string and tmux.** The local one is
    /// avoided by passing argv directly, but ssh concatenates its arguments and
    /// hands them to a remote shell, so any quoting done here would have to
    /// survive that. It is not attempted: the text goes over stdin and the
    /// remote shell quotes it once, in `"$(cat)"`, where nothing this side wrote
    /// can change how it parses.
    pub fn type_text(&self, text: &str) -> Result<()> {
        self.run(&format!(r#"tmux send-keys -t {} -l -- "$(cat)""#, self.target), Some(text))?;
        Ok(())
    }

    /// Press Enter in the window.
    pub fn press_enter(&self) -> Result<()> {
        self.run(&format!("tmux send-keys -t {} Enter", self.target), None)?;
        Ok(())
    }

    /// Which windows exist, and what is running in each.
    pub fn windows(&self) -> Result<Vec<(String, String)>> {
        let raw = self.run(
            "tmux list-windows -a -F '#{session_name}:#{window_name} #{pane_current_command}' 2>&1",
            None,
        )?;
        Ok(raw
            .lines()
            .filter_map(|line| line.split_once(' '))
            .map(|(name, cmd)| (name.to_string(), cmd.to_string()))
            .collect())
    }
}
