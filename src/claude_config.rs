use std::{
    collections::BTreeMap,
    fs,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use fs2::FileExt;
use serde_json::{Map, Value, json};
use tempfile::NamedTempFile;

use crate::{
    config::{AppPaths, Config, Credential, ModelEntry, Profile, set_private},
    discovery::{self, ModelCache},
    proxy,
};

pub(crate) const MANAGED_ENV_KEYS: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_MODEL",
    "ANTHROPIC_DEFAULT_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
    "ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
    "ANTHROPIC_DEFAULT_SONNET_MODEL_DESCRIPTION",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL_DESCRIPTION",
    "ANTHROPIC_DEFAULT_FABLE_MODEL",
    "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME",
    "ANTHROPIC_DEFAULT_FABLE_MODEL_DESCRIPTION",
    "CLAUDE_CODE_SUBAGENT_MODEL",
    "CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY",
];

#[derive(Debug)]
pub struct ApplyResult {
    pub path: PathBuf,
    pub backup: Option<PathBuf>,
    pub model_count: usize,
    pub preferences: crate::claude_preferences::Ownership,
}

pub fn settings_path() -> Result<PathBuf> {
    Ok(crate::platform::override_path("CLAUDE_CONFIG_DIR", || {
        Ok(crate::platform::home()?.join(".claude"))
    })?
    .join("settings.json"))
}

#[cfg(test)]
pub fn apply(path: &Path, profile: &Profile, models: &[ModelEntry]) -> Result<ApplyResult> {
    apply_expected(path, profile, models, None, None, None)
}
fn apply_expected(
    path: &Path,
    profile: &Profile,
    models: &[ModelEntry],
    expected: Option<&Value>,
    preferences: Option<&crate::claude_preferences::Settings>,
    previous: Option<&Value>,
) -> Result<ApplyResult> {
    if !profile.enabled {
        anyhow::bail!("cannot apply a disabled provider to Claude");
    }
    let parent = path
        .parent()
        .context("Claude settings path has no parent")?;
    fs::create_dir_all(parent)?;
    let lock_path = path.with_extension("json.ccsw.lock");
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(lock_path)?;
    lock.lock_exclusive()?;
    let mut root = if path.exists() {
        serde_json::from_slice::<Value>(&fs::read(path)?)
            .with_context(|| format!("failed to parse {}", path.display()))?
            .as_object()
            .cloned()
            .context("Claude settings must contain a JSON object")?
    } else {
        Map::new()
    };
    let original_document = Value::Object(root.clone());
    verify_expected(expected, &original_document)?;

    let env = root
        .entry("env")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .context("env in Claude settings must be an object")?;
    for key in MANAGED_ENV_KEYS {
        env.remove(*key);
    }
    env.insert(
        "ANTHROPIC_BASE_URL".into(),
        Value::String(profile.base_url.clone()),
    );
    match &profile.credential {
        Credential::Bearer { value } => {
            env.insert("ANTHROPIC_AUTH_TOKEN".into(), Value::String(value.clone()));
        }
        Credential::XApiKey { value } | Credential::ApiKey { value } => {
            env.insert("ANTHROPIC_API_KEY".into(), Value::String(value.clone()));
        }
        Credential::None => {}
    }
    for (role, model_id) in profile.aliases.iter() {
        let prefix = format!("ANTHROPIC_DEFAULT_{}_MODEL", role.to_ascii_uppercase());
        env.insert(prefix.clone(), Value::String(model_id.to_owned()));
        if let Some(model) = models.iter().find(|model| model.id == model_id) {
            if let Some(label) = &model.label {
                env.insert(format!("{prefix}_NAME"), Value::String(label.clone()));
            }
            if let Some(description) = &model.description {
                env.insert(
                    format!("{prefix}_DESCRIPTION"),
                    Value::String(description.clone()),
                );
            }
        }
    }
    if let Some(model) = &profile.subagent_model {
        env.insert(
            "CLAUDE_CODE_SUBAGENT_MODEL".into(),
            Value::String(model.clone()),
        );
    }

    root.insert("model".into(), Value::String(profile.default_model.clone()));
    root.insert(
        "modelPicker".into(),
        json!({
            "replaceBuiltInOptions": true,
            "options": models.iter().map(model_picker_row).collect::<Vec<_>>()
        }),
    );

    let mut document = Value::Object(root);
    let owned = crate::claude_preferences::apply(
        &mut document,
        preferences.unwrap_or(&crate::claude_preferences::Settings::default()),
        &crate::claude_preferences::from_snapshot(previous),
    )?;
    // Persist recovery metadata before replacing the client file.
    let mut snapshot = managed_snapshot(&document);
    snapshot["client_preferences"] = serde_json::to_value(&owned)?;
    let mut before = managed_snapshot(&original_document);
    let mut before_fields = crate::claude_preferences::from_snapshot(previous);
    for path in owned.keys().chain(before_fields.clone().keys()) {
        let value = original_document.pointer(path).cloned();
        before_fields.insert(
            path.clone(),
            crate::claude_preferences::Owned {
                before: value.clone(),
                after: value,
            },
        );
    }
    before["client_preferences"] = serde_json::to_value(before_fields)?;
    snapshot["client_preference_settings"] =
        serde_json::to_value(preferences.cloned().unwrap_or_default())?;
    write_preferences_journal(
        path,
        &json!({"before": before, "after": snapshot,
        "previous_preferences": crate::claude_preferences::from_snapshot(previous)}),
    )?;
    let backup = if path.exists() {
        let backup = path.with_extension("json.ccsw-backup");
        fs::copy(path, &backup)?;
        set_private(&backup)?;
        Some(backup)
    } else {
        None
    };
    let mut temp = NamedTempFile::new_in(parent)?;
    temp.write_all(serde_json::to_string_pretty(&document)?.as_bytes())?;
    temp.write_all(b"\n")?;
    temp.as_file().sync_all()?;
    set_private(temp.path())?;
    temp.persist(path).map_err(|error| error.error)?;
    set_private(path)?;
    FileExt::unlock(&lock).ok();
    Ok(ApplyResult {
        path: path.to_path_buf(),
        backup,
        model_count: models.len(),
        preferences: owned,
    })
}

