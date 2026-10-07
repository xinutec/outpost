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

/// How to reach a machine. The *window* a session lives in is no longer part
/// of this: a session names its own tmux pane in its state file, so it is read
/// off [`Info`] rather than configured here.
///
/// ⚠ **The host has no default and is never compiled in.** A host name baked
/// into a public binary would make it a record of which machines exist, which
/// is not its job.
pub struct Remote {
    /// An ssh destination. A `Host` block in `~/.ssh/config`, never a bare
    /// address: the port and the login live there and are not this tool's to
    /// know.
    pub host: String,
}

/// How much of a transcript's tail to read. A live one reaches gigabytes; this
/// covers hundreds of exchanges.
const TAIL: u64 = 8 << 20;

/// What the CLI itself says about a session, from `~/.claude/sessions/<pid>.json`.
///
/// A first-party liveness signal, which is worth preferring: the usual
/// alternative is inferring busy-ness from a running process and a growing
/// transcript, and the CLI writes its own `status` down.
#[derive(serde::Deserialize, Clone, Debug)]
pub struct Info {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(default)]
    pub pid: u64,
    /// The tmux pane this session runs in, as the CLI records it:
    /// `session:@window.%pane`. ⚠ **This is how a session is located**, so two
    /// Claudes in one tmux server never have to be told apart by guesswork — each
    /// says where it is. Empty when the session is not under tmux.
    #[serde(default)]
    pub tmux: String,
    /// The tmux window's *name*, filled in after reading the file by matching
    /// the window id in `tmux` against the live window list. Not in the state
    /// file. This is what a person selects on (`-t security`), because it is the
    /// tab they named, not the session's internal id.
    #[serde(skip)]
    pub window: Option<String>,
    /// `idle`, `shell`, `busy`, `waiting`. ⚠ `shell` means a command is
    /// running, which is NOT the model thinking and NOT idle. `waiting` means
    /// it is blocked on a question or menu and needs an answer.
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

impl Info {
    /// The tmux window id out of `tmux` (`tox:@2.%2` -> `@2`), used to look the
    /// window's name up in the live list.
    pub fn window_id(&self) -> Option<&str> {
        let after = self.tmux.split_once(':')?.1;
        Some(after.split_once('.').map_or(after, |(w, _)| w))
    }

    /// The tmux pane id out of `tmux` (`tox:@2.%2` -> `%2`). A pane id is unique
    /// across the whole tmux server, so it addresses this session's pane with no
    /// window name in the way — which matters because the window gets renamed.
    pub fn pane(&self) -> Option<&str> {
        self.tmux
            .rsplit_once('.')
            .map(|(_, pane)| pane)
            .filter(|pane| pane.starts_with('%'))
    }

    /// The tmux target to type into or capture, or an error naming the session
    /// that is not in tmux rather than letting a later `send-keys` fail obscurely.
    pub fn target(&self) -> Result<&str> {
        self.pane()
            .context("this session is not attached to a tmux pane")
    }

    /// What to call this session in a list or a prompt: the window name it is in,
    /// or a short session id when it has no window name yet.
    pub fn label(&self) -> String {
        self.window
            .clone()
            .unwrap_or_else(|| self.session_id.chars().take(8).collect())
    }

    /// Whether the session has stopped working — either it finished its turn
    /// (`idle`) or it is blocked on a question (`waiting`).
    ///
    /// ⚠ **Both are "not working", and a waiter that knew only `idle` sat
    /// silent through a question until timeout** — the one moment an answer was
    /// most wanted. `busy` and `shell` are the working states; everything else
    /// observed so far is a stop.
    pub fn stopped(&self) -> bool {
        matches!(self.status.as_str(), "idle" | "waiting")
    }

    /// Whether it stopped because it is asking something, rather than because
    /// it finished. The two call for different next moves, so the caller is told
    /// which.
    pub fn waiting(&self) -> bool {
        self.status == "waiting"
    }
}

impl Remote {
    /// The host for this invocation, from the config file or the environment.
    /// See [`crate::config`]. The window is no longer resolved here — a session
    /// says where it is.
    pub fn resolve(selector: Option<&str>) -> Result<Self> {
        Ok(Self {
            host: crate::config::host(selector)?,
        })
    }

