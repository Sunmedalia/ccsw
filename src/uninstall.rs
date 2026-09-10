//! Conservative, current-user-only removal. Never recursively deletes a directory.
use crate::{config::AppPaths, proxy};
use anyhow::{Context, Result, bail};
use fs2::FileExt;
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
    time::Duration,
};

const STATE_FILES: &[&str] = &[
    "proxy.json",
    "proxy.json.lock",
    "proxy.daemon.lock",
    "proxy.lifecycle.lock",
    "proxy.pid",
    "proxy.log",
    "sync-state.json",
    "sync-state.lock",
    "session.lock",
    "codex.lock",
    "codex-binding.json",
    "codex-transaction.json",
    "pi.lock",
    "pi-binding.json",
    "pi-transaction.json",
];

/// Reject links/reparse points and anything outside the chosen user's home.
/// Canonicalizing only HOME permits OS aliases such as macOS /var -> /private/var.
pub fn checked(path: &Path) -> Result<PathBuf> {
    let home_input = std::path::absolute(crate::platform::home()?)?;
    let home = fs::canonicalize(&home_input).context("cannot resolve user home")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = fs::metadata(&home)?;
        // SAFETY: geteuid has no preconditions.
        if meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o022 != 0 {
            bail!("unsafe user home ownership or permissions");
        }
    }
    if home.parent().is_none() {
        bail!("refusing filesystem root as user home");
    }
    let absolute = std::path::absolute(path)?;
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        bail!("parent traversal is not allowed: {}", path.display());
    }
    let relative = absolute
        .strip_prefix(&home_input)
        .or_else(|_| absolute.strip_prefix(&home))
        .with_context(|| {
            format!(
                "refusing path outside current user home: {}",
                path.display()
            )
        })?;
    if relative.as_os_str().is_empty() {
        bail!("refusing user home itself");
    }
    let mut result = home.clone();
    for component in relative.components() {
        result.push(component);
        match fs::symlink_metadata(&result) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    bail!("refusing symbolic link: {}", result.display());
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if meta.file_attributes() & 0x400 != 0 {
                        bail!("refusing reparse point: {}", result.display());
                    }
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    // SAFETY: geteuid has no preconditions.
                    if meta.uid() != unsafe { libc::geteuid() } {
                        bail!("refusing file owned by another user: {}", result.display());
                    }
                    if meta.mode() & 0o022 != 0 {
                        bail!("refusing group/world-writable path: {}", result.display());
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(result)
}

/// Held by CLI/TUI sessions so uninstall cannot race an active instance.
pub fn session(paths: &AppPaths) -> Result<File> {
    let path = paths.state_dir.join("session.lock");
    // Ordinary operations retain support for custom paths outside HOME. Uninstall
    // deliberately refuses such paths rather than widening its deletion boundary.
    fs::create_dir_all(&paths.state_dir)?;
    if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        bail!("session lock is a symbolic link");
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    FileExt::try_lock_shared(&file).context("CCSW uninstall is in progress")?;
    Ok(file)
}

struct Snapshot {
    path: PathBuf,
    bytes: Vec<u8>,
}
impl Snapshot {
    fn read(path: &Path) -> Result<Option<Self>> {
        let path = checked(path)?;
        if !path.exists() {
            return Ok(None);
        }
        let meta = fs::metadata(&path)?;
        if !meta.is_file() {
            bail!("expected a regular file: {}", path.display());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if meta.nlink() != 1 {
                bail!("refusing hard-linked file: {}", path.display());
            }
        }
        if meta.len() > 64 * 1024 * 1024 {
            bail!("file too large for safe uninstall: {}", path.display());
        }
        let bytes = fs::read(&path)?;
        Ok(Some(Self { path, bytes }))
    }
    fn verify(&self) -> Result<()> {
        let current = Self::read(&self.path)?.context("file disappeared during uninstall")?;
        if current.bytes != self.bytes {
            bail!("file changed during uninstall: {}", self.path.display());
        }
        Ok(())
    }
}
struct Settings {
    original: Snapshot,
    replacement: Option<Value>,
}
struct Plan {
    pi: Option<crate::pi::DetachPlan>,
    codex: Option<crate::codex::DetachPlan>,
    account_dirs: Vec<PathBuf>,
    files: Vec<Snapshot>,
    settings: Vec<Settings>,
    service: Option<Snapshot>,
    paths: AppPaths,
}

fn json(bytes: &[u8], label: &str) -> Result<Value> {
    serde_json::from_slice(bytes).with_context(|| format!("invalid {label}; no files removed"))
}
fn plan(paths: &AppPaths) -> Result<Plan> {
    let paths = AppPaths {
        config: checked(&paths.config)?,
        cache: checked(&paths.cache)?,
        state_dir: checked(&paths.state_dir)?,
    };
    let mut names = BTreeSet::from([
        paths.config.clone(),
        paths.config.with_extension("toml.lock"),
        paths.cache.clone(),
        paths.cache.with_extension("json.lock"),
    ]);
    for name in STATE_FILES {
        names.insert(paths.state_dir.join(name));
    }
    let mut account_dirs = Vec::new();
    if paths.config.exists() {
        for id in crate::config::load(&paths.config)?.codex.accounts.keys() {
            if id.len() != 32 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
                bail!("Invalid Codex account ID");
            }
            let dir = checked(&paths.state_dir.join("codex-accounts").join(id))?;
            for name in ["auth.json", "config.toml"] {
                names.insert(dir.join(name));
            }
            account_dirs.push(dir);
        }
    }
    if paths.state_dir.join("codex-binding.json").exists() {
        checked(&paths.state_dir.join("codex-binding.json"))?;
        let binding: Value =
            serde_json::from_slice(&fs::read(paths.state_dir.join("codex-binding.json"))?)?;
        let home = checked(Path::new(
            binding["home"]
                .as_str()
                .context("Invalid Codex home binding")?,
        ))?;
        checked(&home.join("config.toml"))?;
        checked(&home.join("auth.json"))?;
    }
    let codex = crate::codex::prepare_detach(&paths)?;
    let pi = crate::pi::prepare_detach(&paths)?;
    let mut files = Vec::new();
    for name in names {
        if let Some(file) = Snapshot::read(&name)? {
            files.push(file);
        }
    }
    if let Some(config) = files.iter().find(|f| f.path == paths.config) {
        let value: toml::Value = toml::from_str(std::str::from_utf8(&config.bytes)?)?;
        if value.get("version").is_none() || value.get("profiles").is_none() {
            bail!("not a CCSW configuration; refusing removal");
        }
    }
    let registry = files
        .iter()
        .find(|f| f.path == paths.state_dir.join("proxy.json"))
        .map(|f| json(&f.bytes, "proxy registry"))
        .transpose()?;
    if let Some(registry) = &registry {
        let address: std::net::SocketAddr = registry["listen"]
            .as_str()
            .context("missing proxy listen address")?
            .parse()?;
        if !address.ip().is_loopback()
            || address.port() == 0
            || registry["local_token"].as_str().is_none_or(str::is_empty)
        {
            bail!("invalid local proxy identity");
        }
        for route in registry["routes"]
            .as_object()
            .context("invalid proxy routes")?
            .values()
        {
            let config = route["config_path"]
                .as_str()
                .context("missing route config path")?;
            if checked(Path::new(config))? != paths.config {
                bail!("proxy state is shared with another configuration; refusing removal");
            }
        }
    }
    let state = files
        .iter()
        .find(|f| f.path == paths.state_dir.join("sync-state.json"))
        .map(|f| json(&f.bytes, "sync state"))
        .transpose()?;
    let mut settings_paths = BTreeSet::from([checked(&crate::claude_config::settings_path()?)?]);
    if let Some(state) = &state {
        for binding in state["entries"]
            .as_array()
            .context("invalid sync bindings")?
        {
            if checked(Path::new(
                binding["config"]
                    .as_str()
                    .context("invalid binding config")?,
            ))? != paths.config
            {
                bail!("sync state is shared with another configuration; refusing removal");
            }
            settings_paths.insert(checked(Path::new(
                binding["settings"]
                    .as_str()
                    .context("invalid binding settings")?,
            ))?);
        }
    }
    let mut settings = Vec::new();
    for path in settings_paths {
        let Some(original) = Snapshot::read(&path)? else {
            continue;
        };
        let mut value = json(&original.bytes, "Claude settings")?;
        if !value.is_object() || value.get("env").is_some_and(|e| !e.is_object()) {
            bail!("invalid Claude settings object");
        }
        let endpoint = value["env"]["ANTHROPIC_BASE_URL"].as_str();
        let token = value["env"]["ANTHROPIC_AUTH_TOKEN"].as_str();
        let bound = state
            .as_ref()
            .and_then(|s| s["entries"].as_array())
            .is_some_and(|entries| {
                entries.iter().any(|b| {
                    b["settings"]
                        .as_str()
                        .and_then(|s| checked(Path::new(s)).ok())
                        .as_ref()
                        == Some(&path)
                        && token.is_some()
                        && endpoint.is_some()
                        && b["token"].as_str() == token
                        && b["endpoint"].as_str() == endpoint
                })
            });
        let owned = bound || (registry.is_some() && proxy::owns_settings(&paths, &value)?);
        let replacement = if owned {
            let saved = state
                .as_ref()
                .and_then(|s| s["entries"].as_array())
                .and_then(|entries| {
                    entries.iter().find(|b| {
                        b["settings"]
                            .as_str()
                            .and_then(|s| checked(Path::new(s)).ok())
                            .as_ref()
                            == Some(&path)
                    })
                })
                .and_then(|b| b.get("managed"))
                .filter(|v| v.is_object());
            if saved.is_none() {
                println!(
                    "Preserve unverified legacy model fields: {}",
                    path.display()
                );
            } else if let Some(saved) = saved {
                let conflicts = crate::claude_config::managed_conflicts(saved, &value);
                if !conflicts.is_empty() {
                    println!(
                        "Preserve externally edited fields: {}",
                        conflicts.join(", ")
                    );
                }
            }
            crate::claude_config::remove_managed(&mut value, saved);
            Some(value)
        } else {
            None
        };
        // A backup is removed only when ownership of its associated settings is
        // established. Externally switched settings and their backup remain intact.
        if owned {
            for extra in [
                path.with_extension("json.ccsw-backup"),
                path.with_extension("json.ccsw.lock"),
            ] {
                if let Some(file) = Snapshot::read(&extra)? {
                    files.push(file);
                }
            }
        }
        settings.push(Settings {
            original,
            replacement,
        });
    }
    let service_path = proxy::service_status()?.path;
    let service = Snapshot::read(&service_path)?;
    if let Some(service) = &service {
        validate_service(service, &paths)?;
    }
    let mut unique = BTreeSet::new();
    for path in files
        .iter()
        .map(|f| &f.path)
        .chain(settings.iter().map(|s| &s.original.path))
        .chain(service.iter().map(|s| &s.path))
    {
        if !unique.insert(path) {
            bail!("overlapping uninstall paths: {}", path.display());
        }
    }
    Ok(Plan {
        pi,
        codex,
        account_dirs,
        files,
        settings,
        service,
        paths,
    })
}