fn verify_expected(expected: Option<&Value>, value: &Value) -> Result<()> {
    if let Some(expected) = expected {
        let conflicts = managed_conflicts(expected, value);
        if !conflicts.is_empty() {
            anyhow::bail!(
                "Claude fields changed before write: {}; press p to reconnect",
                conflicts.join(", ")
            );
        }
    }
    Ok(())
}

/// Exact snapshot of present managed fields. Missing and explicit null differ.
pub(crate) fn managed_snapshot(value: &Value) -> Value {
    let mut snapshot = Map::new();
    let mut env = Map::new();
    for key in MANAGED_ENV_KEYS {
        if let Some(v) = value.get("env").and_then(|e| e.get(*key)) {
            env.insert((*key).into(), v.clone());
        }
    }
    snapshot.insert("env".into(), Value::Object(env));
    for key in ["model", "modelPicker"] {
        if let Some(v) = value.get(key) {
            snapshot.insert(key.into(), v.clone());
        }
    }
    Value::Object(snapshot)
}

pub(crate) fn managed_conflicts(snapshot: &Value, current: &Value) -> Vec<String> {
    let mut conflicts = crate::claude_preferences::conflicts(
        &crate::claude_preferences::from_snapshot(Some(snapshot)),
        current,
    );
    for key in MANAGED_ENV_KEYS {
        if snapshot["env"].get(*key) != current.get("env").and_then(|e| e.get(*key)) {
            conflicts.push(format!("env.{key}"));
        }
    }
    for key in ["model", "modelPicker"] {
        if snapshot.get(key) != current.get(key) {
            conflicts.push(key.into());
        }
    }
    conflicts
}

