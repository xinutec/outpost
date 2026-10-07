//! Where to point, without saying so on every command line.
//!
//! ⚠ **The config lives OUTSIDE this repository on purpose** — at
//! `~/.config/outpost/config.toml`. Host names are the one thing this tool must
//! not carry: the binary is public, and a checked-in target list would make it a
//! record of which machines exist and what runs on them. Keeping it in the
//! user's config directory also keeps it out of any dotfiles repo that is
//! itself published.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Default)]
pub struct Config {
    /// The target used when none is named on the command line.
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub targets: std::collections::BTreeMap<String, Target>,
}

#[derive(Deserialize, Clone)]
pub struct Target {
    pub host: String,
    pub window: String,
}

/// `$XDG_CONFIG_HOME/outpost/config.toml`, else `~/.config/outpost/config.toml`.
pub fn path() -> Result<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME")
        && !xdg.is_empty()
    {
        return Ok(PathBuf::from(xdg).join("outpost/config.toml"));
    }
    let home = std::env::var("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".config/outpost/config.toml"))
}

pub fn load() -> Result<Config> {
    let path = path()?;
    // A missing file is not an error: the environment variables alone are a
    // complete way to use this, and saying "no such file" for a first run would
    // be a worse message than the one `resolve` gives.
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(Config::default());
    };
    toml::from_str(&text).with_context(|| format!("reading {}", path.display()))
}

/// Which host this invocation is for.
///
/// ⚠ **No longer resolves a window** — a session names its own tmux pane, so the
/// window is read off the session, not the config. What is left is the host:
/// `-t <name>` pointing at a config target uses that target's host, otherwise
/// the `default` target's host, otherwise `OUTPOST_HOST`. A selector that names
/// no config target is NOT an error here — it is almost always a session's
/// window name, which the caller resolves against the live sessions.
pub fn host(selector: Option<&str>) -> Result<String> {
    let config = load()?;
    if let Some(name) = selector
        && let Some(target) = config.targets.get(name)
    {
        return Ok(target.host.clone());
    }
    if let Some(default) = config.default.as_deref()
        && let Some(target) = config.targets.get(default)
    {
        return Ok(target.host.clone());
    }
    if let Ok(host) = std::env::var("OUTPOST_HOST")
        && !host.trim().is_empty()
    {
        return Ok(host);
    }
    bail!(
        "no host to reach. write {} with a default target, or set OUTPOST_HOST.",
        path().map(|p| p.display().to_string()).unwrap_or_default()
    )
}

/// A plain tmux window a config target points at — for panes that are NOT a
/// Claude session, like a bot's log or the build shell. `None` when the selector
/// names no config target (then it is a session's window name instead).
pub fn window_target(selector: Option<&str>) -> Result<Option<String>> {
    let Some(name) = selector else {
        return Ok(None);
    };
    let config = load()?;
    Ok(config.targets.get(name).map(|t| t.window.clone()))
}
