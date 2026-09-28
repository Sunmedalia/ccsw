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
        "User-configured model managed by CCSW" => "User-configured model managed by Mux".into(),
        "Open CCSW Pulse usage monitor" => "Open Mux Pulse usage monitor".into(),
        "Toggle CCSW Pulse usage monitor" => "Toggle Mux Pulse usage monitor".into(),
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
    replace_external_json_with_backup(path, old, new, "mux-migration-backup")
}

fn replace_external_json_with_backup(
    path: &Path,
    old: &AppPaths,
    new: &AppPaths,
    suffix: &str,
) -> Result<()> {
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
        "{}.{suffix}",
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
    replace_external_toml_with_backup(path, old, new, "mux-migration-backup")
}

fn replace_external_toml_with_backup(
    path: &Path,
    old: &AppPaths,
    new: &AppPaths,
    suffix: &str,
) -> Result<()> {
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
    let backup = path.with_extension(format!("toml.{suffix}"));
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
        return repair_legacy_artifacts(new, &old);
    }
    from_paths(new, &old)
}

fn archive_stale_pi_binding(new: &AppPaths) -> Result<()> {
    let path = new.state_dir.join("pi-binding.json");
    if !path.exists() {
        return Ok(());
    }
    if fs::symlink_metadata(&path)?.file_type().is_symlink() {
        bail!("refusing symbolic link in Pi migration cleanup");
    }
    let before = fs::read(&path)?;
    let binding: Value = serde_json::from_slice(&before)?;
    let home = Path::new(binding["home"].as_str().context("Pi binding has no home")?);
    let models_path = home.join("models.json");
    let settings_path = home.join("settings.json");
    if !models_path.exists() || !settings_path.exists() {
        return Ok(());
    }
    let models_before = fs::read(&models_path)?;
    let settings_before = fs::read(&settings_path)?;
    let models: Value = serde_json::from_slice(&models_before)?;
    let settings: Value = serde_json::from_slice(&settings_before)?;
    let Some(keys) = binding["keys"].as_array() else {
        return Ok(());
    };
    let Some(expected_provider) = binding["expected"][1]["defaultProvider"].as_str() else {
        return Ok(());
    };
    let Some(current_provider) = settings["defaultProvider"].as_str() else {
        return Ok(());
    };
    // Native Pi editing replaced the old mirror workflow. A binding with none
    // of its providers present and no managed default is no longer an owner.
    if keys.is_empty()
        || keys.iter().any(|key| {
            key.as_str().is_none_or(|key| {
                !key.starts_with("mux-") || models["providers"].get(key).is_some()
            })
        })
        || current_provider == expected_provider
        || current_provider.starts_with("mux-")
    {
        return Ok(());
    }
    let backup = new.state_dir.join("migration-backups/pi-binding.json");
    write_new(&backup, &before)?;
    if fs::read(&path)? != before {
        bail!("Pi binding changed during migration cleanup");
    }
    if fs::read(&models_path)? != models_before || fs::read(&settings_path)? != settings_before {
        bail!("Pi native configuration changed during migration cleanup");
    }
    fs::remove_file(path)?;
    Ok(())
}

