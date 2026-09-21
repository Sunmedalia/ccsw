//! Paths and process lookup shared by the three clients. Values are never shell-expanded.
use anyhow::{Context, Result, bail};
use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
};

pub fn nonempty_env(key: &str) -> Option<OsString> {
    env::var_os(key).filter(|value| !value.is_empty())
}

pub fn home() -> Result<PathBuf> {
    #[cfg(windows)]
    let value = nonempty_env("USERPROFILE")
        .or_else(|| nonempty_env("HOME"))
        .or_else(|| {
            let mut drive = nonempty_env("HOMEDRIVE")?;
            drive.push(nonempty_env("HOMEPATH")?);
            Path::new(&drive).is_absolute().then_some(drive)
        });
    #[cfg(not(windows))]
    let value = nonempty_env("HOME");
    std::path::absolute(value.context("cannot locate user home directory")?).map_err(Into::into)
}

#[cfg(windows)]
pub fn appdata(local: bool) -> Result<PathBuf> {
    let (key, suffix) = if local {
        ("LOCALAPPDATA", "AppData/Local")
    } else {
        ("APPDATA", "AppData/Roaming")
    };
    match nonempty_env(key) {
        Some(value) => Ok(std::path::absolute(value)?),
        None => Ok(home()?.join(suffix)),
    }
}

pub fn override_path(key: &str, fallback: impl FnOnce() -> Result<PathBuf>) -> Result<PathBuf> {
    let path = match nonempty_env(key) {
        Some(value) => PathBuf::from(value),
        None => fallback()?,
    };
    Ok(std::path::absolute(path)?)
}

pub fn directories() -> Result<(PathBuf, PathBuf, PathBuf)> {
    let directory = |key, suffix: &str, local| {
        override_path(key, || {
            #[cfg(windows)]
            {
                Ok(appdata(local)?.join(suffix))
            }
            #[cfg(not(windows))]
            {
                let _ = local;
                Ok(home()?.join(suffix))
            }
        })
    };
    // XDG variables designate the parent of ccsw, rather than the app directory.
    let xdg = |key, suffix, local| {
        if let Some(value) = nonempty_env(key) {
            Ok(std::path::absolute(PathBuf::from(value).join("ccsw"))?)
        } else {
            directory(key, suffix, local)
        }
    };
    #[cfg(windows)]
    let defaults = ("ccsw", "ccsw/state", "ccsw/cache");
    #[cfg(not(windows))]
    let defaults = (".config/ccsw", ".local/state/ccsw", ".cache/ccsw");
    // A full config override does not require a default config directory.
    let config = if let Some(value) = nonempty_env("CCSW_CONFIG") {
        std::path::absolute(value)?
            .parent()
            .context("config has no parent")?
            .to_path_buf()
    } else {
        xdg("XDG_CONFIG_HOME", defaults.0, false)?
    };
    Ok((
        config,
        xdg("XDG_STATE_HOME", defaults.1, true)?,
        xdg("XDG_CACHE_HOME", defaults.2, true)?,
    ))
}

/// Identity for ordinary configuration binding comparisons, NOT an uninstall safety check.
pub fn identity(path: &Path) -> Result<PathBuf> {
    match fs::canonicalize(path) {
        Ok(path) => return Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let absolute = std::path::absolute(path)?;
    if let (Some(parent), Some(name)) = (absolute.parent(), absolute.file_name()) {
        return Ok(identity(parent)?.join(name));
    }
    Ok(absolute)
}

pub fn same_path(left: &Path, right: &Path) -> Result<bool> {
    Ok(identity(left)? == identity(right)?)
}

pub fn resolve_program(program: &OsStr) -> Result<PathBuf> {
    if program.is_empty() {
        bail!("executable is empty");
    }
    if program
        .to_str()
        .is_some_and(|v| v.starts_with('"') || v.ends_with('"'))
    {
        bail!(
            "executable setting must contain a path without enclosing quotes or command arguments"
        );
    }
    let path = Path::new(program);
    #[cfg(not(windows))]
    {
        if path.components().count() > 1 || path.is_absolute() {
            return Ok(std::path::absolute(path)?);
        }
        // Preserve Unix exec lookup, including executable-bit checks.
        Ok(path.to_path_buf())
    }
    #[cfg(windows)]
    {
        let extensions: Vec<OsString> = nonempty_env("PATHEXT")
            .unwrap_or_else(|| ".EXE;.COM;.CMD;.BAT".into())
            .to_string_lossy()
            .split(';')
            .filter(|ext| {
                [".exe", ".com", ".cmd", ".bat"]
                    .iter()
                    .any(|supported| ext.eq_ignore_ascii_case(supported))
            })
            .map(OsString::from)
            .collect();
        let candidates = |base: &Path| -> Vec<PathBuf> {
            if base.extension().is_some() {
                vec![base.to_path_buf()]
            } else {
                extensions
                    .iter()
                    .map(|ext| {
                        let mut value = base.as_os_str().to_os_string();
                        value.push(ext);
                        PathBuf::from(value)
                    })
                    .collect()
            }
        };
        let bases = if path.components().count() > 1 || path.is_absolute() {
            vec![std::path::absolute(path)?]
        } else {
            env::split_paths(&nonempty_env("PATH").unwrap_or_default())
                .filter(|p| !p.as_os_str().is_empty())
                .map(|p| p.join(path))
                .collect()
        };
        for base in bases {
            for candidate in candidates(&base) {
                if candidate.is_file() {
                    let ext = candidate
                        .extension()
                        .and_then(OsStr::to_str)
                        .unwrap_or_default();
                    if !["exe", "com", "cmd", "bat"]
                        .iter()
                        .any(|value| ext.eq_ignore_ascii_case(value))
                    {
                        bail!(
                            "unsupported executable extension; use a native .exe or a .cmd/.bat launcher"
                        );
                    }
                    return Ok(std::path::absolute(candidate)?);
                }
            }
        }
        bail!(
            "executable not found in PATH or at the configured path: {}",
            path.display()
        )
    }
}