fn validate_service(service: &Snapshot, paths: &AppPaths) -> Result<()> {
    let registry = paths.state_dir.join("proxy.json");
    #[cfg(windows)]
    {
        if checked(&crate::windows::startup_registry(&service.path)?)? != registry {
            bail!("startup belongs to another configuration");
        }
    }
    #[cfg(not(windows))]
    {
        let text = std::str::from_utf8(&service.bytes)?;
        #[cfg(target_os = "linux")]
        let owned = text.starts_with("[Unit]\nDescription=CCSW protocol proxy\n")
            && text.contains(&format!(
                " internal proxy-serve --registry {}\n",
                registry.display()
            ));
        #[cfg(target_os = "macos")]
        let owned = text.contains("<string>com.ccsw.proxy</string>")
            && text.contains(&format!(
                "<string>--registry</string><string>{}</string>",
                crate::proxy::xml_escape(&registry.to_string_lossy())
            ));
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        let owned = false;
        if !owned {
            bail!(
                "cannot verify ownership of startup file: {}",
                service.path.display()
            );
        }
    }
    Ok(())
}

pub fn run(paths: &AppPaths, execute: bool) -> Result<()> {
    let mut plan = plan(paths)?;
    // Release the session coordination pathname last, after all data removals.
    plan.files
        .sort_by_key(|f| f.path.file_name().is_some_and(|n| n == "session.lock"));
    for file in &plan.files {
        println!("Remove file: {}", file.path.display());
    }
    if let Some(pi) = &plan.pi {
        println!("Restore managed Pi settings: {}", pi.home.display());
    }
    if let Some(codex) = &plan.codex {
        println!(
            "Detach Codex settings and preserve external edits: {}",
            codex.home.display()
        );
    }
    for settings in &plan.settings {
        println!(
            "{} Claude settings: {}",
            if settings.replacement.is_some() {
                "Detach"
            } else {
                "Preserve"
            },
            settings.original.path.display()
        );
    }
    if let Some(service) = &plan.service {
        println!("Disable and remove startup: {}", service.path.display());
    }
    if !execute {
        println!(
            "Preview only. Run ccsw uninstall --yes to execute. The program binary is retained."
        );
        return Ok(());
    }
    // Acquire all application locks before stopping services or changing files.
    // Existing files only: an empty uninstall must not create configuration.
    let mut locks = Vec::new();
    let mut lock_paths = vec![
        plan.paths.state_dir.join("session.lock"),
        plan.paths.state_dir.join("sync-state.lock"),
        plan.paths.state_dir.join("codex.lock"),
        plan.paths.state_dir.join("pi.lock"),
        plan.paths.config.with_extension("toml.lock"),
        plan.paths.cache.with_extension("json.lock"),
        plan.paths.state_dir.join("proxy.lifecycle.lock"),
        plan.paths.state_dir.join("proxy.json.lock"),
    ];
    if let Some(pi) = &plan.pi {
        lock_paths.push(pi.home.join(".ccsw-pi.lock"));
    }
    if let Some(codex) = &plan.codex {
        lock_paths.push(codex.home.join(".ccsw.lock"));
    }
    lock_paths.extend(
        plan.settings
            .iter()
            .filter(|s| s.replacement.is_some())
            .map(|s| s.original.path.with_extension("json.ccsw.lock")),
    );
    for path in lock_paths {
        if path.exists() {
            checked(&path)?;
            let file = OpenOptions::new().read(true).write(true).open(&path)?;
            FileExt::try_lock_exclusive(&file).with_context(|| {
                format!("CCSW is busy; close other instances: {}", path.display())
            })?;
            locks.push(file);
        }
    }
    for file in &plan.files {
        file.verify()?;
    }
    for settings in &plan.settings {
        settings.original.verify()?;
    }
    if let Some(service) = &plan.service {
        service.verify()?;
        disable_service(service)?;
    }
    // Never signal a PID from disk: it might be stale or forged.
    if plan.paths.state_dir.join("proxy.json").exists() && proxy::status(&plan.paths)?.running {
        proxy::shutdown_authenticated(&plan.paths).context(
            "could not stop proxy; stop older CCSW proxies manually before uninstalling",
        )?;
    }
    let daemon_path = plan.paths.state_dir.join("proxy.daemon.lock");
    if daemon_path.exists() {
        let daemon = OpenOptions::new()
            .read(true)
            .write(true)
            .open(checked(&daemon_path)?)?;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if FileExt::try_lock_exclusive(&daemon).is_ok() {
                break;
            }
            if std::time::Instant::now() >= deadline {
                bail!("proxy is still active; configuration retained");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        locks.push(daemon);
    }
    for settings in &plan.settings {
        if let Some(value) = &settings.replacement {
            settings.original.verify()?;
            let mut temp =
                tempfile::NamedTempFile::new_in(settings.original.path.parent().unwrap())?;
            temp.write_all(&serde_json::to_vec_pretty(value)?)?;
            temp.as_file().sync_all()?;
            crate::config::set_private(temp.path())?;
            temp.persist(&settings.original.path).map_err(|e| e.error)?;
        }
    }
    if let Some(pi) = &plan.pi {
        crate::pi::execute_detach(pi)?;
    }
    if let Some(codex) = &plan.codex {
        crate::codex::execute_detach(codex)?;
    }
    // Keep operation locks through deletion. Partial I/O failures are reported;
    // remaining files can be safely processed on a later retry.
    for file in &plan.files {
        if !file.path.exists() && file.path.file_name().is_some_and(|n| n == "proxy.pid") {
            continue;
        }
        // Logs and PID may change during graceful shutdown; validate file type
        // and boundary again, but do not require their old contents.
        if file
            .path
            .file_name()
            .is_some_and(|n| n == "proxy.log" || n == "proxy.pid")
        {
            Snapshot::read(&file.path)?;
        } else {
            file.verify()?;
        }
        fs::remove_file(&file.path).with_context(|| {
            format!(
                "could not remove {}; uninstall incomplete",
                file.path.display()
            )
        })?;
    }
    if let Some(service) = &plan.service {
        service.verify()?;
        fs::remove_file(&service.path)?;
    }
    for dir in &plan.account_dirs {
        let _ = fs::remove_dir(checked(dir)?);
    }
    let account_root = plan.paths.state_dir.join("codex-accounts");
    if account_root.exists() {
        let _ = fs::remove_dir(checked(&account_root)?);
    }
    // Remove only explicitly known, empty app directories, never their parents.
    for dir in [
        plan.paths.config.parent().unwrap(),
        plan.paths.cache.parent().unwrap(),
        plan.paths.state_dir.as_path(),
    ] {
        if dir
            .file_name()
            .is_some_and(|n| n == "ccsw" || n == "state" || n == "cache")
            && dir.exists()
        {
            checked(dir)?;
            match fs::remove_dir(dir) {
                Ok(()) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::DirectoryNotEmpty | std::io::ErrorKind::NotFound
                    ) => {}
                Err(e) => return Err(e.into()),
            }
        }
    }
    drop(locks);
    println!("CCSW configuration removed. Unrelated files and the program binary were preserved.");
    Ok(())
}

fn disable_service(service: &Snapshot) -> Result<()> {
    #[cfg(target_os = "linux")]
    let output = std::process::Command::new("systemctl")
        .args(["--user", "disable", "--now", "ccsw-proxy.service"])
        .output()?;
    #[cfg(target_os = "macos")]
    let output = {
        // SAFETY: getuid has no preconditions.
        let domain = format!("gui/{}", unsafe { libc::getuid() });
        std::process::Command::new("launchctl")
            .arg("bootout")
            .arg(domain)
            .arg(&service.path)
            .output()?
    };
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if !output.status.success() {
        bail!(
            "could not disable startup; configuration retained: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let _ = service;
    Ok(())
}
