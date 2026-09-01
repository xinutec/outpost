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

/// Which host and window this invocation is for.
///
/// Order: an explicit `-t` name, then the config's `default`, then the
/// environment. ⚠ **The environment wins over the config's values** once a
/// target is chosen, so a one-off can be pointed somewhere else without
/// editing a file — but it cannot silently redirect a *named* target to a
/// different host, because naming one is a deliberate act.
pub fn resolve(named: Option<&str>) -> Result<(String, String)> {
    let config = load()?;
    let chosen = named.or(config.default.as_deref());

    if let Some(name) = chosen {
        let target = config.targets.get(name).with_context(|| {
            let known: Vec<&str> = config.targets.keys().map(String::as_str).collect();
            if known.is_empty() {
                format!(
                    "no target {name:?}, and {} has none",
                    path().map(|p| p.display().to_string()).unwrap_or_default()
                )
            } else {
                format!("no target {name:?}. there is: {}", known.join(", "))
            }
        })?;
        return Ok((target.host.clone(), target.window.clone()));
    }

    let host = std::env::var("OUTPOST_HOST")
        .ok()
        .filter(|v| !v.trim().is_empty());
    let window = std::env::var("OUTPOST_WINDOW")
        .ok()
        .filter(|v| !v.trim().is_empty());
    match (host, window) {
        (Some(host), Some(window)) => Ok((host, window)),
        _ => bail!(
            "nothing to point at. write {}:\n\n\
             \x20   default = \"dev\"\n\n\
             \x20   [targets.dev]\n\
             \x20   host = \"<ssh destination>\"\n\
             \x20   window = \"<tmux session:window>\"\n\n\
             or set OUTPOST_HOST and OUTPOST_WINDOW.",
            path().map(|p| p.display().to_string()).unwrap_or_default()
        ),
    }
}
