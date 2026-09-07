use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use url::Url;

pub const CONFIG_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            profiles: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    #[serde(default = "default_profile_enabled", skip_serializing_if = "is_true")]
    pub enabled: bool,
    pub base_url: String,
    #[serde(default)]
    pub api_format: ApiFormat,
    #[serde(default)]
    pub credential: Credential,
    pub default_model: String,
    #[serde(default)]
    pub aliases: RoleModels,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_model: Option<String>,
    #[serde(default)]
    pub fallback_models: Vec<String>,
    /// Additional catalog model ids explicitly enabled by the user.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enabled_models: Vec<String>,
    /// Catalog model ids explicitly disabled by the user.
    ///
    /// This is kept separately from `models`: disabling controls availability,
    /// while deleting a model removes its catalog entry.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_models: Vec<String>,
    #[serde(default)]
    pub models: Vec<ModelEntry>,
}

fn default_profile_enabled() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApiFormat {
    #[default]
    Anthropic,
    OpenaiChat,
    OpenaiResponses,
}

impl ApiFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Anthropic => "Anthropic",
            Self::OpenaiChat => "OpenAI Chat",
            Self::OpenaiResponses => "OpenAI Responses",
        }
    }

    pub fn is_openai(self) -> bool {
        !matches!(self, Self::Anthropic)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Credential {
    Bearer {
        value: String,
    },
    XApiKey {
        value: String,
    },
    ApiKey {
        value: String,
    },
    #[default]
    None,
}

impl Credential {
    pub fn masked(&self) -> String {
        match self {
            Self::Bearer { value } => format!("Bearer {}", mask_secret(value)),
            Self::XApiKey { value } => format!("x-api-key {}", mask_secret(value)),
            Self::ApiKey { value } => format!("api-key {}", mask_secret(value)),
            Self::None => "No authentication".into(),
        }
    }

    pub fn value(&self) -> Option<&str> {
        match self {
            Self::Bearer { value } | Self::XApiKey { value } | Self::ApiKey { value } => {
                Some(value)
            }
            Self::None => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleModels {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opus: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sonnet: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub haiku: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fable: Option<String>,
}

impl RoleModels {
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, &str)> {
        [
            ("opus", self.opus.as_deref()),
            ("sonnet", self.sonnet.as_deref()),
            ("haiku", self.haiku.as_deref()),
            ("fable", self.fable.as_deref()),
        ]
        .into_iter()
        .filter_map(|(role, model)| model.map(|model| (role, model)))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelEntry {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl ModelEntry {
    pub fn label(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.id)
    }
}

pub(crate) fn canonical_model_id(id: &str) -> &str {
    if id.to_ascii_lowercase().ends_with("[1m]") {
        &id[..id.len().saturating_sub(4)]
    } else {
        id
    }
}

pub(crate) fn deduplicate_model_entries(
    models: impl IntoIterator<Item = ModelEntry>,
) -> Vec<ModelEntry> {
    let mut unique = BTreeMap::<String, ModelEntry>::new();
    for model in models {
        let canonical = canonical_model_id(&model.id).to_owned();
        match unique.entry(canonical.clone()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(model);
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                let existing = entry.get_mut();
                let prefer_incoming = canonical_model_id(&existing.id) != existing.id
                    && canonical_model_id(&model.id) == model.id;
                if prefer_incoming {
                    let mut preferred = model;
                    if preferred.label.is_none() {
                        preferred.label = existing.label.take();
                    }
                    if preferred.description.is_none() {
                        preferred.description = existing.description.take();
                    }
                    *existing = preferred;
                } else {
                    if existing.label.is_none() {
                        existing.label = model.label;
                    }
                    if existing.description.is_none() {
                        existing.description = model.description;
                    }
                }
            }
        }
    }
    unique.into_values().collect()
}

impl Profile {
    pub fn required_model_ids(&self) -> BTreeSet<String> {
        let mut ids = BTreeSet::from([self.default_model.clone()]);
        ids.extend(self.aliases.iter().map(|(_, id)| id.to_owned()));
        ids.extend(self.subagent_model.iter().cloned());
        ids.extend(self.fallback_models.iter().cloned());
        ids
    }

    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            bail!("profile name cannot be empty");
        }
        let url = Url::parse(&self.base_url).context("base_url is not a valid URL")?;
        if !matches!(url.scheme(), "http" | "https") {
            bail!("base_url must use http or https");
        }
        if self.default_model.trim().is_empty() {
            bail!("default_model cannot be empty");
        }
        if canonical_context_model(&self.default_model)
            .trim()
            .is_empty()
        {
            bail!("default_model must contain a model id before [1m]");
        }
        if let Some(value) = self.credential.value()
            && value.is_empty()
        {
            bail!("credential cannot be empty");
        }
        if self.api_format == ApiFormat::Anthropic
            && matches!(self.credential, Credential::ApiKey { .. })
        {
            bail!("Anthropic routes cannot emit an api-key header; use x-api-key or bearer");
        }
        let mut ids = BTreeSet::new();
        for model in &self.models {
            let canonical = canonical_context_model(&model.id);
            if canonical.trim().is_empty() {
                bail!("model id cannot be empty");
            }
            if !ids.insert(canonical) {
                bail!("duplicate model id after normalizing [1m]: {}", model.id);
            }
        }
        let mut enabled = BTreeSet::new();
        for id in &self.enabled_models {
            let canonical = canonical_context_model(id);
            if canonical.trim().is_empty() {
                bail!("enabled model id cannot be empty");
            }
            if !enabled.insert(canonical) {
                bail!("duplicate enabled model id after normalizing [1m]: {id}");
            }
        }
        let mut disabled = BTreeSet::new();
        for id in &self.disabled_models {
            let canonical = canonical_context_model(id);
            if canonical.trim().is_empty() {
                bail!("disabled model id cannot be empty");
            }
            if !disabled.insert(canonical) {
                bail!("duplicate disabled model id after normalizing [1m]: {id}");
            }
            if enabled.contains(canonical) {
                bail!("model cannot be both enabled and disabled: {id}");
            }
        }
        Ok(())
    }
}

fn canonical_context_model(id: &str) -> &str {
    canonical_model_id(id)
}

fn default_version() -> u32 {
    CONFIG_VERSION
}

pub fn mask_secret(value: &str) -> String {
    if value.is_empty() {
        "(empty)".into()
    } else {
        "••••••".into()
    }
}

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub config: PathBuf,
    pub state_dir: PathBuf,
    pub cache: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self> {
        let (config_dir, state_dir, cache_dir) = crate::platform::directories()?;
        let config = env::var_os("CCSW_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(|| config_dir.join("config.toml"));
        let cache = cache_dir.join("models.json");
        Ok(Self {
            config,
            state_dir,
            cache,
        })
    }
}

pub fn load(path: &Path) -> Result<Config> {
    if !path.exists() {
        return Ok(Config::default());
    }
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut raw: toml::Value =
        toml::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))?;
    let version = raw
        .get("version")
        .and_then(toml::Value::as_integer)
        .unwrap_or(1);
    if version == 1 {
        migrate_v1(&mut raw)?;
    } else if version != i64::from(CONFIG_VERSION) {
        bail!(
            "unsupported config version {}; expected {}",
            version,
            CONFIG_VERSION
        );
    }
    let mut config: Config = raw
        .try_into()
        .with_context(|| format!("failed to parse {}", path.display()))?;
    for profile in config.profiles.values_mut() {
        profile.models = deduplicate_model_entries(std::mem::take(&mut profile.models));
    }
    for (id, profile) in &config.profiles {
        validate_profile_id(id).with_context(|| format!("invalid profile id {id}"))?;
        profile
            .validate()
            .with_context(|| format!("invalid profile {id}"))?;
    }
    Ok(config)
}

