//! One-time adoption of installations made under the former project name.
//! The source directories are deliberately retained as a backup.
use crate::config::{self, AppPaths};
use anyhow::{Context, Result, bail};
use fs2::FileExt;
use serde_json::Value;
use std::{
    fs,
    fs::OpenOptions,
    path::{Path, PathBuf},
    time::Duration,
};

fn legacy_sibling(path: &Path) -> Option<PathBuf> {
    let mut result = PathBuf::new();
    let mut found = false;
    for component in path.components() {
        if component.as_os_str() == "mux" && !found {
            result.push("ccsw");
            found = true;
        } else {
            result.push(component.as_os_str());
        }
    }
    found.then_some(result)
}

fn translate(value: &str, old: &AppPaths, new: &AppPaths) -> String {
    for (before, after) in [
        (&old.config, &new.config),
        (&old.state_dir, &new.state_dir),
        (&old.cache, &new.cache),
    ] {
        if let (Some(before), Some(after)) = (before.to_str(), after.to_str())
            && (value == before
                || value.starts_with(&format!("{before}/"))
                || value.starts_with(&format!("{before}\\")))
        {
            return value.replacen(before, after, 1);
        }
    }
    if let Some(name) = value.strip_suffix(" · CCSW Proxy") {
        return format!("{name} · Mux Proxy");
    }
    if let Some(label) = value.strip_prefix("CCSW · ") {
        return format!("Mux · {label}");
    }
    for (before, after) in [
        ("ccsw-role::", "mux-role::"),
        ("ccsw::", "mux::"),
        ("ccsw-proxy-", "mux-proxy-"),
        ("ccsw-", "mux-"),
    ] {
        if let Some(rest) = value.strip_prefix(before) {
            return format!("{after}{rest}");
        }
    }
    match value {
        "ccsw" => "mux".into(),
        "CCSW" => "Mux".into(),
        "ccsw.open" => "mux.open".into(),
        "model_providers.ccsw" => "model_providers.mux".into(),
        _ => value.into(),
    }
}

