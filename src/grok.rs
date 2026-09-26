//! Grok Build native configuration and conservative field ownership.
pub mod auth;
pub mod usage;
use crate::config::{self, ApiFormat, AppPaths, Credential, ModelEntry, Profile};
use anyhow::{Context, Result, bail};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use toml_edit::{DocumentMut, Item, Table};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preferences {
    pub default: Option<String>,
    pub web_search: Option<String>,
    pub fork_secondary_model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub permission_mode: Option<String>,
    pub compact_mode: Option<bool>,
    pub show_thinking_blocks: Option<bool>,
}
impl Preferences {
    pub fn validate(&self) -> Result<()> {
        if self
            .permission_mode
            .as_deref()
            .is_some_and(|v| !["default", "ask", "auto", "always-approve"].contains(&v))
        {
            bail!("Invalid Grok permission mode");
        }
        Ok(())
    }
    fn fields(&self) -> Vec<(Vec<String>, toml::Value)> {
        let mut result = Vec::new();
        for (table, key, value) in [
            ("models", "default", &self.default),
            ("models", "web_search", &self.web_search),
            ("ui", "fork_secondary_model", &self.fork_secondary_model),
            ("models", "default_reasoning_effort", &self.reasoning_effort),
            ("ui", "permission_mode", &self.permission_mode),
        ] {
            if let Some(value) = value {
                result.push((
                    vec![table.into(), key.into()],
                    toml::Value::String(value.clone()),
                ));
            }
        }
        for (key, value) in [
            ("compact_mode", self.compact_mode),
            ("show_thinking_blocks", self.show_thinking_blocks),
        ] {
            if let Some(value) = value {
                result.push((vec!["ui".into(), key.into()], toml::Value::Boolean(value)));
            }
        }
        result
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
    #[serde(default)]
    pub preferences: Preferences,
    /// CCSW provider/model route to the original Grok catalog key.
    #[serde(default)]
    pub imports: BTreeMap<String, String>,
}
pub fn home() -> Result<PathBuf> {
    crate::platform::override_path("GROK_HOME", || Ok(crate::platform::home()?.join(".grok")))
}
fn route(provider: &str, model: &str) -> String {
    format!("{provider}::{model}")
}
pub fn model_key(settings: &Settings, provider: &str, model: &str) -> String {
    settings
        .imports
        .get(&route(provider, model))
        .cloned()
        .unwrap_or_else(|| format!("ccsw::{provider}::{model}"))
}
fn read(path: &Path) -> Result<String> {
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        bail!("Refusing symlink Grok file");
    }
    match fs::read_to_string(path) {
        Ok(v) => Ok(v),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e.into()),
    }
}
fn document(path: &Path) -> Result<DocumentMut> {
    read(path)?
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid Grok config.toml; fix TOML syntax before retrying"))
}
fn value(doc: &DocumentMut, path: &[String]) -> Result<Option<toml::Value>> {
    let raw: toml::Value = toml::from_str(&doc.to_string())?;
    let mut current = &raw;
    for part in path {
        let Some(next) = current.get(part) else {
            return Ok(None);
        };
        current = next;
    }
    Ok(Some(current.clone()))
}
fn put(item: &mut Item, path: &[String], value: Option<&toml::Value>) -> Result<()> {
    let table = item
        .as_table_like_mut()
        .context("Grok configuration section must be a table")?;
    if path.len() == 1 {
        if let Some(value) = value {
            let doc: DocumentMut = format!("v = {}", value).parse()?;
            let mut replacement = doc["v"].clone();
            if let (Some(old), Some(new)) = (
                table.get(&path[0]).and_then(Item::as_value),
                replacement.as_value_mut(),
            ) {
                *new.decor_mut() = old.decor().clone();
            }
            table.insert(&path[0], replacement);
        } else {
            table.remove(&path[0]);
        }
    } else {
        if !table.contains_key(&path[0]) {
            if value.is_none() {
                return Ok(());
            }
            let mut section = Table::new();
            section.set_implicit(true);
            table.insert(&path[0], Item::Table(section));
        }
        let child = table.get_mut(&path[0]).context("Missing Grok section")?;
        put(child, &path[1..], value)?;
        if child.as_table_like().is_some_and(|t| t.is_empty()) {
            table.remove(&path[0]);
        }
    }
    Ok(())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Owned {
    path: Vec<String>,
    original: Option<toml::Value>,
    expected: Option<toml::Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Binding {
    home: PathBuf,
    config: PathBuf,
    fields: Vec<Owned>,
}
#[derive(Serialize, Deserialize)]
struct Transaction {
    home: PathBuf,
    config: PathBuf,
    before: String,
    after: String,
    binding: Option<Binding>,
}
fn state(paths: &AppPaths, name: &str) -> PathBuf {
    paths.state_dir.join(format!("grok-{name}.json"))
}
fn binding(paths: &AppPaths) -> Result<Option<Binding>> {
    let path = state(paths, "binding");
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_slice(&fs::read(path)?)?))
}
fn save_binding(paths: &AppPaths, next: Option<&Binding>) -> Result<()> {
    let path = state(paths, "binding");
    if let Some(next) = next {
        crate::codex::atomic_write(&path, &serde_json::to_vec(next)?)?;
    } else if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}
