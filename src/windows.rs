pub mod process;
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
    ffi::{OsStr, OsString},
    fs,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    process::Command,
};

// Serializes the brief standard-handle adjustment with managed child creation.
pub(super) static SPAWN_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn spawn_background(command: &mut Command) -> Result<std::process::Child> {
    use ::windows::Win32::{
        Foundation::{
            GetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT, HANDLE_FLAGS, SetHandleInformation,
        },
        System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE},
    };
    use std::os::windows::process::CommandExt;
    struct Restore(Vec<(HANDLE, u32)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            for (handle, flags) in &self.0 {
                // SAFETY: borrowed standard handles remain owned by the process.
                let _ = unsafe {
                    SetHandleInformation(
                        *handle,
                        HANDLE_FLAG_INHERIT.0,
                        HANDLE_FLAGS(*flags & HANDLE_FLAG_INHERIT.0),
                    )
                };
            }
        }
    }
    let _lock = SPAWN_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("process spawn lock poisoned"))?;
    let mut restore = Restore(Vec::new());
    for id in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: inspect borrowed standard handles, restore every changed flag on all exits.
        unsafe {
            if let Ok(handle) = GetStdHandle(id)
                && !handle.is_invalid()
            {
                let mut flags = 0;
                GetHandleInformation(handle, &mut flags)?;
                restore.0.push((handle, flags));
                SetHandleInformation(handle, HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0))?;
            }
        }
    }
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW; deliberately outside a client Job.
    Ok(command.spawn()?)
}