fn translate_json(value: &mut Value, old: &AppPaths, new: &AppPaths) -> Result<()> {
    match value {
        Value::Object(map) => {
            let previous = std::mem::take(map);
            for (key, mut child) in previous {
                // Credentials and pre-management snapshots must stay byte-for-byte meaningful.
                if !matches!(
                    key.as_str(),
                    "local_token"
                        | "token"
                        | "apiKey"
                        | "api_key"
                        | "credential"
                        | "env"
                        | "headers"
                        | "extra_headers"
                        | "extraHeaders"
                        | "extra_headers_json"
                        | "original_auth"
                        | "original"
                        | "before"
                ) {
                    translate_json(&mut child, old, new)?;
                } else if key == "env"
                    && let Some(env) = child.as_object_mut()
                {
                    for (name, value) in env {
                        if name.starts_with("ANTHROPIC_") && name.contains("MODEL") {
                            translate_json(value, old, new)?;
                        }
                    }
                }
                let next_key = match key.as_str() {
                    "ccswNativeBaseUrl" => "muxNativeBaseUrl".into(),
                    "ccswNativeApi" => "muxNativeApi".into(),
                    "ccswModelsUrl" => "muxModelsUrl".into(),
                    "ccswProxyOf" => "muxProxyOf".into(),
                    _ => translate(&key, old, new),
                };
                if map.insert(next_key, child).is_some() {
                    bail!("old and new managed JSON identifiers conflict");
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                translate_json(item, old, new)?;
            }
        }
        Value::String(text) => {
            // Codex stores managed values as JSON encoded strings.
            if let Ok(mut nested) = serde_json::from_str::<Value>(text) {
                let before = nested.clone();
                translate_json(&mut nested, old, new)?;
                if nested != before {
                    *text = nested.to_string();
                }
            } else {
                *text = translate(text, old, new);
            }
        }
        _ => {}
    }
    Ok(())
}

fn translated_json(bytes: &[u8], old: &AppPaths, new: &AppPaths) -> Result<Vec<u8>> {
    let mut document: Value = serde_json::from_slice(bytes)?;
    translate_json(&mut document, old, new)?;
    Ok(serde_json::to_vec_pretty(&document)?)
}

fn translate_toml(value: &mut toml::Value, old: &AppPaths, new: &AppPaths) -> Result<()> {
    match value {
        toml::Value::Table(table) => {
            let previous = std::mem::take(table);
            for (key, mut child) in previous {
                if !matches!(
                    key.as_str(),
                    "credential"
                        | "env"
                        | "extra_headers_json"
                        | "extra_headers"
                        | "headers"
                        | "http_headers"
                        | "env_http_headers"
                        | "api_key"
                ) {
                    translate_toml(&mut child, old, new)?;
                }
                if table.insert(translate(&key, old, new), child).is_some() {
                    bail!("old and new managed TOML identifiers conflict");
                }
            }
        }
        toml::Value::Array(items) => {
            for item in items {
                translate_toml(item, old, new)?;
            }
        }
        toml::Value::String(text) => {
            if let Ok(mut nested) = serde_json::from_str::<Value>(text) {
                let before = nested.clone();
                translate_json(&mut nested, old, new)?;
                if nested != before {
                    *text = nested.to_string();
                }
            } else {
                *text = translate(text, old, new);
            }
        }
        _ => {}
    }
    Ok(())
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    let parent = path.parent().context("migration target has no parent")?;
    crate::codex::private_dir(parent)?;
    if fs::symlink_metadata(path).is_ok() {
        bail!("migration target is not a regular file: {}", path.display());
    }
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    use std::io::Write;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    config::set_private(temp.path())?;
    temp.persist_noclobber(path).map_err(|error| error.error)?;
    Ok(())
}

fn copy_tree(source: &Path, target: &Path, old: &AppPaths, new: &AppPaths) -> Result<()> {
    if !source.exists() {
        return Ok(());
    }
    if fs::symlink_metadata(source)?.file_type().is_symlink() {
        bail!(
            "refusing symbolic link in legacy data: {}",
            source.display()
        );
    }
    crate::codex::private_dir(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let from = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.ends_with(".lock")
            || name == "proxy.pid"
            || name == "proxy.log"
            || name.starts_with("usage.sqlite3")
            || name.ends_with("-transaction.json")
        {
            continue;
        }
        let to = target.join(name.as_ref());
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            bail!("refusing symbolic link in legacy data: {}", from.display());
        }
        if kind.is_dir() {
            copy_tree(&from, &to, old, new)?;
        } else if kind.is_file() {
            let bytes = fs::read(&from)?;
            let bytes = if name.ends_with(".json") && name != "auth.json" {
                translated_json(&bytes, old, new)?
            } else {
                bytes
            };
            write_new(&to, &bytes)?;
        } else {
            bail!("unsupported legacy data file: {}", from.display());
        }
    }
    Ok(())
}