fn lock(paths: &AppPaths, home: &Path) -> Result<Vec<fs::File>> {
    let mut locks = Vec::new();
    for path in [paths.state_dir.join("grok.lock"), home.join(".ccsw.lock")] {
        fs::create_dir_all(path.parent().context("Missing lock directory")?)?;
        if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            bail!("Invalid Grok lock path");
        }
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)?;
        config::set_private(&path)?;
        file.try_lock_exclusive()
            .context("Grok configuration busy; retry")?;
        locks.push(file);
    }
    Ok(locks)
}
fn check_binding(binding: &Binding, paths: &AppPaths, home: &Path) -> Result<()> {
    if !crate::platform::same_path(&binding.home, home)?
        || !crate::platform::same_path(&binding.config, &paths.config)?
    {
        bail!("Grok is connected to another home or CCSW configuration");
    }
    Ok(())
}
fn recover(paths: &AppPaths, home: &Path) -> Result<()> {
    let path = state(paths, "transaction");
    if !path.exists() {
        return Ok(());
    }
    let txn: Transaction = serde_json::from_slice(&fs::read(&path)?)?;
    if !crate::platform::same_path(&txn.home, home)?
        || !crate::platform::same_path(&txn.config, &paths.config)?
    {
        bail!("Interrupted Grok sync belongs to another configuration");
    }
    if let Some(b) = &txn.binding {
        check_binding(b, paths, home)?;
    }
    let current = read(&home.join("config.toml"))?;
    if current == txn.after {
        save_binding(paths, txn.binding.as_ref())?;
    } else if current != txn.before {
        bail!("Grok config changed during interrupted sync; left untouched");
    }
    fs::remove_file(path)?;
    Ok(())
}
fn commit(
    paths: &AppPaths,
    home: &Path,
    before: String,
    after: String,
    next: Option<Binding>,
) -> Result<()> {
    let path = home.join("config.toml");
    if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        bail!("Refusing symlink Grok config");
    }
    let txn = Transaction {
        home: home.to_path_buf(),
        config: paths.config.clone(),
        before,
        after,
        binding: next,
    };
    let journal = state(paths, "transaction");
    crate::codex::atomic_write(&journal, &serde_json::to_vec(&txn)?)?;
    if read(&path)? != txn.before {
        fs::remove_file(journal)?;
        bail!("Grok config changed before write; retry");
    }
    crate::codex::atomic_write(&path, txn.after.as_bytes())?;
    save_binding(paths, txn.binding.as_ref())?;
    fs::remove_file(journal)?;
    Ok(())
}
pub fn connected(paths: &AppPaths) -> bool {
    state(paths, "binding").exists() || state(paths, "transaction").exists()
}
pub fn conflicts(paths: &AppPaths, home: &Path) -> Result<Vec<String>> {
    let _locks = lock(paths, home)?;
    recover(paths, home)?;
    let Some(b) = binding(paths)? else {
        return Ok(vec![]);
    };
    check_binding(&b, paths, home)?;
    let doc = document(&home.join("config.toml"))?;
    b.fields
        .iter()
        .filter_map(|f| match value(&doc, &f.path) {
            Ok(current) if current == f.expected => None,
            Ok(_) => Some(Ok(f.path.join("."))),
            Err(e) => Some(Err(e)),
        })
        .collect()
}
pub fn status(paths: &AppPaths, home: &Path) -> Result<String> {
    let conflicts = conflicts(paths, home)?;
    Ok(if !conflicts.is_empty() {
        format!(
            "Grok external changes: {} · p reconnect",
            conflicts.join(", ")
        )
    } else if connected(paths) {
        format!(
            "Grok connected · {} · restart Grok to load changes",
            home.join("config.toml").display()
        )
    } else {
        format!(
            "Grok not connected · {} · i import · p connect",
            home.join("config.toml").display()
        )
    })
}
/// Config must be loaded in the Grok client scope (profiles are the active list).
pub fn apply(
    paths: &AppPaths,
    home: &Path,
    config: &config::Config,
    preferred: Option<String>,
    reconnect: bool,
) -> Result<Option<String>> {
    let _locks = lock(paths, home)?;
    recover(paths, home)?;
    config.grok.preferences.validate()?;
    let before = read(&home.join("config.toml"))?;
    let mut doc: DocumentMut = before.parse().map_err(|_| {
        anyhow::anyhow!("Invalid Grok config.toml; fix TOML syntax before retrying")
    })?;
    let previous = binding(paths)?;
    if let Some(b) = &previous {
        check_binding(b, paths, home)?;
    }
    let mut originals = BTreeMap::new();
    if let Some(b) = &previous {
        for f in &b.fields {
            let current = value(&doc, &f.path)?;
            if current != f.expected && !reconnect {
                bail!(
                    "Grok field changed externally: {} · press p to reconnect",
                    f.path.join(".")
                );
            }
            if current == f.expected {
                put(doc.as_item_mut(), &f.path, f.original.as_ref())?;
                originals.insert(f.path.clone(), f.original.clone());
            }
        }
    }
    let mut desired = BTreeMap::<Vec<String>, Option<toml::Value>>::new();
    let mut available = Vec::new();
    for (id, profile) in &config.profiles {
        if !profile.enabled {
            continue;
        }
        for model in crate::discovery::active_models(profile, &[]) {
            let key = model_key(&config.grok, id, &model.id);
            available.push(key.clone());
            let mut add = |name: &str, v: toml::Value| {
                desired.insert(vec!["model".into(), key.clone(), name.into()], Some(v));
            };
            for (name, v) in [
                ("model", model.id.as_str()),
                ("base_url", profile.base_url.as_str()),
                ("name", &format!("{} · {}", profile.name, model.label())),
                (
                    "api_backend",
                    match profile.api_format {
                        ApiFormat::Anthropic => "messages",
                        ApiFormat::OpenaiChat => "chat_completions",
                        ApiFormat::OpenaiResponses => "responses",
                    },
                ),
            ] {
                add(name, toml::Value::String(v.into()));
            }
            if let Some(v) = &model.description {
                add("description", toml::Value::String(v.clone()));
            }
            for (name, v) in [
                ("max_completion_tokens", model.max_output_tokens),
                ("context_window", model.context_window),
            ] {
                if let Some(v) = v {
                    add(name, toml::Value::Integer(v.into()));
                }
            }
            if model.id.ends_with("[1m]") {
                add(
                    "model",
                    toml::Value::String(config::canonical_model_id(&model.id).into()),
                );
                add("context_window", toml::Value::Integer(1_000_000));
            }
            // Remove stale imported credential selectors before writing the selected auth.
            for &name in if matches!(profile.credential, Credential::None) {
                &["api_key"][..]
            } else {
                &["api_key", "env_key", "auth_provider"][..]
            } {
                let p = vec!["model".into(), key.clone(), name.into()];
                if value(&doc, &p)?.is_some() {
                    desired.insert(p, None);
                }
            }
            let header_path = vec!["model".into(), key.clone(), "extra_headers".into()];
            let mut headers = value(&doc, &header_path)?
                .and_then(|v| v.as_table().cloned())
                .unwrap_or_default();
            headers.retain(|k, _| {
                !["authorization", "x-api-key", "api-key"]
                    .contains(&k.to_ascii_lowercase().as_str())
            });
            match &profile.credential {
                Credential::Bearer { value } => {
                    desired.insert(
                        vec!["model".into(), key.clone(), "api_key".into()],
                        Some(toml::Value::String(value.clone())),
                    );
                }
                Credential::XApiKey { value } => {
                    headers.insert("x-api-key".into(), toml::Value::String(value.clone()));
                }
                Credential::ApiKey { value } => {
                    headers.insert("api-key".into(), toml::Value::String(value.clone()));
                }
                Credential::None => {}
            }
            if profile.api_format == ApiFormat::Anthropic {
                headers
                    .entry("anthropic-version")
                    .or_insert(toml::Value::String("2023-06-01".into()));
            }
            if !headers.is_empty() || value(&doc, &header_path)?.is_some() {
                desired.insert(header_path, Some(toml::Value::Table(headers)));
            }
        }
    }
    available.sort();
    let requested_default = preferred
        .clone()
        .or_else(|| config.grok.preferences.default.clone());
    let managed_default =
        |key: &str| key.starts_with("ccsw::") || config.grok.imports.values().any(|v| v == key);
    let default = match requested_default {
        Some(key) if managed_default(&key) && !available.contains(&key) => {
            if preferred.is_some() {
                bail!("Selected Grok default is disabled or deleted");
            }
            available.first().cloned()
        }
        other => other,
    };
    for (p, v) in config.grok.preferences.fields() {
        desired.insert(p, Some(v));
    }
    let default_path = vec!["models".into(), "default".into()];
    if let Some(default) = default {
        desired.insert(default_path, Some(toml::Value::String(default)));
    } else {
        desired.remove(&default_path);
        let original = value(&doc, &default_path)?;
        if original
            .as_ref()
            .and_then(|v| v.as_str())
            .is_some_and(managed_default)
        {
            desired.insert(
                default_path,
                available.first().map(|k| toml::Value::String(k.clone())),
            );
        } else if original.is_none()
            && let Some(first) = available.first()
        {
            desired.insert(default_path, Some(toml::Value::String(first.clone())));
        }
    }
    let disabled_path = vec!["models".into(), "disabled_models".into()];
    let mut disabled = value(&doc, &disabled_path)?
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    disabled.retain(|v| !available.iter().any(|k| v.as_str() == Some(k)));
    for key in config
        .grok
        .imports
        .values()
        .filter(|k| !available.contains(k))
    {
        let v = toml::Value::String(key.clone());
        if !disabled.contains(&v) {
            disabled.push(v);
        }
    }
    if !disabled.is_empty() || value(&doc, &disabled_path)?.is_some() {
        desired.insert(disabled_path, Some(toml::Value::Array(disabled)));
    }
    let mut fields = Vec::new();
    for (path, expected) in desired {
        let original = originals.remove(&path).unwrap_or(value(&doc, &path)?);
        put(doc.as_item_mut(), &path, expected.as_ref())?;
        fields.push(Owned {
            path,
            original,
            expected,
        });
    }
    let applied_default = value(&doc, &["models".into(), "default".into()])?
        .and_then(|v| v.as_str().map(str::to_owned));
    commit(
        paths,
        home,
        before,
        doc.to_string(),
        Some(Binding {
            home: home.to_path_buf(),
            config: paths.config.clone(),
            fields,
        }),
    )?;
    Ok(applied_default)
}