fn migrate_v1(raw: &mut toml::Value) -> Result<()> {
    let table = raw
        .as_table_mut()
        .context("config root must be a TOML table")?;
    table.insert("version".into(), toml::Value::Integer(2));
    if let Some(profiles) = table
        .get_mut("profiles")
        .and_then(toml::Value::as_table_mut)
    {
        for profile in profiles
            .iter_mut()
            .filter_map(|(_, value)| value.as_table_mut())
        {
            profile.insert("api_format".into(), toml::Value::String("anthropic".into()));
            if let Some(kind) = profile
                .get_mut("credential")
                .and_then(toml::Value::as_table_mut)
                .and_then(|credential| credential.get_mut("kind"))
                .and_then(|value| value.as_str())
                .map(str::to_owned)
            {
                let migrated = match kind.as_str() {
                    "auth-token" => "bearer",
                    "api-key" => "x-api-key",
                    _ => continue,
                };
                profile
                    .get_mut("credential")
                    .and_then(toml::Value::as_table_mut)
                    .expect("credential was a table above")
                    .insert("kind".into(), toml::Value::String(migrated.into()));
            }
        }
    }
    Ok(())
}

pub fn update(path: &Path, edit: impl FnOnce(&mut Config) -> Result<()>) -> Result<Config> {
    update_locked(path, edit, true)
}

