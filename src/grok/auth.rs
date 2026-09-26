//! Delegate OAuth to Grok Build. CCSW never stores or refreshes session tokens.
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::{
    ffi::OsStr,
    fs,
    io::{BufRead, BufReader, Read},
    path::Path,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub saved: bool,
    pub email: Option<String>,
    pub expired: bool,
    pub refreshable: bool,
}
impl Status {
    pub fn description(&self) -> String {
        if !self.saved {
            return "No saved Grok OAuth login".into();
        }
        format!(
            "Saved OAuth login{} · {}",
            self.email
                .as_deref()
                .map(|email| format!(": {email}"))
                .unwrap_or_default(),
            if self.expired && self.refreshable {
                "token expired; Grok can attempt refresh"
            } else if self.expired {
                "token expired; sign in again"
            } else {
                "local credentials found; not validated online"
            }
        )
    }
}
fn read_auth(home: &Path) -> Result<Option<Value>> {
    let path = home.join("auth.json");
    if !path.exists() {
        return Ok(None);
    }
    if fs::symlink_metadata(&path)?.file_type().is_symlink() {
        bail!("Refusing symlink Grok auth.json");
    }
    let mut bytes = Vec::new();
    fs::File::open(&path)?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        bail!("Grok auth.json exceeds size limit");
    }
    let auth: Value = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("Cannot read Grok auth.json; sign in again"))?;
    if !auth.is_object() {
        bail!("Grok auth.json must be an object");
    }
    Ok(Some(auth))
}
fn entry_status(entry: &Value) -> Status {
    let mode = entry["auth_mode"].as_str().unwrap_or("");
    let nonempty = |key: &str| entry[key].as_str().is_some_and(|s| !s.is_empty());
    if !["oidc", "oauth", "oauth2", "web_login", "web-login"].contains(&mode)
        || (!nonempty("key") && !nonempty("access_token") && !nonempty("refresh_token"))
    {
        return Status::default();
    }
    Status {
        saved: true,
        email: entry["email"]
            .as_str()
            .map(|s| s.chars().filter(|c| !c.is_control()).take(200).collect()),
        expired: entry["expires_at"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .is_some_and(|time| time < chrono::Utc::now()),
        refreshable: nonempty("refresh_token"),
    }
}
/// Read the same native session used for status. This value contains secrets and
/// must never be logged, sent through UI channels, or serialized into CCSW config.
pub(super) fn saved_entry(home: &Path) -> Result<Option<Value>> {
    let Some(auth) = read_auth(home)? else {
        return Ok(None);
    };
    let object = auth
        .as_object()
        .context("Grok auth.json must be an object")?;
    let candidates: Vec<&Value> = if object.contains_key("auth_mode") {
        vec![&auth]
    } else {
        object.values().collect()
    };
    let mut best: Option<&Value> = None;
    for entry in candidates {
        let current = entry_status(entry);
        if current.saved && best.is_none_or(|old| entry_status(old).expired && !current.expired) {
            best = Some(entry);
        }
    }
    Ok(best.cloned())
}
pub fn status(home: &Path) -> Result<Status> {
    Ok(saved_entry(home)?
        .as_ref()
        .map(entry_status)
        .unwrap_or_default())
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Browser,
    Device,
    Logout,
}
impl Action {
    fn args(self) -> &'static [&'static str] {
        match self {
            Self::Browser => &["login", "--oauth"],
            Self::Device => &["login", "--device-auth"],
            Self::Logout => &["logout"],
        }
    }
}
/// Only authorization links and human-readable device codes can enter the UI.
/// Raw child output (including OAuth callback codes or tokens) is discarded.
#[derive(Default)]
struct Progress {
    waiting_code: bool,
}
impl Progress {
    fn parse(&mut self, line: &str) -> Option<String> {
        let line = line.trim();
        if [
            "Then enter this code:",
            "Confirm this code in your browser:",
        ]
        .contains(&line)
        {
            self.waiting_code = true;
            return None;
        }
        if self.waiting_code && !line.is_empty() {
            self.waiting_code = false;
            if (4..=32).contains(&line.len())
                && line
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-')
            {
                return Some(format!("Device code: {line}"));
            }
        }
        for text in line.split_whitespace() {
            let text = text.trim_matches(|c| matches!(c, '(' | ')' | '"' | '\''));
            let Ok(url) = url::Url::parse(text) else {
                continue;
            };
            if url.scheme() != "https"
                || !url.username().is_empty()
                || url.password().is_some()
                || url.fragment().is_some()
                || !matches!(
                    url.host_str(),
                    Some("auth.x.ai" | "accounts.x.ai" | "grok.com")
                )
            {
                continue;
            }
            if !url.path().contains("auth") && !url.path().contains("device") {
                continue;
            }
            if url.query_pairs().any(|(key, _)| {
                ![
                    "client_id",
                    "response_type",
                    "redirect_uri",
                    "scope",
                    "state",
                    "code_challenge",
                    "code_challenge_method",
                    "user_code",
                    "prompt",
                    "audience",
                    "resource",
                    "login_hint",
                ]
                .contains(&key.as_ref())
            }) {
                continue;
            }
            return Some(format!("Authorize: {url}"));
        }
        None
    }
}
fn read_progress(pipe: impl Read + Send + 'static, sender: mpsc::SyncSender<String>) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(pipe);
        let mut progress = Progress::default();
        let mut discard = false;
        loop {
            let mut bytes = Vec::new();
            match reader
                .by_ref()
                .take(16 * 1024 + 1)
                .read_until(b'\n', &mut bytes)
            {
                Ok(0) | Err(_) => break,
                Ok(n) if n > 16 * 1024 || discard => {
                    progress.waiting_code = false;
                    discard = bytes.last() != Some(&b'\n');
                }
                _ => {
                    if let Ok(line) = std::str::from_utf8(&bytes)
                        && let Some(safe) = progress.parse(line)
                        && sender.send(safe).is_err()
                    {
                        break;
                    }
                }
            }
        }
    });
}
pub fn run(
    action: Action,
    home: &Path,
    cancel: &AtomicBool,
    notify: impl FnMut(String),
) -> Result<Status> {
    let program = crate::platform::nonempty_env("CCSW_GROK_BIN").unwrap_or_else(|| "grok".into());
    run_program(
        &program,
        action,
        home,
        cancel,
        Duration::from_secs(15 * 60),
        notify,
    )
}
fn run_program(
    program: &OsStr,
    action: Action,
    home: &Path,
    cancel: &AtomicBool,
    timeout: Duration,
    mut notify: impl FnMut(String),
) -> Result<Status> {
    if cancel.load(Ordering::Relaxed) {
        bail!("Grok authorization cancelled");
    }
    fs::create_dir_all(home)?;
    let mut command = Command::new(crate::platform::resolve_program(program)?);
    command
        .args(action.args())
        .current_dir(home)
        .env("GROK_HOME", home)
        .env_remove("GROK_CONFIG")
        .env_remove("GROK_CONFIG_PATH")
        .env_remove("XAI_API_KEY")
        .env_remove("GROK_CODE_XAI_API_KEY")
        .env_remove("RUST_LOG")
        .env_remove("GROK_DEBUG")
        .env_remove("GROK_DEBUG_FILE")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = crate::managed_process::ManagedChild::spawn(&mut command)
        .context("Cannot start Grok CLI; install it or set CCSW_GROK_BIN")?;
    let (sender, receiver) = mpsc::sync_channel(32);
    read_progress(
        child.stdout.take().context("Grok stdout unavailable")?,
        sender.clone(),
    );
    read_progress(
        child.stderr.take().context("Grok stderr unavailable")?,
        sender,
    );
    let deadline = Instant::now() + timeout;
    let result = loop {
        while let Ok(progress) = receiver.try_recv() {
            notify(progress);
        }
        if cancel.load(Ordering::Relaxed) {
            break Err(anyhow::anyhow!("Grok authorization cancelled"));
        }
        if Instant::now() >= deadline {
            break Err(anyhow::anyhow!("Grok authorization timed out; retry login"));
        }
        if let Some(exit) = child.try_wait()? {
            break Ok(exit);
        }
        std::thread::sleep(Duration::from_millis(40));
    };
    #[cfg(unix)]
    {
        // The child owns a separate process group. Reap any surviving pipe holders.
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
    }
    child.terminate();
    // Drain the bounded pipe readers before reporting completion.
    let drain_until = Instant::now() + Duration::from_secs(2);
    while let Ok(progress) =
        receiver.recv_timeout(drain_until.saturating_duration_since(Instant::now()))
    {
        notify(progress);
    }
    let exit = result?;
    if !exit.success() {
        bail!(
            "Grok {} failed (exit {}). Retry or run the native command in a terminal for details",
            if action == Action::Logout {
                "logout"
            } else {
                "login"
            },
            exit.code()
                .map(|v| v.to_string())
                .unwrap_or_else(|| "interrupted".into())
        );
    }
    let status = status(home)?;
    if action != Action::Logout && !status.saved {
        bail!("Grok exited without a saved OAuth login; retry authorization");
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_status_handles_native_formats_expiry_and_redacts_tokens() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("auth.json");
        assert!(!status(temp.path()).unwrap().saved);
        fs::write(&path, r#"{"issuer/client":{"auth_mode":"oidc","key":"SECRET","refresh_token":"REFRESH","email":"user@example.com\n","expires_at":"2000-01-01T00:00:00Z"}}"#).unwrap();
        let saved = status(temp.path()).unwrap();
        assert!(saved.saved && saved.expired && saved.refreshable);
        assert_eq!(saved.email.as_deref(), Some("user@example.com"));
        assert!(!saved.description().contains("SECRET"));
        fs::write(&path, r#"{"auth_mode":"oauth","access_token":"SECRET"}"#).unwrap();
        assert!(status(temp.path()).unwrap().saved);
        fs::write(&path, r#"{"auth_mode":"api_key","key":"SECRET"}"#).unwrap();
        assert!(!status(temp.path()).unwrap().saved);
        fs::write(&path, "SECRET invalid JSON").unwrap();
        assert!(
            !status(temp.path())
                .unwrap_err()
                .to_string()
                .contains("SECRET")
        );
    }
    #[test]
    fn progress_exposes_only_authorization_links_and_device_codes() {
        let mut parser = Progress::default();
        for line in [
            "access_token=SECRET",
            "https://evil.example/device",
            "https://user:password@auth.x.ai/authorize",
            "https://auth.x.ai/authorize?access_token=SECRET",
            "https://auth.x.ai/authorize?code=SECRET",
        ] {
            assert_eq!(parser.parse(line), None);
        }
        assert_eq!(
            parser.parse("https://accounts.x.ai/device"),
            Some("Authorize: https://accounts.x.ai/device".into())
        );
        assert_eq!(parser.parse("Then enter this code:"), None);
        assert_eq!(
            parser.parse("ABCD-EFGH"),
            Some("Device code: ABCD-EFGH".into())
        );
        assert_eq!(parser.parse("ABCD-EFGH"), None);
    }
    #[cfg(unix)]
    fn script(home: &Path, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = home.join("mock-grok");
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }
    #[cfg(unix)]
    #[test]
    fn native_login_device_and_logout_own_credentials() {
        let home = tempfile::tempdir().unwrap();
        let program = script(
            home.path(),
            r#"
if [ "$1" = logout ]; then rm "$GROK_HOME/auth.json"; exit 0; fi
printf '%s\n' "$*" > "$GROK_HOME/args"
printf '%s\n' 'https://accounts.x.ai/device' 'Then enter this code:' 'ABCD-EFGH' 'access_token=SECRET' >&2
printf '%s' '{"issuer":{"auth_mode":"oidc","key":"SECRET"}}' > "$GROK_HOME/auth.json"
"#,
        );
        for (action, expected) in [
            (Action::Browser, "login --oauth"),
            (Action::Device, "login --device-auth"),
        ] {
            let mut progress = Vec::new();
            let saved = run_program(
                program.as_os_str(),
                action,
                home.path(),
                &AtomicBool::new(false),
                Duration::from_secs(5),
                |line| progress.push(line),
            )
            .unwrap();
            assert!(saved.saved);
            assert_eq!(
                fs::read_to_string(home.path().join("args")).unwrap().trim(),
                expected
            );
            assert!(progress.iter().any(|line| line.contains("ABCD-EFGH")));
            assert!(progress.iter().all(|line| !line.contains("SECRET")));
        }
        assert!(
            !run_program(
                program.as_os_str(),
                Action::Logout,
                home.path(),
                &AtomicBool::new(false),
                Duration::from_secs(5),
                |_| {}
            )
            .unwrap()
            .saved
        );
    }
    #[cfg(unix)]
    #[test]
    fn native_failure_cancellation_and_timeout_preserve_credentials() {
        let home = tempfile::tempdir().unwrap();
        let original = r#"{"auth_mode":"oidc","key":"SECRET"}"#;
        fs::write(home.path().join("auth.json"), original).unwrap();
        let program = script(home.path(), "echo SECRET >&2; exit 7");
        let error = run_program(
            program.as_os_str(),
            Action::Browser,
            home.path(),
            &AtomicBool::new(false),
            Duration::from_secs(2),
            |_| {},
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("exit 7") && !error.contains("SECRET"));
        let program = script(home.path(), "exec sleep 5");
        let cancelled = AtomicBool::new(true);
        assert!(
            run_program(
                program.as_os_str(),
                Action::Device,
                home.path(),
                &cancelled,
                Duration::from_secs(2),
                |_| {}
            )
            .unwrap_err()
            .to_string()
            .contains("cancelled")
        );
        assert!(
            run_program(
                program.as_os_str(),
                Action::Device,
                home.path(),
                &AtomicBool::new(false),
                Duration::from_millis(80),
                |_| {}
            )
            .unwrap_err()
            .to_string()
            .contains("timed out")
        );
        let cancel = AtomicBool::new(false);
        std::thread::scope(|scope| {
            let flag = &cancel;
            scope.spawn(move || {
                std::thread::sleep(Duration::from_millis(80));
                flag.store(true, Ordering::Relaxed);
            });
            assert!(
                run_program(
                    program.as_os_str(),
                    Action::Browser,
                    home.path(),
                    &cancel,
                    Duration::from_secs(2),
                    |_| {}
                )
                .unwrap_err()
                .to_string()
                .contains("cancelled")
            );
        });
        assert_eq!(
            fs::read_to_string(home.path().join("auth.json")).unwrap(),
            original
        );
    }
}