/// Verify a native model will inherit the CLI session rather than a local API override.
pub fn validate_oauth_model(home: &Path, model: &str) -> Result<()> {
    let status = auth::status(home)?;
    if !status.saved {
        bail!("Sign in to Grok OAuth first");
    }
    if status.expired && !status.refreshable {
        bail!("Saved Grok OAuth session expired; sign in again");
    }
    if model.trim().is_empty() || model.starts_with("ccsw::") {
        bail!("Choose a native Grok model, e.g. grok-build");
    }
    let doc = document(&home.join("config.toml"))?;
    if doc.get("model").and_then(|v| v.get(model)).is_some() {
        bail!("This model has a local override; choose an unconfigured native Grok model");
    }
    if doc
        .get("endpoints")
        .and_then(|v| v.get("models_base_url"))
        .is_some()
    {
        bail!(
            "A custom catalog endpoint is configured; restore the native endpoint before using OAuth models"
        );
    }
    Ok(())
}

#[derive(Clone)]
pub struct Import {
    pub settings: Settings,
    pub preview: String,
}
pub fn prepare_import(home: &Path, current: &Settings) -> Result<Import> {
    let text = read(&home.join("config.toml"))?;
    let raw: toml::Value = toml::from_str(&text).map_err(|_| {
        anyhow::anyhow!("Invalid Grok config.toml; fix TOML syntax before retrying")
    })?;
    let mut settings = current.clone();
    let mut preview = Vec::new();
    if let Some(models) = raw.get("model").and_then(|v| v.as_table()) {
        for (key, entry) in models {
            if key.starts_with("ccsw::") {
                continue;
            }
            let get = |name: &str| entry.get(name).and_then(|v| v.as_str());
            let Some(url) = get("base_url") else {
                preview.push(format!(
                    "{key}: inherited endpoint; preserved, not imported"
                ));
                continue;
            };
            let backend = match get("api_backend").unwrap_or("chat_completions") {
                "messages" => ApiFormat::Anthropic,
                "responses" => ApiFormat::OpenaiResponses,
                "chat_completions" => ApiFormat::OpenaiChat,
                _ => {
                    preview.push(format!("{key}: unsupported backend; preserved"));
                    continue;
                }
            };
            let model = get("model").unwrap_or(key);
            let headers = entry.get("extra_headers").and_then(|v| v.as_table());
            let header = |name: &str| {
                headers
                    .and_then(|h| h.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)))
                    .and_then(|(_, v)| v.as_str())
            };
            let credential = if let Some(v) = header("x-api-key") {
                Credential::XApiKey { value: v.into() }
            } else if let Some(v) = header("api-key") {
                Credential::ApiKey { value: v.into() }
            } else if let Some(v) = get("api_key").filter(|v| !v.is_empty()) {
                Credential::Bearer { value: v.into() }
            } else if let Some(v) = header("authorization").and_then(|v| v.strip_prefix("Bearer "))
            {
                Credential::Bearer { value: v.into() }
            } else {
                Credential::None
            };
            let existing = settings
                .imports
                .iter()
                .find(|(_, v)| *v == key)
                .and_then(|(r, _)| r.split_once("::").map(|(p, _)| p.to_string()));
            let id = existing
                .or_else(|| {
                    settings
                        .profiles
                        .iter()
                        .find(|(_, p)| {
                            p.base_url == url
                                && p.api_format == backend
                                && p.credential == credential
                        })
                        .map(|(id, _)| id.clone())
                })
                .unwrap_or_else(|| {
                    let mut n = 1;
                    loop {
                        let id = format!("grok-import-{n}");
                        if !settings.profiles.contains_key(&id) {
                            break id;
                        }
                        n += 1;
                    }
                });
            let profile = settings
                .profiles
                .entry(id.clone())
                .or_insert_with(|| Profile {
                    name: get("name").unwrap_or("Imported Grok provider").into(),
                    enabled: true,
                    base_url: url.into(),
                    api_format: backend,
                    credential,
                    default_model: model.into(),
                    aliases: Default::default(),
                    subagent_model: None,
                    fallback_models: vec![],
                    enabled_models: vec![],
                    disabled_models: vec![],
                    models: vec![],
                });
            if settings
                .imports
                .iter()
                .any(|(r, k)| r == &route(&id, model) && k != key)
            {
                preview.push(format!("{key}: duplicate upstream model; preserved"));
                continue;
            }
            let token = |name: &str| {
                entry
                    .get(name)
                    .and_then(|v| v.as_integer())
                    .and_then(|v| u32::try_from(v).ok())
            };
            profile.models.retain(|m| m.id != model);
            profile.models.push(ModelEntry {
                id: model.into(),
                label: get("name").map(|name| {
                    name.strip_prefix(&format!("{} · ", profile.name))
                        .unwrap_or(name)
                        .to_string()
                }),
                description: get("description").map(str::to_string),
                max_output_tokens: token("max_completion_tokens"),
                context_window: token("context_window"),
                reasoning_max: None,
            });
            if !profile.enabled_models.iter().any(|m| m == model) {
                profile.enabled_models.push(model.into());
            }
            if raw
                .get("models")
                .and_then(|v| v.get("disabled_models"))
                .and_then(|v| v.as_array())
                .is_some_and(|a| a.iter().any(|v| v.as_str() == Some(key)))
                && !profile.disabled_models.iter().any(|m| m == model)
            {
                profile.disabled_models.push(model.into());
            }
            settings.imports.insert(route(&id, model), key.clone());
            preview.push(format!(
                "{key} → {id} · {model} · {} · {}",
                url,
                profile.credential.masked()
            ));
        }
    }
    let string = |table: &str, key: &str| {
        raw.get(table)
            .and_then(|v| v.get(key))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };
    let boolean = |key: &str| {
        raw.get("ui")
            .and_then(|v| v.get(key))
            .and_then(|v| v.as_bool())
    };
    settings.preferences = Preferences {
        default: string("models", "default"),
        web_search: string("models", "web_search"),
        fork_secondary_model: string("ui", "fork_secondary_model"),
        reasoning_effort: string("models", "default_reasoning_effort"),
        permission_mode: string("ui", "permission_mode"),
        compact_mode: boolean("compact_mode"),
        show_thinking_blocks: boolean("show_thinking_blocks"),
    };
    settings.preferences.validate()?;
    for p in settings.profiles.values() {
        p.validate()?;
    }
    preview.push("Common client settings:".into());
    for (path, value) in settings.preferences.fields() {
        preview.push(format!("{} = {}", path.join("."), value));
    }
    preview.push("Enter import · Esc cancel".into());
    Ok(Import {
        settings,
        preview: preview.join("\n"),
    })
}