/// Interactive edits fail promptly on contention so the terminal remains usable.
pub fn try_update(path: &Path, edit: impl FnOnce(&mut Config) -> Result<()>) -> Result<Config> {
    update_locked(path, edit, false)
}

fn update_locked(
    path: &Path,
    edit: impl FnOnce(&mut Config) -> Result<()>,
    wait: bool,
) -> Result<Config> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let lock_path = path.with_extension("toml.lock");
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)?;
    if wait {
        lock.lock_exclusive()?;
    } else {
        lock.try_lock_exclusive()
            .context("configuration is busy in another CCSW instance; retry saving")?;
    }
    let mut latest = load(path)?;
    edit(&mut latest)?;
    latest.version = CONFIG_VERSION;
    for (id, profile) in &latest.profiles {
        validate_profile_id(id)?;
        profile.validate()?;
    }
    write_unlocked(path, &latest)?;
    FileExt::unlock(&lock).ok();
    Ok(latest)
}

/// Merge an edit against the latest profile without overwriting independent changes.
/// Arrays are atomic fields; conflicting edits must be retried from a fresh view.
pub fn merge_profile(original: &Profile, edited: &Profile, latest: &Profile) -> Result<Profile> {
    fn merge(
        old: &serde_json::Value,
        new: &serde_json::Value,
        live: &serde_json::Value,
        field: &str,
    ) -> Result<serde_json::Value> {
        if old == new {
            return Ok(live.clone());
        }
        if old == live || new == live {
            return Ok(new.clone());
        }
        if let (Some(old), Some(new), Some(live)) =
            (old.as_object(), new.as_object(), live.as_object())
        {
            let keys = old
                .keys()
                .chain(new.keys())
                .chain(live.keys())
                .collect::<BTreeSet<_>>();
            let mut result = serde_json::Map::new();
            for key in keys {
                let value = merge(
                    old.get(key).unwrap_or(&serde_json::Value::Null),
                    new.get(key).unwrap_or(&serde_json::Value::Null),
                    live.get(key).unwrap_or(&serde_json::Value::Null),
                    &format!("{field}.{key}"),
                )?;
                if !value.is_null() {
                    result.insert(key.clone(), value);
                }
            }
            return Ok(result.into());
        }
        bail!("{field} changed in another CCSW instance; reopen the editor and retry");
    }
    let merged = merge(
        &serde_json::to_value(original)?,
        &serde_json::to_value(edited)?,
        &serde_json::to_value(latest)?,
        "profile",
    )?;
    let profile: Profile = serde_json::from_value(merged)?;
    profile.validate()?;
    Ok(profile)
}

pub fn update_profile(
    path: &Path,
    id: &str,
    original: &Profile,
    edited: &Profile,
) -> Result<Config> {
    try_update(path, |latest| {
        let current = latest
            .profiles
            .get(id)
            .context("provider was removed in another CCSW instance")?;
        let merged = merge_profile(original, edited, current)?;
        latest.profiles.insert(id.to_owned(), merged);
        Ok(())
    })
}

fn write_unlocked(path: &Path, config: &Config) -> Result<()> {
    let encoded = toml::to_string_pretty(config).context("failed to encode config")?;
    let parent = path.parent().context("config path has no parent")?;
    let mut temp = NamedTempFile::new_in(parent).context("failed to create temporary config")?;
    temp.write_all(encoded.as_bytes())?;
    temp.as_file().sync_all()?;
    set_private(temp.path())?;
    temp.persist(path)
        .map_err(|error| error.error)
        .context("failed to replace config")?;
    set_private(path)?;
    Ok(())
}

pub fn validate_profile_id(id: &str) -> Result<()> {
    if id.is_empty()
        || !id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    {
        bail!("profile id may only contain letters, numbers, '-' and '_'");
    }
    Ok(())
}

