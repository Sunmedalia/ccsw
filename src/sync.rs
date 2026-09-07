//! One synchronization path for the CLI and TUI. State is bound to both files.
use crate::{
    claude_config::{self, ApplyResult},
    config::{self, AppPaths, Config},
    discovery, proxy,
};
use anyhow::{Context, Result, bail};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::hash_map::DefaultHasher,
    fs::{self, OpenOptions},
    hash::{Hash, Hasher},
    io::Write,
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;

#[derive(Default, Serialize, Deserialize)]
struct State {
    #[serde(default)]
    entries: Vec<Binding>,
}
#[derive(Serialize, Deserialize)]
struct Binding {
    config: PathBuf,
    settings: PathBuf,
    preferred: Option<String>,
    revision: u64,
    endpoint: Option<String>,
    token: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    NotConnected,
    Pending,
    Syncing,
    Synced,
    Failed,
    Paused,
}
impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::NotConnected => "Not connected",
            Self::Pending => "Pending",
            Self::Syncing => "Syncing",
            Self::Synced => "Synced",
            Self::Failed => "Failed",
            Self::Paused => "Paused · press p to reconnect",
        }
    }
}

fn identity(path: &Path) -> Result<PathBuf> {
    if let Ok(path) = fs::canonicalize(path) {
        return Ok(path);
    }
    let absolute = std::path::absolute(path)?;
    if let (Some(parent), Some(name)) = (absolute.parent(), absolute.file_name()) {
        return Ok(identity(parent)?.join(name));
    }
    Ok(absolute)
}
fn read_settings(path: &Path) -> Result<Value> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let value: Value =
        serde_json::from_slice(&fs::read(path)?).context("Claude settings contain invalid JSON")?;
    if !value.is_object() || value.get("env").is_some_and(|env| !env.is_object()) {
        bail!("Claude settings and env must be JSON objects");
    }
    Ok(value)
}
fn load(paths: &AppPaths) -> Result<State> {
    let path = paths.state_dir.join("sync-state.json");
    if !path.exists() {
        return Ok(State::default());
    }
    serde_json::from_slice(&fs::read(path)?).context("could not read sync state")
}
fn save(paths: &AppPaths, state: &State) -> Result<()> {
    let mut temp = NamedTempFile::new_in(&paths.state_dir)?;
    temp.write_all(&serde_json::to_vec_pretty(state)?)?;
    temp.as_file().sync_all()?;
    config::set_private(temp.path())?;
    temp.persist(paths.state_dir.join("sync-state.json"))
        .map_err(|error| error.error)?;
    Ok(())
}
// Change detection only, never used for authentication or integrity verification.
fn revision(config: &Config) -> Result<u64> {
    let mut hash = DefaultHasher::new();
    serde_json::to_vec(config)?.hash(&mut hash);
    Ok(hash.finish())
}
fn matches(binding: &Binding, value: &Value) -> bool {
    binding.endpoint.as_deref() == value["env"]["ANTHROPIC_BASE_URL"].as_str()
        && binding.token.as_deref() == value["env"]["ANTHROPIC_AUTH_TOKEN"].as_str()
        && (binding.endpoint.is_some()
            || (value.get("modelPicker").is_none() && value.get("model").is_none()))
}
pub fn inspect(paths: &AppPaths, settings: &Path) -> Result<Status> {
    let config_path = identity(&paths.config)?;
    let settings_path = identity(settings)?;
    let state = load(paths)?;
    let value = read_settings(settings)?;
    if let Some(binding) = state
        .entries
        .iter()
        .find(|entry| entry.config == config_path && entry.settings == settings_path)
    {
        if !matches(binding, &value) {
            return Ok(Status::Paused);
        }
        return Ok(
            if binding.revision == revision(&config::load(&paths.config)?)? {
                Status::Synced
            } else {
                Status::Pending
            },
        );
    }
    Ok(if proxy::owns_settings(paths, &value)? {
        Status::Pending
    } else {
        Status::NotConnected
    })
}