    /// Run a shell snippet on the far side and return its stdout.
    ///
    /// ⚠ **ssh reports two different failures in the same channel.** A remote
    /// command that exits non-zero gives its own status; a connection that never
    /// got there gives 255. Conflating them turns "the host is unreachable" into
    /// "the session said no", so 255 is named separately.
    fn run(&self, script: &str, stdin: Option<&str>) -> Result<String> {
        let mut child = Command::new("ssh")
            // ⚠ **Without these, a hung connection outlives every deadline this
            // tool has.** `--timeout` bounds the polling LOOP and is only
            // checked between polls, so one stalled ssh blocks forever and the
            // wait never gives up — which is why callers kept wrapping the
            // whole thing in a shell `timeout`, a backstop that kills the
            // process and explains nothing. Bounding the connection here makes
            // that wrapper unnecessary.
            //
            // ConnectTimeout covers never reaching the host; the keepalives
            // cover the worse case, a session that established and then went
            // silent, which TCP alone will sit on for hours.
            .args(["-o", "BatchMode=yes"])
            .args(["-o", "ConnectTimeout=10"])
            .args(["-o", "ServerAliveInterval=15"])
            .args(["-o", "ServerAliveCountMax=2"])
            .arg(&self.host)
            .arg(script)
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
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
    pub fn pane(&self, target: &str, back: usize) -> Result<String> {
        self.run(
            &format!("tmux capture-pane -p -S -{back} -t {target} 2>&1"),
            None,
        )
    }

    /// Every Claude session that is actually RUNNING on the host, each with the
    /// name of the tmux window it lives in.
    ///
    /// ⚠ **Liveness is checked, not assumed.** The state files are named by pid
    /// and a dead one is left behind on every restart, so an earlier version
    /// that picked "the newest file" could pick a corpse. `kill -0 <pid>` is the
    /// test; the pid is the filename, so no JSON parsing is needed to apply it.
    ///
    /// ⚠ **One round trip, two answers.** The window names come from the same
    /// ssh call as the session files, past a marker line, because a second call
    /// to resolve them would double the cost of every `wait` poll.
    pub fn sessions(&self) -> Result<Vec<Info>> {
        let raw = self.run(
            r#"for f in ~/.claude/sessions/*.json; do
                 [ -f "$f" ] || continue
                 pid=${f##*/}; pid=${pid%.json}
                 case "$pid" in ''|*[!0-9]*) continue;; esac
                 kill -0 "$pid" 2>/dev/null && { cat "$f"; echo; }
               done 2>/dev/null
               echo "@@@WINDOWS@@@"
               tmux list-windows -a -F '#{window_id} #{window_name}' 2>/dev/null || true"#,
            None,
        )?;
        let (files, windows) = raw
            .split_once("@@@WINDOWS@@@")
            .unwrap_or((raw.as_str(), ""));
        let names: std::collections::HashMap<&str, &str> = windows
            .lines()
            .filter_map(|line| line.split_once(' '))
            .collect();
        let mut out = Vec::new();
        for line in files.lines().filter(|line| !line.trim().is_empty()) {
            let Ok(mut info) = serde_json::from_str::<Info>(line) else {
                continue;
            };
            info.window = info
                .window_id()
                .and_then(|id| names.get(id))
                .map(|name| (*name).to_string());
            out.push(info);
        }
        Ok(out)
    }

    /// The current state of one session, by pid, re-read for a poll.
    ///
    /// ⚠ **A vanished file is a real event, not an error to swallow:** the
    /// session exited or was respawned under a new pid. Callers waiting on it
    /// want to be told, not to spin.
    pub fn status_of(&self, pid: u64) -> Result<Info> {
        let raw = self.run(
            &format!("cat ~/.claude/sessions/{pid}.json 2>/dev/null; echo"),
            None,
        )?;
        let line = raw.lines().find(|line| !line.trim().is_empty());
        line.and_then(|line| serde_json::from_str::<Info>(line).ok())
            .with_context(|| format!("session {pid} is no longer registered — it may have exited"))
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
    pub fn type_text(&self, target: &str, text: &str) -> Result<()> {
        self.run(
            &format!(r#"tmux send-keys -t {target} -l -- "$(cat)""#),
            Some(text),
        )?;
        Ok(())
    }

    /// Press Enter in the window.
    pub fn press_enter(&self, target: &str) -> Result<()> {
        self.run(&format!("tmux send-keys -t {target} Enter"), None)?;
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

/// How the available sessions read in an error message: `: security, development`,
/// or empty when there are none.
fn available(sessions: &[Info]) -> String {
    if sessions.is_empty() {
        return String::new();
    }
    let names: Vec<String> = sessions.iter().map(Info::label).collect();
    format!(": {}", names.join(", "))
}

/// Pick the session to act on.
///
/// ⚠ **One running session is unambiguous; more than one is not, and guessing
/// is the bug this exists to prevent.** An earlier version read whichever state
/// file was newest, so with two Claudes up it would flip between them mid-task.
/// The rule instead: with a name, take the session in the window of that name;
/// with none, take the only session there is, or refuse and list them.
pub fn choose<'a>(sessions: &'a [Info], selector: Option<&str>) -> Result<&'a Info> {
    match selector {
        Some(name) => {
            let mut hits = sessions
                .iter()
                .filter(|s| s.window.as_deref() == Some(name));
            match (hits.next(), hits.next()) {
                (Some(one), None) => Ok(one),
                (None, _) => bail!(
                    "no running session in a window named {name:?}{}",
                    available(sessions)
                ),
                (Some(_), Some(_)) => {
                    bail!("more than one running session is in a window named {name:?}")
                }
            }
        }
        None => match sessions {
            [] => bail!("no running session on the far side — is Claude running?"),
            [one] => Ok(one),
            many => bail!(
                "{} sessions are running{} — name one with -t <window>",
                many.len(),
                available(many)
            ),
        },
    }
}
