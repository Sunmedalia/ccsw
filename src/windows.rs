use anyhow::{Context, Result, bail};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

pub fn background(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
}

pub fn startup_path() -> Result<PathBuf> {
    let roaming = env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or(crate::platform::home()?.join("AppData/Roaming"));
    Ok(roaming.join("Microsoft/Windows/Start Menu/Programs/Startup/CCSW Proxy.lnk"))
}

fn shortcut(path: &Path, executable: &Path, registry: &Path) -> Result<()> {
    fs::create_dir_all(path.parent().context("shortcut has no parent")?)?;
    let mut command = Command::new("powershell.exe");
    // Paths travel through environment variables, never through PowerShell source interpolation.
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            r#"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$s = (New-Object -ComObject WScript.Shell).CreateShortcut($env:CCSW_SHORTCUT)
$s.TargetPath = $env:CCSW_EXECUTABLE
$s.Arguments = 'internal proxy-start --registry "' + $env:CCSW_REGISTRY + '"'
$s.WindowStyle = 7
$s.Save()
"#,
        ])
        .env("CCSW_SHORTCUT", path)
        .env("CCSW_EXECUTABLE", executable)
        .env("CCSW_REGISTRY", registry);
    background(&mut command);
    let output = command
        .output()
        .context("cannot create Windows startup shortcut")?;
    if !output.status.success() {
        bail!(
            "cannot create Windows startup shortcut: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

pub fn install(executable: &Path, registry: &Path) -> Result<PathBuf> {
    let path = startup_path()?;
    shortcut(
        &path,
        &std::path::absolute(executable)?,
        &std::path::absolute(registry)?,
    )?;
    Ok(path)
}

pub fn uninstall() -> Result<Option<PathBuf>> {
    let path = startup_path()?;
    if !path.exists() {
        return Ok(None);
    }
    // Use the stored registry, including any custom state directory used at installation.
    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            r#"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
(New-Object -ComObject WScript.Shell).CreateShortcut($env:CCSW_SHORTCUT).Arguments
"#,
        ])
        .env("CCSW_SHORTCUT", &path);
    background(&mut command);
    let output = command.output()?;
    if !output.status.success() {
        bail!("cannot read Windows startup shortcut");
    }
    let arguments = String::from_utf8(output.stdout)?;
    let registry = arguments
        .trim()
        .strip_prefix("internal proxy-start --registry \"")
        .and_then(|s| s.strip_suffix('"'))
        .context("invalid CCSW startup shortcut")?;
    let mut paths = crate::config::AppPaths::discover()?;
    paths.state_dir = Path::new(registry)
        .parent()
        .context("registry has no parent")?
        .to_path_buf();
    if crate::proxy::status(&paths)?.running {
        crate::proxy::stop(&paths)?;
    }
    fs::remove_file(&path)?;
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shortcut_round_trips_paths_with_spaces() {
        let temp = tempfile::tempdir().unwrap();
        let link = temp.path().join("startup test.lnk");
        let exe = temp.path().join("app folder/ccsw.exe");
        let registry = temp.path().join("用户 state/proxy.json");
        shortcut(&link, &exe, &registry).unwrap();
        assert!(link.exists());
        let output = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                r#"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$s = (New-Object -ComObject WScript.Shell).CreateShortcut($env:CCSW_SHORTCUT)
$s.TargetPath
$s.Arguments
"#,
            ])
            .env("CCSW_SHORTCUT", &link)
            .output()
            .unwrap();
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        assert_eq!(
            Path::new(text.lines().next().unwrap().trim_start_matches('\u{feff}')),
            exe.as_path(),
            "{text:?}"
        );
        let arguments = text.lines().nth(1).unwrap();
        let saved_registry = arguments
            .strip_prefix("internal proxy-start --registry \"")
            .and_then(|s| s.strip_suffix('"'))
            .unwrap();
        assert_eq!(Path::new(saved_registry), registry.as_path(), "{text:?}");
        fs::remove_file(link).unwrap();
    }
}
