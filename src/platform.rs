use anyhow::{Context, Result};
use std::{env, path::PathBuf};

pub fn home() -> Result<PathBuf> {
    #[cfg(windows)]
    let value = env::var_os("USERPROFILE").or_else(|| env::var_os("HOME"));
    #[cfg(not(windows))]
    let value = env::var_os("HOME");
    value
        .map(PathBuf::from)
        .context("cannot locate user home directory")
}

pub fn directories() -> Result<(PathBuf, PathBuf, PathBuf)> {
    let home = home()?;
    #[cfg(windows)]
    let defaults = {
        let roaming = env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Roaming"));
        let local = env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Local"));
        (
            roaming.join("ccsw"),
            local.join("ccsw/state"),
            local.join("ccsw/cache"),
        )
    };
    #[cfg(not(windows))]
    let defaults = (
        home.join(".config/ccsw"),
        home.join(".local/state/ccsw"),
        home.join(".cache/ccsw"),
    );
    let resolve = |key, fallback| {
        env::var_os(key)
            .map(|p| PathBuf::from(p).join("ccsw"))
            .unwrap_or(fallback)
    };
    Ok((
        resolve("XDG_CONFIG_HOME", defaults.0),
        resolve("XDG_STATE_HOME", defaults.1),
        resolve("XDG_CACHE_HOME", defaults.2),
    ))
}
