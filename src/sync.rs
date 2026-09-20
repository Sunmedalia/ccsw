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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    managed: Option<Value>,
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
    crate::platform::identity(path)
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
    if binding
        .managed
        .as_ref()
        .is_some_and(|saved| !claude_config::managed_conflicts(saved, value).is_empty())
    {
        return false;
    }
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
        if binding.managed.is_none() || !matches(binding, &value) {
            return Ok(Status::Paused);
        }
        if binding.endpoint.is_some() && !proxy::owns_settings(paths, &value)? {
            return Ok(Status::Pending);
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
        Status::Paused
    } else {
        Status::NotConnected
    })
}

/// Credential-free explanation of why automatic synchronization is paused.
pub fn diagnostic(paths: &AppPaths, settings: &Path) -> Result<Vec<String>> {
    let config_path = identity(&paths.config)?;
    let settings_path = identity(settings)?;
    let state = load(paths)?;
    let current = read_settings(settings)?;
    let Some(binding) = state
        .entries
        .iter()
        .find(|e| e.config == config_path && e.settings == settings_path)
    else {
        return Ok(vec![]);
    };
    let Some(saved) = &binding.managed else {
        return Ok(vec!["legacy_snapshot".into()]);
    };
    Ok(claude_config::managed_conflicts(saved, &current))
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
    let recovered = if proxy::owns_settings(paths, &value)? {
        claude_config::recover_preferences(settings, &value)?
    } else {
        None
    };
    if let Some(recovered) = &recovered
        && let Some(i) = index
    {
        state.entries[i].managed = Some(recovered.clone());
        save(paths, &state)?;
        claude_config::finish_preferences(settings)?;
    }

    if !explicit {
        if let Some(saved) = index.and_then(|i| state.entries[i].managed.as_ref()) {
            let conflicts = claude_config::managed_conflicts(saved, &value);
            if !conflicts.is_empty() {
                bail!(
                    "Claude managed fields changed externally: {}; press p to reconnect",
                    conflicts.join(", ")
                );
            }
        } else {
            bail!("legacy connection has no field snapshot; press p once to establish ownership");
        }
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
    let expected = if explicit {
        None
    } else {
        index.and_then(|i| state.entries[i].managed.as_ref())
    };
    let checkpoint = proxy::aggregate_checkpoint(paths)?;
    let result = if let Some(id) = &chosen {
        claude_config::apply_all(
            settings,
            paths,
            &effective,
            &cache,
            id,
            expected,
            index
                .and_then(|i| state.entries[i].managed.as_ref())
                .or(recovered.as_ref()),
        )
    } else {
        proxy::clear_aggregate_models(paths)
            .and_then(|()| claude_config::clear_expected(settings, expected))
            .map(|mut result| {
                result.preferences = crate::claude_preferences::from_snapshot(
                    index
                        .and_then(|i| state.entries[i].managed.as_ref())
                        .or(recovered.as_ref()),
                );
                result
            })
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
    let previous_preferences: crate::claude_preferences::Settings = index
        .and_then(|i| state.entries[i].managed.as_ref())
        .and_then(|v| v.get("client_preference_settings"))
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let preferences_pending = chosen.is_none() && previous_preferences != config.claude;

    let binding = Binding {
        managed: Some({
            let mut snapshot = claude_config::managed_snapshot(&applied);
            snapshot["client_preferences"] = serde_json::to_value(&result.preferences)?;
            snapshot["client_preference_settings"] = serde_json::to_value(if chosen.is_some() {
                &config.claude
            } else {
                &previous_preferences
            })?;
            snapshot
        }),
        config: config_path,
        settings: settings_path,
        preferred: preferred
            .map(str::to_owned)
            .or_else(|| previous.map(str::to_owned))
            .or(chosen),
        revision: if preferences_pending {
            0
        } else {
            revision(&config)?
        },
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
    claude_config::finish_preferences(settings)?;
    Ok(result)
}

pub fn disconnect(paths: &AppPaths, settings: &Path) -> Result<Vec<String>> {
    fs::create_dir_all(&paths.state_dir)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(paths.state_dir.join("sync-state.lock"))?;
    lock.try_lock_exclusive()
        .context("Sync is busy; retry disconnect when it finishes")?;
    let mut state = load(paths)?;
    let config_path = identity(&paths.config)?;
    let settings_path = identity(settings)?;
    let index = state
        .entries
        .iter()
        .position(|b| b.config == config_path && b.settings == settings_path)
        .context("Claude is not connected to this configuration")?;
    let value = read_settings(settings)?;
    let binding = &state.entries[index];
    let saved = binding
        .managed
        .as_ref()
        .context("Legacy connection has no snapshot; reconnect first")?;
    // A switched endpoint belongs to another connection; never restore into it.
    if (value["env"].get("ANTHROPIC_BASE_URL").is_some()
        || value["env"].get("ANTHROPIC_AUTH_TOKEN").is_some())
        && (value["env"]["ANTHROPIC_BASE_URL"].as_str() != binding.endpoint.as_deref()
            || value["env"]["ANTHROPIC_AUTH_TOKEN"].as_str() != binding.token.as_deref())
    {
        bail!("Claude's connection changed externally; it was left untouched");
    }
    let conflicts = claude_config::disconnect_owned(settings, saved)?;
    state.entries.remove(index);
    save(paths, &state)?;
    Ok(conflicts)
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
                            stream.set_nonblocking(false).unwrap();
                            stream
                                .set_read_timeout(Some(Duration::from_secs(1)))
                                .unwrap();
                            let mut request = Vec::new();
                            let mut chunk = [0; 1024];
                            while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                                match stream.read(&mut chunk) {
                                    Ok(0) | Err(_) => break,
                                    Ok(count) => request.extend_from_slice(&chunk[..count]),
                                }
                            }
                            let body = json!({"name":"ccsw-proxy", "config_version":config::CONFIG_VERSION, "version":env!("CARGO_PKG_VERSION")}).to_string();
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
    fn disabling_all_models_preserves_preferences_until_reenabled_or_disconnected() {
        let f = Fixture::new();
        f.change(|c| {
            c.claude.env.insert("CUSTOM".into(), "first".into());
        });
        apply(&f.paths, &f.settings, Some("one"), true).unwrap();
        f.change(|c| {
            c.claude.env.insert("CUSTOM".into(), "next".into());
            for profile in c.profiles.values_mut() {
                profile.enabled = false;
            }
        });
        let result = apply(&f.paths, &f.settings, None, false).unwrap();
        assert_eq!(result.model_count, 0);
        assert_eq!(f.value()["env"]["CUSTOM"], "first");
        assert_eq!(inspect(&f.paths, &f.settings).unwrap(), Status::Pending);
        // Explicit sync with no models must not forget original ownership either.
        apply(&f.paths, &f.settings, None, true).unwrap();
        disconnect(&f.paths, &f.settings).unwrap();
        assert!(f.value()["env"].get("CUSTOM").is_none());
    }

    #[test]
    fn interrupted_removal_with_external_edit_preserves_the_external_value() {
        let f = Fixture::new();
        f.change(|c| {
            c.claude.env.insert("CUSTOM".into(), "ours".into());
        });
        apply(&f.paths, &f.settings, Some("one"), true).unwrap();
        let state = load(&f.paths).unwrap();
        f.change(|c| {
            c.claude.env.clear();
        });
        let config = config::load(&f.paths.config).unwrap();
        claude_config::apply_all(
            &f.settings,
            &f.paths,
            &config,
            &discovery::ModelCache::default(),
            "one",
            None,
            state.entries[0].managed.as_ref(),
        )
        .unwrap();
        let mut current = f.value();
        current["env"]["CUSTOM"] = json!("external");
        fs::write(&f.settings, serde_json::to_vec(&current).unwrap()).unwrap();
        apply(&f.paths, &f.settings, Some("one"), true).unwrap();
        disconnect(&f.paths, &f.settings).unwrap();
        assert_eq!(f.value()["env"]["CUSTOM"], "external");
    }

    #[test]
    fn preferences_follow_sync_restore_and_disconnect() {
        let f = Fixture::new();
        let mut original = f.value();
        original["env"]["CUSTOM"] = json!("before");
        original["attribution"] = json!({"commit":"original","pr":"original-pr","extra":true});
        fs::write(&f.settings, serde_json::to_vec(&original).unwrap()).unwrap();
        f.change(|c| {
            c.claude.env.insert("CUSTOM".into(), "after".into());
            c.claude.hide_attribution = Some(true);
        });
        apply(&f.paths, &f.settings, Some("one"), true).unwrap();
        assert_eq!(f.value()["env"]["CUSTOM"], "after");
        apply(&f.paths, &f.settings, Some("two"), true).unwrap();
        assert_eq!(f.value()["env"]["CUSTOM"], "after");
        assert_eq!(f.value()["attribution"]["commit"], "");
        f.change(|c| {
            c.claude.env.clear();
        });
        apply(&f.paths, &f.settings, None, false).unwrap();
        assert_eq!(f.value()["env"]["CUSTOM"], "before");
        disconnect(&f.paths, &f.settings).unwrap();
        assert_eq!(f.value(), original);
        assert_eq!(
            inspect(&f.paths, &f.settings).unwrap(),
            Status::NotConnected
        );
    }
    #[test]
    fn preference_external_edit_pauses_then_retake_restores_new_baseline() {
        let f = Fixture::new();
        f.change(|c| {
            c.claude.env.insert("CUSTOM".into(), "ours".into());
        });
        apply(&f.paths, &f.settings, Some("one"), true).unwrap();
        let mut value = f.value();
        value["env"]["CUSTOM"] = json!("external-secret");
        fs::write(&f.settings, serde_json::to_vec(&value).unwrap()).unwrap();
        let error = apply(&f.paths, &f.settings, None, false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("CUSTOM"));
        assert!(!error.contains("external-secret"));
        apply(&f.paths, &f.settings, Some("one"), true).unwrap();
        disconnect(&f.paths, &f.settings).unwrap();
        assert_eq!(f.value()["env"]["CUSTOM"], "external-secret");
    }
    #[test]
    fn interrupted_first_sync_keeps_original_preference_baseline() {
        let f = Fixture::new();
        let mut original = f.value();
        original["env"]["CUSTOM"] = json!("before");
        fs::write(&f.settings, serde_json::to_vec(&original).unwrap()).unwrap();
        f.change(|c| {
            c.claude.env.insert("CUSTOM".into(), "ours".into());
        });
        let config = config::load(&f.paths.config).unwrap();
        claude_config::apply_all(
            &f.settings,
            &f.paths,
            &config,
            &discovery::ModelCache::default(),
            "one",
            None,
            None,
        )
        .unwrap();
        // Simulate termination after settings replacement but before Binding save.
        apply(&f.paths, &f.settings, Some("one"), true).unwrap();
        disconnect(&f.paths, &f.settings).unwrap();
        assert_eq!(f.value()["env"]["CUSTOM"], "before");
    }

    #[test]
    fn external_model_edit_pauses_sync_without_exposing_values() {
        let fixture = Fixture::new();
        apply(&fixture.paths, &fixture.settings, Some("one"), true).unwrap();
        let mut value = fixture.value();
        value["model"] = json!("user-selected-private-model");
        fs::write(&fixture.settings, serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(
            inspect(&fixture.paths, &fixture.settings).unwrap(),
            Status::Paused
        );
        let error = apply(&fixture.paths, &fixture.settings, None, false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("model"));
        assert!(!error.contains("user-selected-private-model"));
        assert_eq!(fixture.value(), value);
        apply(&fixture.paths, &fixture.settings, Some("one"), true).unwrap();
        assert_eq!(
            inspect(&fixture.paths, &fixture.settings).unwrap(),
            Status::Synced
        );
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
        assert_eq!(
            fixture.value()["env"]["ANTHROPIC_DEFAULT_HAIKU_MODEL"],
            "ccsw-role::haiku"
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
            Status::Paused
        );
        assert!(apply(&fixture.paths, &fixture.settings, None, false).is_err());
        apply(&fixture.paths, &fixture.settings, Some("two"), true).unwrap();
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
    #[test]
    fn changed_proxy_port_is_pending_even_without_model_edits() {
        let fixture = Fixture::new();
        apply(&fixture.paths, &fixture.settings, Some("one"), true).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        proxy::set_port(&fixture.paths, port).unwrap();
        assert_eq!(
            inspect(&fixture.paths, &fixture.settings).unwrap(),
            Status::Pending
        );
        assert!(
            fixture.value()["env"]["ANTHROPIC_BASE_URL"]
                .as_str()
                .unwrap()
                .contains("/r/")
        );
    }
}