#[cfg(unix)]
pub fn set_private(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
pub fn set_private(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    fn profile() -> Profile {
        Profile {
            name: "Local gateway".into(),
            enabled: true,
            base_url: "http://127.0.0.1:18080".into(),
            api_format: ApiFormat::Anthropic,
            credential: Credential::Bearer {
                value: "secret-token".into(),
            },
            default_model: "claude-sonnet".into(),
            aliases: RoleModels::default(),
            subagent_model: None,
            fallback_models: vec![],
            enabled_models: vec!["claude-sonnet".into()],
            disabled_models: vec!["claude-opus".into()],
            models: vec![ModelEntry {
                id: "claude-sonnet".into(),
                label: Some("Sonnet".into()),
                description: None,
            }],
        }
    }

    #[test]
    fn round_trips_config() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        update(&path, |config| {
            config.profiles.insert("local".into(), profile());
            Ok(())
        })
        .unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.profiles["local"].default_model, "claude-sonnet");
        assert_eq!(loaded.profiles["local"].enabled_models, ["claude-sonnet"]);
        assert_eq!(loaded.profiles["local"].disabled_models, ["claude-opus"]);
        assert!(!std::fs::read_to_string(path).unwrap().is_empty());
    }

    #[test]
    fn legacy_profiles_default_to_enabled_and_disabled_state_persists() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        fs::write(
            &path,
            r#"version = 2

[profiles.legacy]
name = "Legacy"
base_url = "https://example.com"
default_model = "model-a"
"#,
        )
        .unwrap();
        assert!(load(&path).unwrap().profiles["legacy"].enabled);

        let updated = update(&path, |config| {
            config.profiles.get_mut("legacy").unwrap().enabled = false;
            Ok(())
        })
        .unwrap();
        assert!(!updated.profiles["legacy"].enabled);
        assert!(!load(&path).unwrap().profiles["legacy"].enabled);
        assert!(
            fs::read_to_string(path)
                .unwrap()
                .contains("enabled = false")
        );
    }

    #[test]
    fn load_repairs_duplicate_base_and_1m_catalog_entries() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(
            &path,
            r#"version = 2

[profiles.local]
name = "Local"
base_url = "http://localhost:8080"
default_model = "model-a[1m]"

[[profiles.local.models]]
id = "model-a[1m]"
label = "Model A 1M"

[[profiles.local.models]]
id = "model-a"
description = "Base model"
"#,
        )
        .unwrap();

        let loaded = load(&path).unwrap();
        let models = &loaded.profiles["local"].models;
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "model-a");
        assert_eq!(models[0].label.as_deref(), Some("Model A 1M"));
        assert_eq!(models[0].description.as_deref(), Some("Base model"));
        assert_eq!(loaded.profiles["local"].default_model, "model-a[1m]");
    }

    #[test]
    fn masks_secrets() {
        assert_eq!(mask_secret("123456789"), "••••••");
        assert_eq!(mask_secret("tiny"), "••••••");
    }

    #[test]
    fn migrates_v1_credentials_and_protocol() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        fs::write(
            &path,
            r#"version = 1
[profiles.old]
name = "Old"
base_url = "https://example.com"
default_model = "model"
[profiles.old.credential]
kind = "api-key"
value = "secret"
"#,
        )
        .unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.version, 2);
        assert_eq!(loaded.profiles["old"].api_format, ApiFormat::Anthropic);
        assert!(matches!(
            loaded.profiles["old"].credential,
            Credential::XApiKey { .. }
        ));
    }

    #[test]
    fn concurrent_updates_preserve_different_profiles() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        let left = path.clone();
        let right = path.clone();
        let a = thread::spawn(move || {
            update(&left, |config| {
                config.profiles.insert("a".into(), profile());
                Ok(())
            })
            .unwrap();
        });
        let b = thread::spawn(move || {
            update(&right, |config| {
                config.profiles.insert("b".into(), profile());
                Ok(())
            })
            .unwrap();
        });
        a.join().unwrap();
        b.join().unwrap();
        let loaded = load(&path).unwrap();
        assert!(loaded.profiles.contains_key("a"));
        assert!(loaded.profiles.contains_key("b"));
    }
    #[test]
    fn three_way_merge_preserves_independent_fields_and_rejects_conflicts() {
        let original = profile();
        let mut edited = original.clone();
        edited.default_model = "new-model".into();
        let mut latest = original.clone();
        latest.base_url = "https://changed.example".into();
        latest.aliases.haiku = Some("worker".into());
        let merged = merge_profile(&original, &edited, &latest).unwrap();
        assert_eq!(merged.default_model, "new-model");
        assert_eq!(merged.base_url, latest.base_url);
        assert_eq!(merged.aliases.haiku, latest.aliases.haiku);
        latest.default_model = "another-model".into();
        assert!(merge_profile(&original, &edited, &latest).is_err());
    }
    #[test]
    fn interactive_update_reports_busy_without_waiting_for_the_lock() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path.with_extension("toml.lock"))
            .unwrap();
        lock.lock_exclusive().unwrap();
        let error = try_update(&path, |_| Ok(())).unwrap_err();
        assert!(error.to_string().contains("configuration is busy"));
        assert!(!path.exists());
    }
}