fn replace_external_json(path: &Path, old: &AppPaths, new: &AppPaths) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        bail!("refusing symbolic link: {}", path.display());
    }
    let before = fs::read(path)?;
    let mut value: Value = serde_json::from_slice(&before)?;
    let original = value.clone();
    translate_json(&mut value, old, new)?;
    if value == original {
        return Ok(());
    }
    let after = serde_json::to_vec_pretty(&value)?;
    let backup = path.with_extension(format!(
        "{}.mux-migration-backup",
        path.extension().and_then(|s| s.to_str()).unwrap_or("file")
    ));
    write_new(&backup, &before)?;
    let mut temp = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    use std::io::Write;
    temp.write_all(&after)?;
    temp.as_file()
        .set_permissions(fs::metadata(path)?.permissions())?;
    temp.as_file().sync_all()?;
    if fs::read(path)? != before {
        bail!(
            "client configuration changed during migration: {}",
            path.display()
        );
    }
    temp.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn replace_external_toml(path: &Path, old: &AppPaths, new: &AppPaths) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        bail!("refusing symbolic link: {}", path.display());
    }
    let before = fs::read_to_string(path)?;
    let mut doc: toml_edit::DocumentMut = before.parse()?;
    fn rewrite(item: &mut toml_edit::Item, old: &AppPaths, new: &AppPaths) -> Result<()> {
        if let Some(table) = item.as_table_like_mut() {
            let keys: Vec<String> = table.iter().map(|(name, _)| name.to_owned()).collect();
            for key in keys {
                let translated = translate(&key, old, new);
                if translated != key && table.contains_key(&translated) {
                    bail!("old and new managed client identifiers conflict");
                }
                if translated != key
                    && !table.contains_key(&translated)
                    && let Some(value) = table.remove(&key)
                {
                    table.insert(&translated, value);
                }
                if !matches!(
                    translated.as_str(),
                    "env"
                        | "api_key"
                        | "http_headers"
                        | "env_http_headers"
                        | "credential"
                        | "extra_headers"
                        | "headers"
                ) && let Some(child) = table.get_mut(&translated)
                {
                    rewrite(child, old, new)?;
                }
            }
        } else if let Some(tables) = item.as_array_of_tables_mut() {
            for table in tables.iter_mut() {
                for (_, child) in table.iter_mut() {
                    rewrite(child, old, new)?;
                }
            }
        } else if let Some(value) = item.as_value_mut()
            && let Some(text) = value.as_str()
        {
            let changed = translate(text, old, new);
            if changed != text {
                *value = toml_edit::Value::from(changed);
            }
        }
        Ok(())
    }
    for (name, item) in doc.iter_mut() {
        if !matches!(
            name.get(),
            "env" | "api_key" | "http_headers" | "env_http_headers" | "extra_headers" | "headers"
        ) {
            rewrite(item, old, new)?;
        }
    }
    let after = doc.to_string();
    if after == before {
        return Ok(());
    }
    let backup = path.with_extension("toml.mux-migration-backup");
    write_new(&backup, before.as_bytes())?;
    let mut temp = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    use std::io::Write;
    temp.write_all(after.as_bytes())?;
    temp.as_file()
        .set_permissions(fs::metadata(path)?.permissions())?;
    temp.as_file().sync_all()?;
    if fs::read_to_string(path)? != before {
        bail!(
            "client configuration changed during migration: {}",
            path.display()
        );
    }
    temp.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn stop_legacy_proxy(old: &AppPaths) -> Result<bool> {
    let registry = old.state_dir.join("proxy.json");
    if !registry.exists() {
        return Ok(false);
    }
    let value: Value = serde_json::from_slice(&fs::read(&registry)?)?;
    let listen = value["listen"]
        .as_str()
        .context("legacy proxy has no listen address")?;
    let address: std::net::SocketAddr = listen.parse()?;
    if !address.ip().is_loopback() {
        bail!("legacy proxy address is not loopback");
    }
    let token = value["local_token"]
        .as_str()
        .context("legacy proxy has no local token")?;
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()?;
    let health = client
        .get(format!("http://{listen}/health"))
        .bearer_auth(token)
        .send();
    let mut running = false;
    if let Ok(response) = health
        && response.status().is_success()
    {
        let body: Value = response.json()?;
        if body["name"] != "ccsw-proxy" {
            bail!("another process owns the legacy proxy port");
        }
        running = true;
        client
            .post(format!("http://{listen}/internal/shutdown"))
            .bearer_auth(token)
            .send()?
            .error_for_status()?;
    }
    let daemon_path = old.state_dir.join("proxy.daemon.lock");
    if daemon_path.exists() {
        let daemon = OpenOptions::new()
            .read(true)
            .write(true)
            .open(daemon_path)?;
        for _ in 0..50 {
            if daemon.try_lock_exclusive().is_ok() {
                return Ok(running);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        bail!("the legacy proxy did not stop; old data was retained");
    }
    Ok(running)
}

fn migrate_usage(old: &AppPaths, new: &AppPaths) -> Result<()> {
    let source = old.state_dir.join(crate::usage::FILE);
    let target = new.state_dir.join(crate::usage::FILE);
    if !source.exists() || target.exists() {
        return Ok(());
    }
    let source_db =
        rusqlite::Connection::open_with_flags(&source, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let staged = tempfile::NamedTempFile::new_in(&new.state_dir)?;
    let staged_path = staged.path().to_path_buf();
    // VACUUM INTO includes committed WAL records without copying live sidecars.
    source_db.execute("VACUUM INTO ?1", [staged_path.to_string_lossy().as_ref()])?;
    let mut db = rusqlite::Connection::open(&staged_path)?;
    let transaction = db.transaction()?;
    transaction.execute(
        "UPDATE requests SET config=?1 WHERE config=?2",
        rusqlite::params![new.config.to_string_lossy(), old.config.to_string_lossy()],
    )?;
    transaction.execute("UPDATE requests SET model='mux' || substr(model,5) WHERE model LIKE 'ccsw::%' OR model LIKE 'ccsw-role::%'", [])?;
    transaction.commit()?;
    db.close().map_err(|(_, error)| error)?;
    config::set_private(&staged_path)?;
    staged
        .persist_noclobber(&target)
        .map_err(|error| error.error)?;
    Ok(())
}

fn legacy_service_exists() -> Result<bool> {
    #[cfg(target_os = "macos")]
    return Ok(crate::platform::home()?
        .join("Library/LaunchAgents/com.ccsw.proxy.plist")
        .exists());
    #[cfg(target_os = "linux")]
    return Ok(crate::platform::home()?
        .join(".config/systemd/user/ccsw-proxy.service")
        .exists());
    #[cfg(windows)]
    return Ok(crate::platform::appdata(false)?
        .join("Microsoft/Windows/Start Menu/Programs/Startup/CCSW Proxy.lnk")
        .exists());
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    Ok(false)
}

fn disable_legacy_service(old: &AppPaths) -> Result<bool> {
    #[cfg(target_os = "macos")]
    let path = crate::platform::home()?.join("Library/LaunchAgents/com.ccsw.proxy.plist");
    #[cfg(target_os = "linux")]
    let path = crate::platform::home()?.join(".config/systemd/user/ccsw-proxy.service");
    #[cfg(windows)]
    let path = crate::platform::appdata(false)?
        .join("Microsoft/Windows/Start Menu/Programs/Startup/CCSW Proxy.lnk");
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    return Ok(false);
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    {
        if !path.exists() {
            return Ok(false);
        }
        if fs::symlink_metadata(&path)?.file_type().is_symlink() {
            bail!("legacy startup entry is a symbolic link");
        }
        let bytes = fs::read(&path)?;
        #[cfg(not(windows))]
        {
            let text = std::str::from_utf8(&bytes)?;
            let registry = old.state_dir.join("proxy.json");
            #[cfg(target_os = "macos")]
            let owns = text.contains("<string>com.ccsw.proxy</string>")
                && text.contains(&crate::proxy::xml_escape(&registry.to_string_lossy()));
            #[cfg(target_os = "linux")]
            let owns = text.starts_with("[Unit]\nDescription=CCSW protocol proxy\n")
                && text.contains(&format!(
                    " internal proxy-serve --registry {}\n",
                    registry.display()
                ));
            if !owns {
                bail!("legacy startup entry belongs to another configuration");
            }
        }
        #[cfg(windows)]
        if !crate::platform::same_path(
            &crate::windows::startup_registry(&path)?,
            &old.state_dir.join("proxy.json"),
        )? {
            bail!("legacy startup entry belongs to another configuration");
        }
        write_new(&path.with_extension("mux-migration-backup"), &bytes)?;
        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("launchctl")
                .args([
                    "bootout",
                    &format!("gui/{}", unsafe { libc::getuid() }),
                    path.to_string_lossy().as_ref(),
                ])
                .output()?;
        }
        #[cfg(target_os = "linux")]
        {
            let output = std::process::Command::new("systemctl")
                .args(["--user", "disable", "--now", "ccsw-proxy.service"])
                .output()?;
            if !output.status.success() {
                bail!("could not disable the legacy proxy service");
            }
        }
        fs::remove_file(path)?;
        Ok(true)
    }
}

fn migrate_herdr(old: &AppPaths, new: &AppPaths) -> Result<()> {
    let output = match std::process::Command::new("herdr")
        .args(["plugin", "list", "--plugin", "ccsw", "--json"])
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => return Ok(()),
    };
    let list: Value = serde_json::from_slice(&output.stdout)?;
    let plugins = list["result"]["plugins"]
        .as_array()
        .context("invalid Herdr plugin list")?;
    let config_path = crate::platform::override_path("HERDR_CONFIG_PATH", || {
        Ok(crate::platform::home()?.join(".config/herdr/config.toml"))
    })?;
    let has_shortcut =
        config_path.exists() && fs::read_to_string(&config_path)?.contains("ccsw.open");
    if plugins.is_empty() && !has_shortcut {
        return Ok(());
    }
    if plugins.len() > 1 {
        bail!("multiple legacy Herdr plugins were found; retained their configuration");
    }
    let data_root = crate::platform::override_path("XDG_DATA_HOME", || {
        Ok(crate::platform::home()?.join(".local/share"))
    })?;
    let root = data_root.join("mux/herdr/migrated");
    let mut manifest: toml::Value = include_str!("../herdr-plugin.toml").parse()?;
    manifest.as_table_mut().unwrap().remove("build");
    write_new(
        &root.join("herdr-plugin.toml"),
        toml::to_string_pretty(&manifest)?.as_bytes(),
    )?;
    let binary = root.join("target/release/mux");
    write_new(&binary, &fs::read(std::env::current_exe()?)?)?;
    fs::set_permissions(
        &binary,
        fs::metadata(std::env::current_exe()?)?.permissions(),
    )?;
    let link = std::process::Command::new("herdr")
        .args(["plugin", "link"])
        .arg(&root)
        .output()?;
    if !link.status.success() {
        bail!("could not register the Mux Herdr plugin");
    }
    let enable = std::process::Command::new("herdr")
        .args(["plugin", "enable", "mux"])
        .output()?;
    if !enable.status.success() {
        bail!("could not enable the Mux Herdr plugin");
    }
    replace_external_toml(&config_path, old, new)?;
    if !plugins.is_empty() {
        let disable = std::process::Command::new("herdr")
            .args(["plugin", "disable", "ccsw"])
            .output()?;
        if !disable.status.success() {
            bail!("could not disable the old Herdr plugin");
        }
        // Retain its files as a backup; the old entry is disabled and no longer bound.
    }
    let _ = std::process::Command::new("herdr")
        .args(["server", "reload-config"])
        .output();
    Ok(())
}

fn bound_home(old: &AppPaths, client: &str, fallback: PathBuf) -> Result<PathBuf> {
    let path = old.state_dir.join(format!("{client}-binding.json"));
    if !path.exists() {
        return Ok(fallback);
    }
    let binding: Value = serde_json::from_slice(&fs::read(path)?)?;
    Ok(PathBuf::from(
        binding["home"]
            .as_str()
            .context("legacy client binding has no home")?,
    ))
}

pub fn run(new: &AppPaths) -> Result<()> {
    let pending = new.state_dir.join("mux-migration-pending.json");
    if pending.exists() {
        let value: Value = serde_json::from_slice(&fs::read(&pending)?)?;
        let old = AppPaths {
            config: PathBuf::from(
                value["config"]
                    .as_str()
                    .context("migration journal has no config path")?,
            ),
            state_dir: PathBuf::from(
                value["state_dir"]
                    .as_str()
                    .context("migration journal has no state path")?,
            ),
            cache: PathBuf::from(
                value["cache"]
                    .as_str()
                    .context("migration journal has no cache path")?,
            ),
        };
        return from_paths(new, &old);
    }
    let (Some(config), Some(state_dir), Some(cache)) = (
        legacy_sibling(&new.config),
        legacy_sibling(&new.state_dir.join("marker")).map(|p| p.parent().unwrap().to_path_buf()),
        legacy_sibling(&new.cache),
    ) else {
        return Ok(());
    };
    if !config.exists() {
        return Ok(());
    }
    let old = AppPaths {
        config,
        state_dir,
        cache,
    };
    if new.config.exists() && !new.state_dir.join("mux-migration-pending.json").exists() {
        return Ok(());
    }
    from_paths(new, &old)
}

pub fn from_paths(new: &AppPaths, old: &AppPaths) -> Result<()> {
    let pending = new.state_dir.join("mux-migration-pending.json");
    if crate::platform::same_path(&old.config, &new.config)?
        || crate::platform::same_path(&old.state_dir, &new.state_dir)?
        || crate::platform::same_path(&old.cache, &new.cache)?
    {
        bail!("old and new data paths must be different");
    }
    if new.state_dir.starts_with(&old.state_dir) || old.state_dir.starts_with(&new.state_dir) {
        bail!("old and new state directories must not contain each other");
    }
    crate::codex::private_dir(new.config.parent().unwrap())?;
    let lock_path = new.config.with_extension("toml.lock");
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)?;
    lock.lock_exclusive()?;
    if new.config.exists() && !pending.exists() {
        bail!("Mux already has a configuration; migration will not overwrite it");
    }
    let legacy_session = old.state_dir.join("session.lock");
    let _legacy_lock = if legacy_session.exists() {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&legacy_session)?;
        file.try_lock_exclusive()
            .context("close all CCSW windows before Mux migrates the old data")?;
        Some(file)
    } else {
        None
    };
    for journal in [
        "codex-transaction.json",
        "pi-transaction.json",
        "grok-transaction.json",
    ] {
        if old.state_dir.join(journal).exists() {
            bail!("recover the interrupted CCSW transaction before starting Mux");
        }
    }
    let pi_home = bound_home(old, "pi", crate::pi::home()?)?;
    if pi_home.join(".ccsw-native-transaction.json").exists() {
        bail!("recover the interrupted legacy Pi native transaction before starting Mux");
    }
    let mut old_config: toml::Value = fs::read_to_string(&old.config)?.parse()?;
    let _ = config::load(&old.config)?;
    let installed_service = legacy_service_exists()?;
    let pending_info = if pending.exists() {
        serde_json::from_slice::<Value>(&fs::read(&pending)?)?
    } else {
        serde_json::json!({"service": installed_service, "config": old.config, "state_dir": old.state_dir, "cache": old.cache})
    };
    write_new(&pending, &serde_json::to_vec(&pending_info)?)?;
    disable_legacy_service(old)?;
    let was_running = stop_legacy_proxy(old)?;
    copy_tree(&old.state_dir, &new.state_dir, old, new)?;
    migrate_usage(old, new)?;
    if let Some(parent) = old.cache.parent() {
        copy_tree(parent, new.cache.parent().unwrap(), old, new)?;
    }
    translate_toml(&mut old_config, old, new)?;
    let translated = toml::to_string_pretty(&old_config)?;
    // Existing client files are edited only after their originals are backed up.
    let mut claude_settings = vec![crate::claude_config::settings_path()?];
    let sync_state = old.state_dir.join("sync-state.json");
    if sync_state.exists() {
        let state: Value = serde_json::from_slice(&fs::read(sync_state)?)?;
        if let Some(entries) = state["entries"].as_array() {
            for entry in entries {
                if entry["config"].as_str() == old.config.to_str()
                    && let Some(settings) = entry["settings"].as_str()
                {
                    claude_settings.push(PathBuf::from(settings));
                }
            }
        }
    }
    claude_settings.sort();
    claude_settings.dedup();
    for settings in claude_settings {
        if settings
            .with_extension("json.ccsw-preferences-journal")
            .exists()
        {
            bail!(
                "recover the interrupted legacy Claude preferences transaction before starting Mux"
            );
        }
        replace_external_json(&settings, old, new)?;
        let backup = settings.with_extension("json.ccsw-backup");
        if backup.exists() {
            write_new(
                &settings.with_extension("json.mux-backup"),
                &fs::read(backup)?,
            )?;
        }
    }
    let codex_home = bound_home(old, "codex", crate::codex::home()?)?;
    replace_external_toml(&codex_home.join("config.toml"), old, new)?;
    for name in ["models.json", "settings.json"] {
        replace_external_json(&pi_home.join(name), old, new)?;
    }
    let grok_home = bound_home(old, "grok", crate::grok::home()?)?;
    replace_external_toml(&grok_home.join("config.toml"), old, new)?;
    migrate_herdr(old, new)?;
    // Publish the configuration last, so a partial migration can be retried.
    write_new(&new.config, translated.as_bytes())?;
    let _ = config::load(&new.config)?;
    if pending_info["service"] == true {
        crate::proxy::install(new)?;
    } else if was_running {
        crate::proxy::start(new, None)?;
    }
    fs::remove_file(&pending)?;
    eprintln!(
        "Migrated CCSW data to Mux; the original directories and client file backups were retained."
    );
    Ok(())
}