pub fn disconnect(paths: &AppPaths, home: &Path) -> Result<Vec<String>> {
    let _locks = lock(paths, home)?;
    recover(paths, home)?;
    let Some(b) = binding(paths)? else {
        return Ok(vec![]);
    };
    check_binding(&b, paths, home)?;
    let before = read(&home.join("config.toml"))?;
    let mut doc: DocumentMut = before.parse().map_err(|_| {
        anyhow::anyhow!("Invalid Grok config.toml; fix TOML syntax before retrying")
    })?;
    let mut conflicts = Vec::new();
    for f in b.fields {
        if value(&doc, &f.path)? == f.expected {
            put(doc.as_item_mut(), &f.path, f.original.as_ref())?;
        } else {
            conflicts.push(f.path.join("."));
        }
    }
    commit(paths, home, before, doc.to_string(), None)?;
    Ok(conflicts)
}

#[derive(Clone)]
pub struct DetachPlan {
    pub home: PathBuf,
    paths: AppPaths,
    before: String,
}
pub fn prepare_detach(paths: &AppPaths) -> Result<Option<DetachPlan>> {
    if !connected(paths) {
        return Ok(None);
    }
    let home = if state(paths, "transaction").exists() {
        let txn: Transaction = serde_json::from_slice(&fs::read(state(paths, "transaction"))?)?;
        txn.home
    } else {
        binding(paths)?.map(|b| b.home).unwrap_or(home()?)
    };
    crate::uninstall::checked(&home.join("config.toml"))?;
    status(paths, &home)?;
    Ok(Some(DetachPlan {
        before: read(&home.join("config.toml"))?,
        home,
        paths: paths.clone(),
    }))
}
pub fn execute_detach(plan: &DetachPlan) -> Result<()> {
    if read(&plan.home.join("config.toml"))? != plan.before {
        bail!("Grok configuration changed during uninstall; retry");
    }
    disconnect(&plan.paths, &plan.home)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, AppPaths, PathBuf, config::Config) {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths {
            config: temp.path().join("ccsw.toml"),
            state_dir: temp.path().join("state"),
            cache: temp.path().join("cache/models.json"),
        };
        let home = temp.path().join("grok");
        fs::create_dir_all(&home).unwrap();
        let c = config::Config::default();
        (temp, paths, home, c)
    }
    fn profile(format: ApiFormat) -> Profile {
        Profile {
            name: "Test".into(),
            enabled: true,
            base_url: "https://example.invalid/v1".into(),
            api_format: format,
            credential: Credential::Bearer {
                value: "test-secret".into(),
            },
            default_model: "test".into(),
            aliases: Default::default(),
            subagent_model: None,
            fallback_models: vec![],
            enabled_models: vec![],
            disabled_models: vec![],
            models: vec![ModelEntry {
                id: "test".into(),
                label: Some("Test model".into()),
                description: None,
                max_output_tokens: Some(1024),
                context_window: Some(100_000),
                reasoning_max: None,
            }],
        }
    }
    fn raw(home: &Path) -> toml::Value {
        toml::from_str(&fs::read_to_string(home.join("config.toml")).unwrap()).unwrap()
    }
    #[test]
    fn protocols_same_name_defaults_and_disconnect() {
        let (_t, paths, home, mut c) = fixture();
        fs::write(
            home.join("config.toml"),
            "# Keep me\n[ui]\ntheme = 'auto' # existing\n[models]\ndefault = 'grok-4.7'\n",
        )
        .unwrap();
        for (id, format) in [
            ("a", ApiFormat::Anthropic),
            ("b", ApiFormat::OpenaiChat),
            ("c", ApiFormat::OpenaiResponses),
        ] {
            c.profiles.insert(id.into(), profile(format));
        }
        c.profiles.get_mut("a").unwrap().credential = Credential::XApiKey {
            value: "anthropic-secret".into(),
        };
        c.grok.preferences.compact_mode = Some(true);
        apply(&paths, &home, &c, Some("ccsw::b::test".into()), false).unwrap();
        let v = raw(&home);
        assert_eq!(
            v["model"]["ccsw::a::test"]["api_backend"].as_str(),
            Some("messages")
        );
        assert_eq!(
            v["model"]["ccsw::b::test"]["api_backend"].as_str(),
            Some("chat_completions")
        );
        assert_eq!(
            v["model"]["ccsw::c::test"]["api_backend"].as_str(),
            Some("responses")
        );
        assert_eq!(
            v["model"]["ccsw::a::test"]["extra_headers"]["x-api-key"].as_str(),
            Some("anthropic-secret")
        );
        assert_eq!(v["model"]["ccsw::b::test"]["model"].as_str(), Some("test"));
        assert_eq!(
            v["model"]["ccsw::b::test"]["max_completion_tokens"].as_integer(),
            Some(1024)
        );
        assert_eq!(v["models"]["default"].as_str(), Some("ccsw::b::test"));
        assert!(conflicts(&paths, &home).unwrap().is_empty());
        disconnect(&paths, &home).unwrap();
        assert_eq!(raw(&home)["models"]["default"].as_str(), Some("grok-4.7"));
        assert!(raw(&home).get("model").is_none());
        let text = read(&home.join("config.toml")).unwrap();
        assert!(text.contains("# Keep me"));
        assert!(text.contains("# existing"));
    }
    #[test]
    fn imported_model_unknown_fields_env_auth_and_repeat_import() {
        let (_t, paths, home, mut c) = fixture();
        fs::write(home.join("config.toml"), "[models]\ndefault='my.model'\ndefault_reasoning_effort='high'\n[model.'my.model']\nmodel='test'\nbase_url='https://example.invalid/v1'\nenv_key='TEST_KEY'\ncustom='untouched'\n[model.'my.model'.extra_headers]\nX-Tenant='hello'\n").unwrap();
        let candidate = prepare_import(&home, &c.grok).unwrap();
        c.grok = candidate.settings;
        c.profiles = c.grok.profiles.clone();
        c.grok.profiles.clear();
        let mut current = c.grok.clone();
        current.profiles = c.profiles.clone();
        assert_eq!(prepare_import(&home, &current).unwrap().settings, current);
        apply(&paths, &home, &c, None, false).unwrap();
        let v = raw(&home);
        assert_eq!(v["model"]["my.model"]["custom"].as_str(), Some("untouched"));
        assert_eq!(v["model"]["my.model"]["env_key"].as_str(), Some("TEST_KEY"));
        let id = c.profiles.keys().next().unwrap().clone();
        c.profiles.get_mut(&id).unwrap().credential = Credential::Bearer {
            value: "new-secret".into(),
        };
        apply(&paths, &home, &c, None, false).unwrap();
        assert!(raw(&home)["model"]["my.model"].get("env_key").is_none());
        assert_eq!(
            raw(&home)["model"]["my.model"]["api_key"].as_str(),
            Some("new-secret")
        );
        disconnect(&paths, &home).unwrap();
        assert_eq!(
            raw(&home)["model"]["my.model"]["env_key"].as_str(),
            Some("TEST_KEY")
        );
        assert!(raw(&home)["model"]["my.model"].get("api_key").is_none());
    }
    #[test]
    fn external_conflict_reconnect_and_preserve_on_disconnect() {
        let (_t, paths, home, mut c) = fixture();
        c.profiles
            .insert("a".into(), profile(ApiFormat::OpenaiChat));
        apply(&paths, &home, &c, None, false).unwrap();
        let path = home.join("config.toml");
        let text = read(&path).unwrap().replace("Test model", "External model");
        fs::write(&path, &text).unwrap();
        assert!(!conflicts(&paths, &home).unwrap().is_empty());
        assert!(apply(&paths, &home, &c, None, false).is_err());
        assert_eq!(read(&path).unwrap(), text);
        apply(&paths, &home, &c, None, true).unwrap();
        disconnect(&paths, &home).unwrap();
        assert_eq!(
            raw(&home)["model"]["ccsw::a::test"]["name"].as_str(),
            Some("Test · External model")
        );
        apply(&paths, &home, &c, None, false).unwrap();
        fs::write(
            &path,
            read(&path).unwrap().replace("Test model", "Changed again"),
        )
        .unwrap();
        assert!(!disconnect(&paths, &home).unwrap().is_empty());
        assert!(
            raw(&home)["model"]["ccsw::a::test"]["name"]
                .as_str()
                .unwrap()
                .contains("Changed again")
        );
    }
    #[test]
    fn disable_and_delete_models_restore_native_default() {
        let (_t, paths, home, mut c) = fixture();
        fs::write(home.join("config.toml"), "[models]\ndefault='grok-4.7'\n").unwrap();
        c.profiles
            .insert("a".into(), profile(ApiFormat::OpenaiChat));
        c.grok.preferences.default = Some("ccsw::a::test".into());
        apply(&paths, &home, &c, None, false).unwrap();
        c.profiles
            .get_mut("a")
            .unwrap()
            .disabled_models
            .push("test".into());
        apply(&paths, &home, &c, None, false).unwrap();
        assert!(raw(&home).get("model").is_none());
        assert_eq!(raw(&home)["models"]["default"].as_str(), Some("grok-4.7"));
        c.profiles.clear();
        apply(&paths, &home, &c, None, false).unwrap();
        assert_eq!(raw(&home)["models"]["default"].as_str(), Some("grok-4.7"));
    }
    #[test]
    fn interrupted_transaction_recovery_and_invalid_file_no_writes() {
        let (_t, paths, home, mut c) = fixture();
        c.profiles
            .insert("a".into(), profile(ApiFormat::OpenaiChat));
        apply(&paths, &home, &c, None, false).unwrap();
        let after = read(&home.join("config.toml")).unwrap();
        let txn = Transaction {
            home: home.clone(),
            config: paths.config.clone(),
            before: String::new(),
            after,
            binding: binding(&paths).unwrap(),
        };
        fs::remove_file(state(&paths, "binding")).unwrap();
        crate::codex::atomic_write(
            &state(&paths, "transaction"),
            &serde_json::to_vec(&txn).unwrap(),
        )
        .unwrap();
        status(&paths, &home).unwrap();
        assert!(binding(&paths).unwrap().is_some());
        assert!(!state(&paths, "transaction").exists());
        fs::write(home.join("config.toml"), "[broken").unwrap();
        assert!(apply(&paths, &home, &c, None, false).is_err());
        assert_eq!(read(&home.join("config.toml")).unwrap(), "[broken");
    }
    #[test]
    fn imported_disable_reenable_and_removed_auth() {
        let (_t, paths, home, mut c) = fixture();
        fs::write(home.join("config.toml"), "[model.custom]\nmodel='test'\nbase_url='https://example.invalid/v1'\n[model.custom.extra_headers]\nx-api-key='old-key'\nX-Tenant='keep'\n").unwrap();
        let imported = prepare_import(&home, &c.grok).unwrap();
        c.grok = imported.settings;
        c.profiles = std::mem::take(&mut c.grok.profiles);
        let id = c.profiles.keys().next().unwrap().clone();
        apply(&paths, &home, &c, None, false).unwrap();
        c.profiles.get_mut(&id).unwrap().enabled = false;
        apply(&paths, &home, &c, None, false).unwrap();
        assert_eq!(
            raw(&home)["models"]["disabled_models"][0].as_str(),
            Some("custom")
        );
        c.profiles.get_mut(&id).unwrap().enabled = true;
        c.profiles.get_mut(&id).unwrap().credential = Credential::None;
        apply(&paths, &home, &c, None, false).unwrap();
        let v = raw(&home);
        assert!(
            v["model"]["custom"]["extra_headers"]
                .get("x-api-key")
                .is_none()
        );
        assert_eq!(
            v["model"]["custom"]["extra_headers"]["X-Tenant"].as_str(),
            Some("keep")
        );
        assert!(
            v.get("models")
                .and_then(|v| v.get("disabled_models"))
                .is_none()
        );
        disconnect(&paths, &home).unwrap();
        assert_eq!(
            raw(&home)["model"]["custom"]["extra_headers"]["x-api-key"].as_str(),
            Some("old-key")
        );
    }

    #[test]
    #[ignore = "requires CCSW_GROK_BIN pointing to an installed Grok Build CLI"]
    fn installed_grok_accepts_generated_config_and_disabled_catalog_keys() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            sync::{
                Arc,
                atomic::{AtomicBool, Ordering},
            },
            time::Duration,
        };
        let bin = std::env::var("CCSW_GROK_BIN").expect("set CCSW_GROK_BIN");
        let (_t, paths, home, mut c) = fixture();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let server = std::thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                if let Ok((mut stream, _)) = listener.accept() {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut request = [0u8; 4096];
                    let _ = stream.read(&mut request);
                    let body = r#"{"object":"list","data":[]}"#;
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                } else {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        });
        for (id, format) in [
            ("a", ApiFormat::Anthropic),
            ("b", ApiFormat::OpenaiChat),
            ("c", ApiFormat::OpenaiResponses),
        ] {
            let mut p = profile(format);
            p.base_url = url.clone();
            c.profiles.insert(id.into(), p);
        }
        c.grok.preferences = Preferences {
            reasoning_effort: Some("high".into()),
            permission_mode: Some("ask".into()),
            compact_mode: Some(true),
            show_thinking_blocks: Some(true),
            ..Default::default()
        };
        apply(&paths, &home, &c, Some("ccsw::a::test".into()), false).unwrap();
        let run = |command: &str| {
            std::process::Command::new(&bin)
                .arg(command)
                .current_dir(&home)
                .env("GROK_HOME", &home)
                .env("GROK_MODELS_BASE_URL", &url)
                .env("GROK_MODELS_LIST_URL", format!("{url}/models"))
                .env("XAI_API_KEY", "dummy-test-key")
                .env_remove("GROK_CONFIG")
                .env_remove("GROK_CONFIG_PATH")
                .env_remove("GROK_DEFAULT_MODEL")
                .output()
                .unwrap()
        };
        let inspect = run("inspect");
        let models = run("models");
        c.profiles.get_mut("a").unwrap().enabled = false;
        apply(&paths, &home, &c, None, false).unwrap();
        let disabled = run("models");
        stop.store(true, Ordering::Relaxed);
        server.join().unwrap();
        assert!(inspect.status.success());
        assert!(models.status.success());
        assert!(disabled.status.success());
        let list = String::from_utf8_lossy(&models.stdout);
        assert!(list.contains("ccsw::a::test"));
        assert!(list.contains("ccsw::b::test"));
        assert!(list.contains("ccsw::c::test"));
        let list = String::from_utf8_lossy(&disabled.stdout);
        assert!(!list.contains("ccsw::a::test"));
        assert!(list.contains("ccsw::b::test"));
    }
    #[test]
    fn invalid_native_toml_does_not_expose_credentials_in_errors() {
        let (_t, paths, home, c) = fixture();
        fs::write(
            home.join("config.toml"),
            "[model.test]\napi_key='private-sensitive-value\n",
        )
        .unwrap();
        let import_error = prepare_import(&home, &c.grok).err().unwrap();
        let sync_error = apply(&paths, &home, &c, None, false).unwrap_err();
        assert!(!format!("{import_error:#}").contains("private-sensitive-value"));
        assert!(!format!("{sync_error:#}").contains("private-sensitive-value"));
    }
}