/// Explicit calls establish ownership; automatic calls require existing ownership.
pub fn apply(
    paths: &AppPaths,
    settings: &Path,
    preferred: Option<&str>,
    explicit: bool,
) -> Result<ApplyResult> {
    fs::create_dir_all(&paths.state_dir)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(paths.state_dir.join("sync-state.lock"))?;
    lock.lock_exclusive()?;
    let config_path = identity(&paths.config)?;
    let settings_path = identity(settings)?;
    let mut state = load(paths)?;
    let index = state
        .entries
        .iter()
        .position(|entry| entry.config == config_path && entry.settings == settings_path);
    let value = read_settings(settings)?;
    if !explicit {
        let owned = if let Some(index) = index {
            matches(&state.entries[index], &value)
        } else {
            proxy::owns_settings(paths, &value)?
        };
        if !owned {
            bail!("Claude settings are not connected to this configuration; press p to reconnect");
        }
    }
    let config = config::load(&paths.config)?;
    let cache = discovery::load_cache(&paths.cache);
    let legacy_preferred = if index.is_none() && proxy::owns_settings(paths, &value)? {
        value["model"]
            .as_str()
            .and_then(|model| model.split_once("::"))
            .map(|(id, _)| id)
    } else {
        None
    };
    let previous = index
        .and_then(|index| state.entries[index].preferred.as_deref())
        .or(legacy_preferred);
    let requested = preferred.or(previous);
    if explicit && let Some(id) = preferred {
        let profile = config
            .profiles
            .get(id)
            .with_context(|| format!("profile '{id}' does not exist"))?;
        if !profile.enabled {
            bail!("profile '{id}' is disabled");
        }
    }
    let has_models = |id: &str| {
        config
            .profiles
            .get(id)
            .is_some_and(|profile| !discovery::active_models(profile, &[]).is_empty())
    };
    let chosen = requested
        .filter(|id| has_models(id))
        .map(str::to_owned)
        .or_else(|| config.profiles.keys().find(|id| has_models(id)).cloned());
    if chosen.is_none() && index.is_none() && !proxy::owns_settings(paths, &value)? {
        bail!("enable at least one model before connecting to Claude");
    }
    let mut effective = config.clone();
    if let Some(id) = &chosen {
        let profile = effective
            .profiles
            .get_mut(id)
            .expect("chosen provider exists");
        let active = discovery::active_models(profile, &[]);
        if !active.iter().any(|model| {
            config::canonical_model_id(&model.id)
                == config::canonical_model_id(&profile.default_model)
        }) {
            profile.default_model = active[0].id.clone();
        }
    }
    let checkpoint = proxy::aggregate_checkpoint(paths)?;
    let result = if let Some(id) = &chosen {
        claude_config::apply_all(settings, paths, &effective, &cache, id)
    } else {
        proxy::clear_aggregate_models(paths).and_then(|()| claude_config::clear(settings))
    };
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            if let Err(rollback) = proxy::restore_aggregate(paths, checkpoint) {
                bail!(
                    "sync failed: {error:#}; restoring proxy routes also failed: {rollback:#}; press p to retry"
                );
            }
            return Err(error).context("local changes saved; Claude sync failed");
        }
    };
    let applied = read_settings(settings)?;
    let binding = Binding {
        config: config_path,
        settings: settings_path,
        preferred: preferred
            .map(str::to_owned)
            .or_else(|| previous.map(str::to_owned))
            .or(chosen),
        revision: revision(&config)?,
        endpoint: applied["env"]["ANTHROPIC_BASE_URL"]
            .as_str()
            .map(str::to_owned),
        token: applied["env"]["ANTHROPIC_AUTH_TOKEN"]
            .as_str()
            .map(str::to_owned),
    };
    if let Some(index) = index {
        state.entries[index] = binding;
    } else {
        state.entries.push(binding);
    }
    save(paths, &state)
        .context("Claude updated, but connection state could not be saved; press p to retry")?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread::{self, JoinHandle},
        time::Duration,
    };

    struct Fixture {
        _temp: tempfile::TempDir,
        paths: AppPaths,
        settings: PathBuf,
        stopped: Arc<AtomicBool>,
        server: Option<JoinHandle<()>>,
    }
    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let paths = AppPaths {
                config: temp.path().join("config.toml"),
                cache: temp.path().join("cache.json"),
                state_dir: temp.path().join("state"),
            };
            let settings = temp.path().join("claude/settings.json");
            fs::create_dir_all(settings.parent().unwrap()).unwrap();
            fs::create_dir_all(&paths.state_dir).unwrap();
            fs::write(&settings, r#"{"theme":"dark","env":{"KEEP_ME":"yes"}}"#).unwrap();
            fs::write(
                &paths.config,
                r#"
version = 2
[profiles.one]
name = "One"
base_url = "https://one.invalid"
default_model = "model-a"
enabled_models = ["model-b"]
disabled_models = ["hidden"]
[profiles.one.credential]
kind = "bearer"
value = "real-upstream-secret"
[profiles.one.aliases]
haiku = "hidden"
[profiles.two]
name = "Two"
base_url = "https://two.invalid"
default_model = "model-z"
"#,
            )
            .unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            listener.set_nonblocking(true).unwrap();
            fs::write(paths.state_dir.join("proxy.json"), serde_json::to_vec(&json!({"listen": address.to_string(), "local_token": "test-local-token", "routes": {}})).unwrap()).unwrap();
            let stopped = Arc::new(AtomicBool::new(false));
            let stop = stopped.clone();
            // A local health stub prevents unit tests from starting a real daemon.
            let server = thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream
                                .set_read_timeout(Some(Duration::from_secs(1)))
                                .unwrap();
                            let mut request = [0; 4096];
                            let _ = stream.read(&mut request);
                            let body = r#"{"name":"ccsw-proxy"}"#;
                            let _ = write!(
                                stream,
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                body.len(),
                                body
                            );
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2))
                        }
                        Err(_) => break,
                    }
                }
            });
            Self {
                _temp: temp,
                paths,
                settings,
                stopped,
                server: Some(server),
            }
        }
        fn value(&self) -> Value {
            serde_json::from_slice(&fs::read(&self.settings).unwrap()).unwrap()
        }
        fn change(&self, edit: impl FnOnce(&mut Config)) {
            config::update(&self.paths.config, |config| {
                edit(config);
                Ok(())
            })
            .unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.stopped.store(true, Ordering::Relaxed);
            if let Some(server) = self.server.take() {
                server.join().unwrap();
            }
        }
    }

    #[test]
    fn connection_is_explicit_then_automatic_and_keeps_preferred_provider() {
        let fixture = Fixture::new();
        let paths = &fixture.paths;
        let settings = &fixture.settings;
        assert_eq!(inspect(paths, settings).unwrap(), Status::NotConnected);
        assert!(apply(paths, settings, None, false).is_err());
        assert!(!paths.state_dir.join("sync-state.json").exists());
        assert_eq!(
            apply(paths, settings, Some("one"), true)
                .unwrap()
                .model_count,
            3
        );
        assert_eq!(inspect(paths, settings).unwrap(), Status::Synced);
        assert_eq!(fixture.value()["theme"], "dark");
        assert_eq!(fixture.value()["env"]["KEEP_ME"], "yes");
        assert!(
            fixture.value()["env"]
                .get("ANTHROPIC_DEFAULT_HAIKU_MODEL")
                .is_none()
        );
        assert!(
            !fs::read_to_string(settings)
                .unwrap()
                .contains("real-upstream-secret")
        );
        assert!(
            !fs::read_to_string(paths.state_dir.join("sync-state.json"))
                .unwrap()
                .contains("real-upstream-secret")
        );
        fixture.change(|config| config.profiles.get_mut("two").unwrap().name = "Two edited".into());
        assert_eq!(inspect(paths, settings).unwrap(), Status::Pending);
        apply(paths, settings, None, false).unwrap();
        assert_eq!(fixture.value()["model"], "one::model-a");
        fixture.change(|config| config.profiles.get_mut("one").unwrap().enabled = false);
        apply(paths, settings, None, false).unwrap();
        assert_eq!(fixture.value()["model"], "two::model-z");
        fixture.change(|config| config.profiles.get_mut("one").unwrap().enabled = true);
        apply(paths, settings, None, false).unwrap();
        assert_eq!(fixture.value()["model"], "one::model-a");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(paths.state_dir.join("sync-state.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn all_disabled_clears_managed_settings_and_remains_connected() {
        let fixture = Fixture::new();
        apply(&fixture.paths, &fixture.settings, Some("one"), true).unwrap();
        fixture.change(|config| {
            for profile in config.profiles.values_mut() {
                profile.enabled = false;
            }
        });
        assert_eq!(
            apply(&fixture.paths, &fixture.settings, None, false)
                .unwrap()
                .model_count,
            0
        );
        assert_eq!(fixture.value()["theme"], "dark");
        assert!(fixture.value().get("modelPicker").is_none());
        assert_eq!(
            inspect(&fixture.paths, &fixture.settings).unwrap(),
            Status::Synced
        );
        fixture.change(|config| config.profiles.get_mut("one").unwrap().enabled = true);
        apply(&fixture.paths, &fixture.settings, None, false).unwrap();
        assert_eq!(fixture.value()["model"], "one::model-a");
    }

    #[test]
    fn external_target_change_pauses_automatic_sync_until_manual_reconnection() {
        let fixture = Fixture::new();
        apply(&fixture.paths, &fixture.settings, Some("one"), true).unwrap();
        let mut value = fixture.value();
        value["env"]["ANTHROPIC_BASE_URL"] = json!("https://other.invalid");
        fs::write(&fixture.settings, serde_json::to_vec(&value).unwrap()).unwrap();
        let before = fs::read(&fixture.settings).unwrap();
        assert_eq!(
            inspect(&fixture.paths, &fixture.settings).unwrap(),
            Status::Paused
        );
        assert!(apply(&fixture.paths, &fixture.settings, None, false).is_err());
        assert_eq!(fs::read(&fixture.settings).unwrap(), before);
        apply(&fixture.paths, &fixture.settings, Some("two"), true).unwrap();
        assert_eq!(fixture.value()["model"], "two::model-z");
        assert_eq!(
            inspect(&fixture.paths, &fixture.settings).unwrap(),
            Status::Synced
        );
    }

    #[test]
    fn settings_failure_restores_routes_and_retry_uses_latest_config() {
        let fixture = Fixture::new();
        apply(&fixture.paths, &fixture.settings, Some("one"), true).unwrap();
        let before = fs::read(&fixture.settings).unwrap();
        let registry_before: Value =
            serde_json::from_slice(&fs::read(fixture.paths.state_dir.join("proxy.json")).unwrap())
                .unwrap();
        fixture.change(|config| {
            config
                .profiles
                .get_mut("one")
                .unwrap()
                .enabled_models
                .push("model-c".into())
        });
        let backup = fixture.settings.with_extension("json.ccsw-backup");
        fs::remove_file(&backup).unwrap();
        fs::create_dir(&backup).unwrap();
        assert!(apply(&fixture.paths, &fixture.settings, None, false).is_err());
        assert_eq!(fs::read(&fixture.settings).unwrap(), before);
        let registry_after: Value =
            serde_json::from_slice(&fs::read(fixture.paths.state_dir.join("proxy.json")).unwrap())
                .unwrap();
        assert_eq!(registry_after["routes"], registry_before["routes"]);
        assert_eq!(
            inspect(&fixture.paths, &fixture.settings).unwrap(),
            Status::Pending
        );
        fs::remove_dir(&backup).unwrap();
        assert_eq!(
            apply(&fixture.paths, &fixture.settings, None, false)
                .unwrap()
                .model_count,
            4
        );
        assert_eq!(
            inspect(&fixture.paths, &fixture.settings).unwrap(),
            Status::Synced
        );
    }

    #[test]
    fn old_connections_are_recognized_and_other_settings_paths_are_not_connected() {
        let fixture = Fixture::new();
        apply(&fixture.paths, &fixture.settings, Some("two"), true).unwrap();
        fs::remove_file(fixture.paths.state_dir.join("sync-state.json")).unwrap();
        assert_eq!(
            inspect(&fixture.paths, &fixture.settings).unwrap(),
            Status::Pending
        );
        apply(&fixture.paths, &fixture.settings, None, false).unwrap();
        assert_eq!(
            inspect(&fixture.paths, &fixture.settings).unwrap(),
            Status::Synced
        );
        assert_eq!(fixture.value()["model"], "two::model-z");
        assert_eq!(
            inspect(
                &fixture.paths,
                &fixture._temp.path().join("another/settings.json")
            )
            .unwrap(),
            Status::NotConnected
        );
    }
    #[test]
    fn disabled_default_uses_an_active_model_without_reenabling_it() {
        let fixture = Fixture::new();
        apply(&fixture.paths, &fixture.settings, Some("one"), true).unwrap();
        fixture.change(|config| {
            config
                .profiles
                .get_mut("one")
                .unwrap()
                .disabled_models
                .push("model-a".into())
        });
        apply(&fixture.paths, &fixture.settings, None, false).unwrap();
        assert_eq!(fixture.value()["model"], "one::model-b");
        let config = config::load(&fixture.paths.config).unwrap();
        assert_eq!(config.profiles["one"].default_model, "model-a");
        assert!(
            config.profiles["one"]
                .disabled_models
                .contains(&"model-a".into())
        );
    }

    #[test]
    fn connecting_without_models_does_not_clear_existing_claude_settings() {
        let fixture = Fixture::new();
        fixture.change(|config| config.profiles.clear());
        let previous = fs::read(&fixture.settings).unwrap();
        assert!(apply(&fixture.paths, &fixture.settings, None, true).is_err());
        assert_eq!(fs::read(&fixture.settings).unwrap(), previous);
        assert_eq!(
            inspect(&fixture.paths, &fixture.settings).unwrap(),
            Status::NotConnected
        );
    }

    #[test]
    fn missing_settings_path_keeps_its_identity_after_creation() {
        let fixture = Fixture::new();
        let settings = fixture._temp.path().join("new/nested/settings.json");
        let before = identity(&settings).unwrap();
        fs::create_dir_all(settings.parent().unwrap()).unwrap();
        fs::write(&settings, "{}").unwrap();
        assert_eq!(identity(&settings).unwrap(), before);
    }
}