pub fn startup_path() -> Result<PathBuf> {
    Ok(crate::platform::appdata(false)?
        .join("Microsoft/Windows/Start Menu/Programs/Startup/CCSW Proxy.lnk"))
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

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

// Windows argv quoting, not cmd.exe quoting. Used ONLY for our Shell Link target.
fn quote_argument(value: &OsStr) -> Vec<u16> {
    let mut result = vec![b'"' as u16];
    let mut slashes = 0;
    for unit in value.encode_wide() {
        if unit == b'\\' as u16 {
            slashes += 1;
            continue;
        }
        let count = if unit == b'"' as u16 {
            slashes * 2 + 1
        } else {
            slashes
        };
        result.extend(std::iter::repeat_n(b'\\' as u16, count));
        result.push(unit);
        slashes = 0;
    }
    result.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    result.push(b'"' as u16);
    result
}

fn arguments(registry: &Path) -> Result<OsString> {
    let mut value: Vec<u16> = "internal proxy-start --registry ".encode_utf16().collect();
    value.extend(quote_argument(registry.as_os_str()));
    if value.contains(&0) || value.len() >= 32767 {
        anyhow::bail!("invalid or oversized startup path");
    }
    Ok(OsString::from_wide(&value))
}

fn parse_registry(arguments: &OsStr) -> Result<PathBuf> {
    use ::windows::Win32::{
        Foundation::{HLOCAL, LocalFree},
        UI::Shell::CommandLineToArgvW,
    };
    let mut command = OsString::from("ccsw.exe ");
    command.push(arguments);
    let command = wide(&command);
    let mut count = 0;
    // SAFETY: terminated UTF-16 input; copy all strings before releasing the allocation.
    let values = unsafe {
        let argv = CommandLineToArgvW(PCWSTR(command.as_ptr()), &mut count);
        if argv.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        let values = std::slice::from_raw_parts(argv, count as usize)
            .iter()
            .map(|value| OsString::from_wide(value.as_wide()))
            .collect::<Vec<_>>();
        LocalFree(Some(HLOCAL(argv.cast())));
        values
    };
    if values.len() != 5
        || values[1] != "internal"
        || values[2] != "proxy-start"
        || values[3] != "--registry"
    {
        anyhow::bail!("invalid CCSW startup shortcut arguments");
    }
    let path = PathBuf::from(&values[4]);
    if !path.is_absolute() || path.file_name().is_none_or(|name| name != "proxy.json") {
        anyhow::bail!("startup registry must be an absolute proxy.json path");
    }
    Ok(path)
}

fn shortcut(path: &Path, executable: &Path, registry: &Path) -> Result<()> {
    fs::create_dir_all(path.parent().context("shortcut has no parent")?)?;
    let _com = ComApartment::new()?;
    let target = wide(executable.as_os_str());
    let filename = wide(path.as_os_str());
    let arguments = wide(&arguments(registry)?);
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

fn read_shortcut(path: &Path) -> Result<(PathBuf, OsString)> {
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
    let decode = |buffer: &[u16]| -> Result<OsString> {
        let end = buffer
            .iter()
            .position(|ch| *ch == 0)
            .context("unterminated shortcut value")?;
        Ok(OsString::from_wide(&buffer[..end]))
    };
    Ok((PathBuf::from(decode(&target)?), decode(&arguments)?))
}

fn owned_registry(path: &Path, executable: &Path) -> Result<PathBuf> {
    let (target, args) = read_shortcut(path)?;
    if !crate::platform::same_path(&target, executable)? {
        anyhow::bail!(
            "startup shortcut targets another executable; remove it using the original CCSW installation"
        );
    }
    parse_registry(&args)
}

pub fn install(executable: &Path, registry: &Path) -> Result<PathBuf> {
    let path = startup_path()?;
    let executable = std::path::absolute(executable)?;
    let registry = std::path::absolute(registry)?;
    if path.exists()
        && !crate::platform::same_path(&owned_registry(&path, &executable)?, &registry)?
    {
        anyhow::bail!("startup belongs to another CCSW configuration");
    }
    fs::create_dir_all(path.parent().context("startup has no parent")?)?;
    let temporary = tempfile::Builder::new()
        .suffix(".lnk")
        .tempfile_in(path.parent().unwrap())?
        .into_temp_path();
    shortcut(&temporary, &executable, &registry)?;
    temporary.persist(&path).map_err(|error| error.error)?;
    Ok(path)
}

pub(crate) fn startup_registry(path: &Path) -> Result<PathBuf> {
    owned_registry(path, &std::env::current_exe()?)
}

pub fn uninstall() -> Result<Option<PathBuf>> {
    let path = startup_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let registry = startup_registry(&path)?;
    let paths = crate::config::AppPaths::discover()?;
    if !crate::platform::same_path(&registry, &paths.state_dir.join("proxy.json"))? {
        anyhow::bail!("startup belongs to another CCSW configuration");
    }
    if crate::proxy::status(&paths)?.running {
        crate::proxy::stop(&paths)?;
    }
    fs::remove_file(&path)?;
    Ok(Some(path))
}

pub fn open_browser(url: &str) -> Result<()> {
    use ::windows::{
        Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        core::w,
    };
    let url = wide(OsStr::new(url));
    // SAFETY: live, terminated Unicode string; Windows opens the registered URL handler.
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(url.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize <= 32 {
        anyhow::bail!("Windows could not open the browser; open the displayed URL manually");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shortcut_round_trips_paths_with_spaces() {
        let temp = tempfile::tempdir().unwrap();
        let link = temp.path().join("用户 startup test.lnk");
        let exe = std::env::current_exe().unwrap();
        let registry = temp.path().join("用户 & %literal% !^() state/proxy.json");
        shortcut(&link, &exe, &registry).unwrap();
        assert!(crate::platform::same_path(&read_shortcut(&link).unwrap().0, &exe).unwrap());
        assert_eq!(startup_registry(&link).unwrap(), registry);
    }
    #[test]
    fn startup_parser_rejects_extra_arguments_and_relative_paths() {
        for value in [
            "internal proxy-start --registry relative/proxy.json",
            "internal proxy-start --registry C:\\state\\proxy.json --extra",
            "internal proxy-serve --registry C:\\state\\proxy.json",
        ] {
            assert!(parse_registry(OsStr::new(value)).is_err());
        }
    }
    #[test]
    fn quoting_round_trips_special_characters() {
        for name in [
            "用户 & %TEMP% !^()",
            "name with spaces",
            "emoji 🦀",
            "back\\slash",
        ] {
            let path = PathBuf::from("C:\\").join(name).join("proxy.json");
            assert_eq!(parse_registry(&arguments(&path).unwrap()).unwrap(), path);
        }
        // Quotes are legal argv content, though not legal Windows filenames.
        let quoted = quote_argument(OsStr::new("a\\\"b\\"));
        assert_eq!(
            quoted,
            "\"a\\\\\\\"b\\\\\"".encode_utf16().collect::<Vec<_>>()
        );
    }
}