/// Called only after endpoint/token ownership is established. Legacy connections
/// without a snapshot authorize removing the connection fields, not model fields.
pub(crate) fn remove_managed(value: &mut Value, snapshot: Option<&Value>) {
    // Restoration only touches values that still match our last write.
    let _ = crate::claude_preferences::restore(
        value,
        &crate::claude_preferences::from_snapshot(snapshot),
    );
    if let Some(env) = value.get_mut("env").and_then(Value::as_object_mut) {
        for key in MANAGED_ENV_KEYS {
            let removable = match snapshot {
                Some(saved) => {
                    saved["env"].get(*key).is_some() && saved["env"].get(*key) == env.get(*key)
                }
                None => matches!(*key, "ANTHROPIC_BASE_URL" | "ANTHROPIC_AUTH_TOKEN"),
            };
            if removable {
                env.remove(*key);
            }
        }
    }
    if let (Some(root), Some(saved)) = (value.as_object_mut(), snapshot) {
        for key in ["model", "modelPicker"] {
            if saved.get(key).is_some() && saved.get(key) == root.get(key) {
                root.remove(key);
            }
        }
    }
}

#[cfg(test)]
pub fn clear(path: &Path) -> Result<ApplyResult> {
    clear_expected(path, None)
}
pub fn clear_expected(path: &Path, expected: Option<&Value>) -> Result<ApplyResult> {
    let parent = path
        .parent()
        .context("Claude settings path has no parent")?;
    fs::create_dir_all(parent)?;
    let lock_path = path.with_extension("json.ccsw.lock");
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(lock_path)?;
    lock.lock_exclusive()?;
    let mut root = if path.exists() {
        serde_json::from_slice::<Value>(&fs::read(path)?)
            .with_context(|| format!("failed to parse {}", path.display()))?
            .as_object()
            .cloned()
            .context("Claude settings must contain a JSON object")?
    } else {
        Map::new()
    };
    verify_expected(expected, &Value::Object(root.clone()))?;
    if let Some(env) = root.get_mut("env").and_then(Value::as_object_mut) {
        for key in MANAGED_ENV_KEYS {
            env.remove(*key);
        }
    }
    root.remove("model");
    root.remove("modelPicker");

    let backup = if path.exists() {
        let backup = path.with_extension("json.ccsw-backup");
        fs::copy(path, &backup)?;
        set_private(&backup)?;
        Some(backup)
    } else {
        None
    };
    let mut temp = NamedTempFile::new_in(parent)?;
    temp.write_all(serde_json::to_string_pretty(&Value::Object(root))?.as_bytes())?;
    temp.write_all(b"\n")?;
    temp.as_file().sync_all()?;
    set_private(temp.path())?;
    temp.persist(path).map_err(|error| error.error)?;
    set_private(path)?;
    FileExt::unlock(&lock).ok();
    Ok(ApplyResult {
        path: path.to_path_buf(),
        backup,
        model_count: 0,
        preferences: crate::claude_preferences::from_snapshot(expected),
    })
}