fn repair_legacy_artifacts(new: &AppPaths, old: &AppPaths) -> Result<()> {
    let marker = new.state_dir.join("mux-migration-branding-v2.json");
    if marker.exists() {
        return Ok(());
    }
    let lock_path = new.state_dir.join("pi.lock");
    crate::codex::private_dir(&new.state_dir)?;
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(lock_path)?;
    lock.try_lock_exclusive()
        .context("Pi operation is active; retry the migration cleanup")?;
    archive_stale_pi_binding(new)?;
    let catalogs = new.state_dir.join("codex-model-catalogs");
    if catalogs.exists() {
        for entry in fs::read_dir(catalogs)? {
            let path = entry?.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                replace_external_json_with_backup(&path, old, new, "mux-branding-v2-backup")?;
            }
        }
    }
    let herdr_config = crate::platform::override_path("HERDR_CONFIG_PATH", || {
        Ok(crate::platform::home()?.join(".config/herdr/config.toml"))
    })?;
    replace_external_toml_with_backup(&herdr_config, old, new, "mux-branding-v2-backup")?;
    let _ = std::process::Command::new("herdr")
        .args(["server", "reload-config"])
        .output();
    write_new(&marker, b"{\"version\":2}")?;
    Ok(())
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
    repair_legacy_artifacts(new, old)?;
    fs::remove_file(&pending)?;
    eprintln!(
        "Migrated CCSW data to Mux; the original directories and client file backups were retained."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn paths(root: &Path, name: &str) -> AppPaths {
        AppPaths {
            config: root.join("config").join(name).join("config.toml"),
            state_dir: root.join("state").join(name),
            cache: root.join("cache").join(name).join("models.json"),
        }
    }

    #[test]
    fn owned_identifiers_and_paths_are_translated_without_substring_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let old = paths(temp.path(), "ccsw");
        let new = paths(temp.path(), "mux");
        for (before, after) in [
            ("ccsw::provider::model", "mux::provider::model"),
            ("ccsw-role::sonnet", "mux-role::sonnet"),
            ("CCSW · provider", "Mux · provider"),
            ("Provider · CCSW Proxy", "Provider · Mux Proxy"),
            ("model_providers.ccsw", "model_providers.mux"),
            (
                "User-configured model managed by CCSW",
                "User-configured model managed by Mux",
            ),
            (
                "Open CCSW Pulse usage monitor",
                "Open Mux Pulse usage monitor",
            ),
            ("my-ccsw-provider", "my-ccsw-provider"),
        ] {
            assert_eq!(translate(before, &old, &new), after);
        }
        assert_eq!(
            translate(old.config.to_str().unwrap(), &old, &new),
            new.config.to_str().unwrap()
        );
        assert_eq!(
            translate(
                old.state_dir.join("accounts/auth.json").to_str().unwrap(),
                &old,
                &new
            ),
            new.state_dir.join("accounts/auth.json").to_str().unwrap()
        );
        let sibling = format!("{}-unrelated/file", old.state_dir.display());
        assert_eq!(translate(&sibling, &old, &new), sibling);
        assert_eq!(legacy_sibling(&new.config), Some(old.config));
    }

    #[test]
    fn json_migration_preserves_credentials_and_restore_snapshots() {
        let temp = tempfile::tempdir().unwrap();
        let old = paths(temp.path(), "ccsw");
        let new = paths(temp.path(), "mux");
        let mut document = json!({
            "provider": "ccsw", "model": "ccsw::one::model",
            "local_token": "ccsw-secret", "api_key": "ccsw-secret",
            "credential": {"value": "ccsw-secret"},
            "headers": {"Authorization": "ccsw-secret"},
            "before": {"provider": "ccsw"}, "original": {"provider": "ccsw"},
            "env": {"ANTHROPIC_AUTH_TOKEN": "ccsw-secret", "CUSTOM": "ccsw-secret", "ANTHROPIC_MODEL": "ccsw::one::model"},
            "encoded": "{\"model\":\"ccsw::one::model\",\"api_key\":\"ccsw-secret\"}",
            "ccswNativeBaseUrl": "https://example.test"
        });
        let before = document.clone();
        translate_json(&mut document, &old, &new).unwrap();
        for key in [
            "local_token",
            "api_key",
            "credential",
            "headers",
            "before",
            "original",
        ] {
            assert_eq!(document[key], before[key]);
        }
        assert_eq!(document["env"]["ANTHROPIC_AUTH_TOKEN"], "ccsw-secret");
        assert_eq!(document["env"]["CUSTOM"], "ccsw-secret");
        assert_eq!(document["env"]["ANTHROPIC_MODEL"], "mux::one::model");
        let encoded: Value = serde_json::from_str(document["encoded"].as_str().unwrap()).unwrap();
        assert_eq!(encoded["model"], "mux::one::model");
        assert_eq!(encoded["api_key"], "ccsw-secret");
        assert!(document.get("muxNativeBaseUrl").is_some());
        let once = document.clone();
        translate_json(&mut document, &old, &new).unwrap();
        assert_eq!(document, once);
    }

    #[test]
    fn mixed_old_and_new_keys_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let old = paths(temp.path(), "ccsw");
        let new = paths(temp.path(), "mux");
        let mut json = json!({"ccsw": 1, "mux": 2});
        assert!(translate_json(&mut json, &old, &new).is_err());
        let mut toml: toml::Value = "ccsw = 1\nmux = 2".parse().unwrap();
        assert!(translate_toml(&mut toml, &old, &new).is_err());
    }

    #[test]
    fn copy_preserves_auth_bytes_and_skips_runtime_files() {
        let temp = tempfile::tempdir().unwrap();
        let old = paths(temp.path(), "ccsw");
        let new = paths(temp.path(), "mux");
        fs::create_dir_all(old.state_dir.join("accounts/demo")).unwrap();
        let auth = b"{ \"token\": \"ccsw-secret\" }\n";
        fs::write(old.state_dir.join("accounts/demo/auth.json"), auth).unwrap();
        fs::write(
            old.state_dir.join("binding.json"),
            b"{\"model\":\"ccsw::one::model\"}",
        )
        .unwrap();
        for name in [
            "session.lock",
            "proxy.pid",
            "proxy.log",
            "usage.sqlite3",
            "usage.sqlite3-wal",
            "pi-transaction.json",
        ] {
            fs::write(old.state_dir.join(name), b"runtime").unwrap();
        }
        copy_tree(&old.state_dir, &new.state_dir, &old, &new).unwrap();
        assert_eq!(
            fs::read(new.state_dir.join("accounts/demo/auth.json")).unwrap(),
            auth
        );
        for name in [
            "session.lock",
            "proxy.pid",
            "proxy.log",
            "usage.sqlite3",
            "usage.sqlite3-wal",
            "pi-transaction.json",
        ] {
            assert!(!new.state_dir.join(name).exists());
        }
        let binding: Value =
            serde_json::from_slice(&fs::read(new.state_dir.join("binding.json")).unwrap()).unwrap();
        assert_eq!(binding["model"], "mux::one::model");
        assert!(old.state_dir.join("binding.json").exists());
    }

    #[test]
    fn usage_migration_preserves_rows_and_original_database() {
        let temp = tempfile::tempdir().unwrap();
        let old = paths(temp.path(), "ccsw");
        let new = paths(temp.path(), "mux");
        fs::create_dir_all(&old.state_dir).unwrap();
        fs::create_dir_all(&new.state_dir).unwrap();
        let source = rusqlite::Connection::open(old.state_dir.join(crate::usage::FILE)).unwrap();
        source.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE requests (config TEXT, model TEXT, tokens INTEGER);").unwrap();
        for (model, tokens) in [
            ("ccsw::one::model", 42),
            ("ccsw-role::sonnet", 21),
            ("native", 8),
        ] {
            source
                .execute(
                    "INSERT INTO requests VALUES (?1, ?2, ?3)",
                    rusqlite::params![old.config.to_string_lossy(), model, tokens],
                )
                .unwrap();
        }
        migrate_usage(&old, &new).unwrap();
        let target = rusqlite::Connection::open(new.state_dir.join(crate::usage::FILE)).unwrap();
        let count: i64 = target
            .query_row("SELECT count(*) FROM requests", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 3);
        let migrated: i64 = target.query_row("SELECT count(*) FROM requests WHERE config=?1 AND model IN ('mux::one::model', 'mux-role::sonnet')", [new.config.to_string_lossy().as_ref()], |r| r.get(0)).unwrap();
        assert_eq!(migrated, 2);
        let total: i64 = target
            .query_row("SELECT sum(tokens) FROM requests", [], |r| r.get(0))
            .unwrap();
        assert_eq!(total, 71);
        let original: i64 = source
            .query_row(
                "SELECT count(*) FROM requests WHERE config=?1",
                [old.config.to_string_lossy().as_ref()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(original, 3);
        migrate_usage(&old, &new).unwrap();
        assert_eq!(
            target
                .query_row("SELECT count(*) FROM requests", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            3
        );
    }

    #[test]
    fn branding_cleanup_has_separate_backups_and_preserves_comments() {
        let temp = tempfile::tempdir().unwrap();
        let old = paths(temp.path(), "ccsw");
        let new = paths(temp.path(), "mux");
        let path = temp.path().join("herdr.toml");
        let initial = "# keep this comment\n[[keys.command]]\ncommand = 'ccsw.open'\ndescription = 'Open CCSW Pulse usage monitor'\n";
        fs::write(&path, initial).unwrap();
        // Simulate the previous migration's preserved backup.
        fs::write(
            path.with_extension("toml.mux-migration-backup"),
            b"old untouched backup",
        )
        .unwrap();
        replace_external_toml_with_backup(&path, &old, &new, "mux-branding-v2-backup").unwrap();
        let after = fs::read_to_string(&path).unwrap();
        assert!(after.contains("# keep this comment"));
        assert!(after.contains("mux.open"));
        assert!(after.contains("Open Mux Pulse usage monitor"));
        assert_eq!(
            fs::read_to_string(path.with_extension("toml.mux-branding-v2-backup")).unwrap(),
            initial
        );
        assert_eq!(
            fs::read(path.with_extension("toml.mux-migration-backup")).unwrap(),
            b"old untouched backup"
        );
        replace_external_toml_with_backup(&path, &old, &new, "mux-branding-v2-backup").unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), after);
    }

    fn pi_fixture(root: &Path) -> (AppPaths, PathBuf, Vec<u8>, Vec<u8>) {
        let new = paths(root, "mux");
        let home = root.join("pi");
        fs::create_dir_all(&new.state_dir).unwrap();
        fs::create_dir_all(&home).unwrap();
        let models = b"{\"providers\": {\"native\": {}}}\n".to_vec();
        let settings = b"{\"defaultProvider\":\"native\",\"defaultModel\":\"model\"}\n".to_vec();
        fs::write(home.join("models.json"), &models).unwrap();
        fs::write(home.join("settings.json"), &settings).unwrap();
        let binding = json!({"home": home, "keys": ["mux-old"], "expected": [{"providers": {}}, {"defaultProvider": "mux-old"}]});
        fs::write(new.state_dir.join("pi-binding.json"), binding.to_string()).unwrap();
        (new, home, models, settings)
    }

    #[test]
    fn stale_pi_binding_is_archived_without_changing_native_settings() {
        let temp = tempfile::tempdir().unwrap();
        let (new, home, models, settings) = pi_fixture(temp.path());
        let binding_path = new.state_dir.join("pi-binding.json");
        let before = fs::read(&binding_path).unwrap();
        archive_stale_pi_binding(&new).unwrap();
        assert!(!binding_path.exists());
        assert_eq!(
            fs::read(new.state_dir.join("migration-backups/pi-binding.json")).unwrap(),
            before
        );
        assert_eq!(fs::read(home.join("models.json")).unwrap(), models);
        assert_eq!(fs::read(home.join("settings.json")).unwrap(), settings);
        archive_stale_pi_binding(&new).unwrap();
    }

    #[test]
    fn pi_binding_is_retained_when_it_still_owns_a_provider_or_default() {
        for retain_default in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let (new, home, _, _) = pi_fixture(temp.path());
            if retain_default {
                fs::write(
                    home.join("settings.json"),
                    b"{\"defaultProvider\":\"mux-old\"}",
                )
                .unwrap();
            } else {
                fs::write(
                    home.join("models.json"),
                    b"{\"providers\":{\"mux-old\":{}}}",
                )
                .unwrap();
            }
            let binding_path = new.state_dir.join("pi-binding.json");
            let before = fs::read(&binding_path).unwrap();
            archive_stale_pi_binding(&new).unwrap();
            assert_eq!(fs::read(&binding_path).unwrap(), before);
            assert!(
                !new.state_dir
                    .join("migration-backups/pi-binding.json")
                    .exists()
            );
        }
    }
}
