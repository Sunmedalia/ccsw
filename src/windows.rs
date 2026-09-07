use ::windows::{
    Win32::{
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize, IPersistFile, STGM_READ,
        },
        UI::{
            Shell::{IShellLinkW, ShellLink},
            WindowsAndMessaging::SW_SHOWMINNOACTIVE,
        },
    },
    core::{Interface, PCWSTR},
};
use anyhow::{Context, Result};
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

// Each operation initializes COM on its calling thread, then releases interfaces
// before balancing initialization. All strings use the Unicode Shell Link API.
struct ComApartment;
impl ComApartment {
    fn new() -> Result<Self> {
        // SAFETY: no reserved pointer; this thread owns the matching uninitialize.
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        }
        Ok(Self)
    }
}
impl Drop for ComApartment {
    fn drop(&mut self) {
        // SAFETY: constructed only after successful CoInitializeEx on this thread.
        unsafe {
            CoUninitialize();
        }
    }
}

fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value.encode_wide().chain(Some(0)).collect()
}

fn shortcut(path: &Path, executable: &Path, registry: &Path) -> Result<()> {
    fs::create_dir_all(path.parent().context("shortcut has no parent")?)?;
    let _com = ComApartment::new()?;
    let target = wide(executable.as_os_str());
    let filename = wide(path.as_os_str());
    let mut arguments = std::ffi::OsString::from("internal proxy-start --registry \"");
    arguments.push(registry);
    arguments.push("\"");
    let arguments = wide(&arguments);
    // SAFETY: COM is initialized; all UTF-16 buffers are NUL-terminated and live
    // through each call. The generated bindings own the interface references.
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(PCWSTR(target.as_ptr()))?;
        link.SetArguments(PCWSTR(arguments.as_ptr()))?;
        link.SetShowCmd(SW_SHOWMINNOACTIVE)?;
        let file: IPersistFile = link.cast()?;
        file.Save(PCWSTR(filename.as_ptr()), true)?;
    }
    Ok(())
}

fn read_shortcut(path: &Path) -> Result<(PathBuf, String)> {
    let _com = ComApartment::new()?;
    let filename = wide(path.as_os_str());
    let mut target = vec![0u16; 32768];
    let mut arguments = vec![0u16; 32768];
    // SAFETY: COM is initialized and all buffers have valid lifetimes and lengths.
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        let file: IPersistFile = link.cast()?;
        file.Load(PCWSTR(filename.as_ptr()), STGM_READ)?;
        link.GetPath(&mut target, std::ptr::null_mut(), 0)?;
        link.GetArguments(&mut arguments)?;
    }
    let decode = |buffer: &[u16]| {
        String::from_utf16(
            &buffer[..buffer
                .iter()
                .position(|ch| *ch == 0)
                .unwrap_or(buffer.len())],
        )
    };
    Ok((PathBuf::from(decode(&target)?), decode(&arguments)?))
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
    let (_, arguments) = read_shortcut(&path)?;
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
        let link = temp.path().join("用户 startup test.lnk");
        let exe = temp.path().join("用户 app folder/ccsw.exe");
        let registry = temp.path().join("用户 state/proxy.json");
        shortcut(&link, &exe, &registry).unwrap();
        assert!(link.exists());
        let (target, arguments) = read_shortcut(&link).unwrap();
        assert_eq!(target, exe);
        let saved_registry = arguments
            .strip_prefix("internal proxy-start --registry \"")
            .and_then(|s| s.strip_suffix('"'))
            .unwrap();
        assert_eq!(Path::new(saved_registry), registry.as_path());
        fs::remove_file(link).unwrap();
    }
}