pub fn apply_all(
    path: &Path,
    paths: &AppPaths,
    config: &Config,
    cache: &ModelCache,
    default_profile_id: &str,
    expected: Option<&Value>,
    previous: Option<&Value>,
) -> Result<ApplyResult> {
    let models_by_profile = config
        .profiles
        .iter()
        .map(|(profile_id, profile)| {
            let discovered = cache
                .profiles
                .get(profile_id)
                .map(|cached| cached.models.as_slice())
                .unwrap_or_default();
            (
                profile_id.clone(),
                discovery::active_models(profile, discovered),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let (profile, models) =
        proxy::aggregate_profile(paths, config, &models_by_profile, default_profile_id)?;
    apply_expected(
        path,
        &profile,
        &models,
        expected,
        Some(&config.claude),
        previous,
    )
}

fn model_picker_row(model: &ModelEntry) -> Value {
    let mut row = Map::new();
    row.insert("model".into(), Value::String(model.id.clone()));
    if let Some(label) = &model.label {
        row.insert("label".into(), Value::String(label.clone()));
    }
    if let Some(description) = &model.description {
        row.insert("description".into(), Value::String(description.clone()));
    }
    Value::Object(row)
}

fn preferences_journal(path: &Path) -> std::path::PathBuf {
    path.with_extension("json.ccsw-preferences-journal")
}
fn write_preferences_journal(path: &Path, snapshot: &Value) -> Result<()> {
    let mut temp = NamedTempFile::new_in(path.parent().context("missing settings parent")?)?;
    temp.write_all(&serde_json::to_vec(snapshot)?)?;
    temp.as_file().sync_all()?;
    set_private(temp.path())?;
    temp.persist(preferences_journal(path))
        .map_err(|e| e.error)?;
    Ok(())
}
pub(crate) fn recover_preferences(path: &Path, document: &Value) -> Result<Option<Value>> {
    let journal = preferences_journal(path);
    if !journal.exists() {
        return Ok(None);
    }
    let pending: Value = serde_json::from_slice(&fs::read(&journal)?)?;
    let snapshot = pending
        .get("after")
        .context("invalid preference recovery journal")?;
    if pending
        .get("before")
        .is_some_and(|before| managed_conflicts(before, document).is_empty())
    {
        return Ok(None);
    }
    let mut recovered = snapshot.clone();
    let mut routing = snapshot.clone();
    routing
        .as_object_mut()
        .context("invalid recovery snapshot")?
        .remove("client_preferences");
    if !managed_conflicts(&routing, document).is_empty() {
        anyhow::bail!(
            "Claude connection changed during interrupted sync; its settings were left untouched"
        );
    }
    let previous: crate::claude_preferences::Ownership =
        serde_json::from_value(pending["previous_preferences"].clone())?;
    let next = crate::claude_preferences::from_snapshot(Some(snapshot));
    let mut owned = next.clone();
    // Account for removals and outside edits as well as completed additions.
    // An unchanged old override still needs its original baseline on retry.
    for (path, old) in previous {
        let current = document.pointer(&path).cloned();
        if !next.get(&path).is_some_and(|new| new.after == current) && old.after == current {
            owned.insert(path, old);
        }
    }
    recovered["client_preferences"] = serde_json::to_value(owned)?;
    Ok(Some(recovered))
}

pub(crate) fn finish_preferences(path: &Path) -> Result<()> {
    match fs::remove_file(preferences_journal(path)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Explicit disconnect restores only unchanged owned fields and leaves outside edits alone.
pub(crate) fn disconnect_owned(path: &Path, snapshot: &Value) -> Result<Vec<String>> {
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.with_extension("json.ccsw.lock"))?;
    lock.lock_exclusive()?;
    let mut document: Value = serde_json::from_slice(&fs::read(path)?)?;
    if document["env"].get("ANTHROPIC_BASE_URL").is_some()
        || document["env"].get("ANTHROPIC_AUTH_TOKEN").is_some()
    {
        for key in ["ANTHROPIC_BASE_URL", "ANTHROPIC_AUTH_TOKEN"] {
            if document["env"].get(key) != snapshot["env"].get(key) {
                anyhow::bail!(
                    "Claude connection changed before disconnect; no fields were changed"
                );
            }
        }
    }
    let conflicts = managed_conflicts(snapshot, &document);
    remove_managed(&mut document, Some(snapshot));
    let mut temp = NamedTempFile::new_in(path.parent().context("missing settings parent")?)?;
    temp.write_all(&serde_json::to_vec_pretty(&document)?)?;
    temp.as_file().sync_all()?;
    set_private(temp.path())?;
    temp.persist(path).map_err(|e| e.error)?;
    finish_preferences(path)?;
    Ok(conflicts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RoleModels;

    fn profile() -> Profile {
        Profile {
            name: "Route".into(),
            enabled: true,
            base_url: "https://gateway.example".into(),
            api_format: crate::config::ApiFormat::Anthropic,
            credential: Credential::Bearer {
                value: "secret-token".into(),
            },
            default_model: "model-a".into(),
            aliases: RoleModels {
                sonnet: Some("model-a".into()),
                ..Default::default()
            },
            subagent_model: None,
            fallback_models: vec![],
            enabled_models: vec!["model-b".into()],
            disabled_models: vec![],
            models: vec![],
        }
    }

    #[test]
    fn detach_preserves_changes_and_distinguishes_absent_from_null() {
        let mut value = json!({"model":"ccsw-model","modelPicker":{"options":[]},"env":{"ANTHROPIC_BASE_URL":"http://localhost","ANTHROPIC_AUTH_TOKEN":"secret","ANTHROPIC_MODEL":null,"KEEP":"yes"}});
        let saved = managed_snapshot(&value);
        value["model"] = json!("custom");
        value["env"]
            .as_object_mut()
            .unwrap()
            .remove("ANTHROPIC_MODEL");
        assert_eq!(
            managed_conflicts(&saved, &value),
            vec!["env.ANTHROPIC_MODEL", "model"]
        );
        remove_managed(&mut value, Some(&saved));
        assert_eq!(value, json!({"model":"custom","env":{"KEEP":"yes"}}));
        let mut legacy = json!({"model":"custom","env":{"ANTHROPIC_BASE_URL":"http://localhost","ANTHROPIC_AUTH_TOKEN":"secret","ANTHROPIC_DEFAULT_OPUS_MODEL":"keep"}});
        remove_managed(&mut legacy, None);
        assert_eq!(legacy["model"], "custom");
        assert_eq!(legacy["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"], "keep");
    }

    #[test]
    fn apply_preserves_unrelated_settings_and_replaces_managed_route() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        fs::write(
            &path,
            r#"{"theme":"dark","env":{"KEEP_ME":"yes","ANTHROPIC_API_KEY":"old"}}"#,
        )
        .unwrap();
        let models = ["model-a", "model-b"]
            .into_iter()
            .map(|id| ModelEntry {
                max_output_tokens: None,
                context_window: None,
                reasoning_max: None,
                id: id.into(),
                label: None,
                description: None,
            })
            .collect::<Vec<_>>();
        let result = apply(&path, &profile(), &models).unwrap();
        let value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["theme"], "dark");
        assert_eq!(value["env"]["KEEP_ME"], "yes");
        assert!(value["env"].get("ANTHROPIC_API_KEY").is_none());
        assert_eq!(value["env"]["ANTHROPIC_AUTH_TOKEN"], "secret-token");
        assert_eq!(value["modelPicker"]["options"].as_array().unwrap().len(), 2);
        assert_eq!(result.model_count, 2);
        assert!(result.backup.unwrap().exists());
    }

    #[test]
    fn clear_removes_managed_models_and_preserves_unrelated_settings() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        fs::write(
            &path,
            r#"{"theme":"dark","model":"route::model-a","modelPicker":{"replaceBuiltInOptions":true,"options":[{"model":"route::model-a"}]},"env":{"KEEP_ME":"yes","ANTHROPIC_BASE_URL":"http://localhost"}}"#,
        )
        .unwrap();

        let result = clear(&path).unwrap();
        let value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["theme"], "dark");
        assert_eq!(value["env"]["KEEP_ME"], "yes");
        assert!(value.get("model").is_none());
        assert!(value.get("modelPicker").is_none());
        assert!(value["env"].get("ANTHROPIC_BASE_URL").is_none());
        assert_eq!(result.model_count, 0);
    }
}
