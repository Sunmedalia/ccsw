//! Native Pi configuration. OAuth credentials are never modified.
pub mod native;
use crate::config::{self, ApiFormat, AppPaths, Credential, ModelEntry, Profile};
use anyhow::{Context, Result, bail};
use clap::Subcommand;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
    #[serde(default)]
    pub imports: BTreeMap<String, String>,
    /// JSON strings preserve Pi-only compatibility options without TOML null loss.
    #[serde(default)]
    pub extras: BTreeMap<String, String>,
}
#[derive(Subcommand)]
pub enum Command {
    /// Inspect native Pi files without importing them into CCSW.
    Files,
    Import {
        #[arg(long)]
        dry_run: bool,
    },
    Apply {
        #[arg(long)]
        profile: String,
        #[arg(long)]
        model: Option<String>,
    },
    Status,
    Disconnect,
}
pub fn run(paths: &AppPaths, command: Command) -> Result<()> {
    let message = match command {
        Command::Files => {
            let home = home()?;
            native::description(&home, &native::load(&home)?)
        }
        Command::Import { dry_run } => import(paths, dry_run)?,
        Command::Apply { profile, model } => {
            apply(paths, &profile, model.as_deref())?;
            "Pi models synced directly to providers; open /model in Pi.".into()
        }
        Command::Status => status(paths)?,
        Command::Disconnect => {
            disconnect(paths)?;
            "Pi managed settings restored.".into()
        }
    };
    println!("{message}");
    Ok(())
}
pub fn home() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("PI_CODING_AGENT_DIR") {
        let path = PathBuf::from(path);
        if let Ok(rest) = path.strip_prefix("~") {
            return Ok(PathBuf::from(std::env::var_os("HOME").context("HOME missing")?).join(rest));
        }
        return Ok(std::path::absolute(path)?);
    }
    Ok(PathBuf::from(
        std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .context("Home directory missing")?,
    )
    .join(".pi/agent"))
}
fn read(path: &Path) -> Result<Value> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let value: Value =
        serde_json::from_slice(&fs::read(path)?).context("Invalid Pi JSON configuration")?;
    if !value.is_object() {
        bail!("Pi configuration must be an object");
    }
    Ok(value)
}
fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("Missing parent directory")?;
    fs::create_dir_all(parent)?;
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        bail!("Refusing symlink target");
    }
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    config::set_private(temp.path())?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
fn save(path: &Path, value: &impl Serialize) -> Result<()> {
    write(path, &serde_json::to_vec_pretty(value)?)
}
fn lock(home: &Path) -> Result<std::fs::File> {
    fs::create_dir_all(home)?;
    let path = home.join(".ccsw-pi.lock");
    if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        bail!("Invalid Pi lock");
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)?;
    config::set_private(&path)?;
    file.try_lock_exclusive()
        .context("Pi configuration busy; retry")?;
    Ok(file)
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Binding {
    pub home: PathBuf,
    config: PathBuf,
    profile: String,
    model: String,
    original: [Value; 2],
    expected: [Value; 2],
    keys: Vec<String>,
    source: String,
}
fn binding_path(paths: &AppPaths) -> PathBuf {
    paths.state_dir.join("pi-binding.json")
}
fn binding(paths: &AppPaths) -> Result<Option<Binding>> {
    let path = binding_path(paths);
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(
        serde_json::from_slice(&fs::read(path)?).context("Invalid Pi binding")?,
    ))
}
fn documents(home: &Path) -> Result<[Value; 2]> {
    Ok([
        read(&home.join("models.json"))?,
        read(&home.join("settings.json"))?,
    ])
}
fn check(b: &Binding, paths: &AppPaths, home: &Path, docs: &[Value; 2]) -> Result<()> {
    if b.home != home
        || b.config
            != fs::canonicalize(&paths.config).or_else(|_| std::path::absolute(&paths.config))?
    {
        bail!("Pi target changed; disconnect the original target first");
    }
    for key in &b.keys {
        if docs[0]["providers"].get(key) != b.expected[0]["providers"].get(key) {
            bail!("Pi provider was externally edited; disconnect before retrying");
        }
    }
    for key in ["defaultProvider", "defaultModel"] {
        if docs[1].get(key) != b.expected[1].get(key) {
            bail!("Pi default was externally edited; disconnect before retrying");
        }
    }
    Ok(())
}
fn restore_value(target: &mut Value, source: &Value, key: &str) {
    if let Some(v) = source.get(key) {
        target[key] = v.clone();
    } else if let Some(map) = target.as_object_mut() {
        map.remove(key);
    }
}
fn restored(b: &Binding, mut docs: [Value; 2]) -> [Value; 2] {
    for key in &b.keys {
        if docs[0]["providers"].get(key) == b.expected[0]["providers"].get(key) {
            restore_value(&mut docs[0]["providers"], &b.original[0]["providers"], key);
        }
    }
    if docs[0]["providers"]
        .as_object()
        .is_some_and(|v| v.is_empty())
        && b.original[0].get("providers").is_none()
    {
        docs[0].as_object_mut().unwrap().remove("providers");
    }
    for key in ["defaultProvider", "defaultModel"] {
        if docs[1].get(key) == b.expected[1].get(key) {
            restore_value(&mut docs[1], &b.original[1], key);
        }
    }
    docs
}
// Journal contains exact pre-operation bytes for rollback, including the binding.
#[derive(Serialize, Deserialize)]
struct Transaction {
    home: PathBuf,
    before: [Option<Vec<u8>>; 3],
    after: [Option<Vec<u8>>; 3],
}
fn transaction_paths(paths: &AppPaths, home: &Path) -> [PathBuf; 3] {
    [
        home.join("models.json"),
        home.join("settings.json"),
        binding_path(paths),
    ]
}
fn recover(paths: &AppPaths, home: &Path) -> Result<()> {
    let journal = paths.state_dir.join("pi-transaction.json");
    if !journal.exists() {
        return Ok(());
    }
    let tx: Transaction = serde_json::from_slice(&fs::read(&journal)?)?;
    if tx.home != home {
        bail!("Interrupted Pi transaction belongs to another target");
    }
    let targets = transaction_paths(paths, home);
    for (i, path) in targets.iter().enumerate() {
        let current = match fs::read(path) {
            Ok(v) => Some(v),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        if current != tx.before[i] && current != tx.after[i] {
            bail!(
                "Pi file changed after interrupted transaction; preserve the edit before recovering"
            );
        }
    }
    for (path, before) in targets.iter().zip(tx.before.iter()) {
        if let Some(bytes) = before {
            write(path, bytes)?;
        } else if path.exists() {
            fs::remove_file(path)?;
        }
    }
    fs::remove_file(journal)?;
    Ok(())
}
fn commit(paths: &AppPaths, home: &Path, docs: &[Value; 2], state: Option<&Binding>) -> Result<()> {
    let targets = transaction_paths(paths, home);
    let mut before = [None, None, None];
    for (i, target) in targets.iter().enumerate() {
        before[i] = match fs::read(target) {
            Ok(v) => Some(v),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
    }
    let journal = paths.state_dir.join("pi-transaction.json");
    save(
        &journal,
        &Transaction {
            home: home.into(),
            before,
            after: [
                Some(serde_json::to_vec_pretty(&docs[0])?),
                Some(serde_json::to_vec_pretty(&docs[1])?),
                state.map(serde_json::to_vec_pretty).transpose()?,
            ],
        },
    )?;
    let result = (|| -> Result<()> {
        save(&targets[0], &docs[0])?;
        save(&targets[1], &docs[1])?;
        if let Some(state) = state {
            save(&targets[2], state)?;
        } else if targets[2].exists() {
            fs::remove_file(&targets[2])?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        recover(paths, home).context("Pi rollback failed; retry disconnect")?;
        return Err(error);
    }
    fs::remove_file(journal)?;
    Ok(())
}
fn literal(value: &str) -> String {
    let value = value.replace('$', "$$");
    if let Some(rest) = value.strip_prefix('!') {
        format!("$!{rest}")
    } else {
        value
    }
}
pub fn apply(paths: &AppPaths, profile_id: &str, model: Option<&str>) -> Result<()> {
    let _state_lock = state_lock(paths)?;
    apply_locked(paths, profile_id, model)
}
fn apply_locked(paths: &AppPaths, profile_id: &str, model: Option<&str>) -> Result<()> {
    let home = home()?;
    let _lock = lock(&home)?;
    recover(paths, &home)?;
    let config = config::update_client(&paths.config, config::Client::Pi, |_| Ok(()))?;
    let wanted = if profile_id.is_empty() {
        ""
    } else {
        let selected = config
            .profiles
            .get(profile_id)
            .context("Provider missing")?;
        if !selected.enabled {
            bail!("Provider disabled");
        }
        let wanted = model.unwrap_or(&selected.default_model);
        if !crate::discovery::active_models(selected, &[])
            .iter()
            .any(|m| strip_1m(&m.id) == strip_1m(wanted))
        {
            bail!("Select an enabled model");
        }
        wanted
    };
    let mut docs = documents(&home)?;
    let old = binding(paths)?;
    if let Some(b) = &old {
        check(b, paths, &home, &docs)?;
    }
    let original = old
        .as_ref()
        .map(|b| b.original.clone())
        .unwrap_or_else(|| docs.clone());
    if let Some(b) = &old {
        docs = restored(b, docs);
    }
    if docs[0].get("providers").is_none() {
        docs[0]["providers"] = json!({});
    }
    if !docs[0]["providers"].is_object() {
        bail!("Pi providers must be an object");
    }
    let mut keys = Vec::new();
    for (id, profile) in config.profiles.iter().filter(|(_, p)| p.enabled) {
        let models = crate::discovery::active_models(profile, &[]);
        if models.is_empty() {
            continue;
        }
        let key = format!("ccsw-{id}");
        if docs[0]["providers"].get(&key).is_some() {
            bail!("Pi provider name collision: {key}");
        }
        let mut provider: Value = config
            .pi
            .extras
            .get(id)
            .map(|s| serde_json::from_str(s))
            .transpose()?
            .unwrap_or(json!({}));
        if !provider.is_object() || provider.get("headers").is_some_and(|v| !v.is_object()) {
            bail!("Invalid Pi compatibility configuration");
        }
        let old_models = provider["models"].as_array().cloned().unwrap_or_default();
        provider["models"] = Value::Array(
            models
                .iter()
                .map(|m| {
                    let id = strip_1m(&m.id);
                    let mut value = old_models
                        .iter()
                        .find(|v| v["id"] == id)
                        .cloned()
                        .unwrap_or(json!({}));
                    value["id"] = json!(id);
                    value["name"] = json!(m.label());
                    value.as_object_mut().unwrap().remove("api");
                    for (field, number) in [
                        (
                            "contextWindow",
                            m.context_window
                                .or_else(|| m.id.ends_with("[1m]").then_some(1_000_000)),
                        ),
                        ("maxTokens", m.max_output_tokens),
                    ] {
                        if let Some(n) = number {
                            value[field] = json!(n);
                        } else {
                            value.as_object_mut().unwrap().remove(field);
                        }
                    }
                    value
                })
                .collect(),
        );
        provider["baseUrl"] = json!(native_base_url(profile, &provider)?);
        provider
            .as_object_mut()
            .unwrap()
            .remove("ccswNativeBaseUrl");
        provider.as_object_mut().unwrap().remove("ccswNativeApi");
        provider["api"] = json!(match profile.api_format {
            ApiFormat::Anthropic => "anthropic-messages",
            ApiFormat::OpenaiChat => "openai-completions",
            ApiFormat::OpenaiResponses => "openai-responses",
        });
        provider["apiKey"] = json!(literal(
            profile.credential.value().unwrap_or("ccsw-keyless")
        ));
        provider.as_object_mut().unwrap().remove("authHeader");
        match &profile.credential {
            Credential::Bearer { .. } => provider["authHeader"] = json!(true),
            Credential::XApiKey { value } | Credential::ApiKey { value } => {
                if provider.get("headers").is_none() {
                    provider["headers"] = json!({});
                }
                let header = if matches!(profile.credential, Credential::XApiKey { .. }) {
                    "x-api-key"
                } else {
                    "api-key"
                };
                provider["headers"][header] = json!(literal(value));
            }
            Credential::None => {}
        }
        docs[0]["providers"][&key] = provider;
        keys.push(key);
    }
    if !profile_id.is_empty() {
        docs[1]["defaultProvider"] = json!(format!("ccsw-{profile_id}"));
        docs[1]["defaultModel"] = json!(strip_1m(wanted));
    }
    let state = Binding {
        home: home.clone(),
        config: fs::canonicalize(&paths.config).or_else(|_| std::path::absolute(&paths.config))?,
        profile: profile_id.into(),
        model: wanted.into(),
        original,
        expected: docs.clone(),
        keys,
        source: source_hash(&config)?,
    };
    commit(paths, &home, &docs, Some(&state))
}
pub fn sync_if_connected(paths: &AppPaths) -> Result<()> {
    if !binding_path(paths).exists() {
        return Ok(());
    }
    let _state_lock = state_lock(paths)?;
    let Some(b) = binding(paths)? else {
        return Ok(());
    };
    let config = config::load_client(&paths.config, config::Client::Pi)?;
    let choices: Vec<_> = config
        .profiles
        .iter()
        .filter_map(|(id, p)| {
            let models = crate::discovery::active_models(p, &[]);
            (!models.is_empty()).then_some((id, p, models))
        })
        .collect();
    let selected = choices
        .iter()
        .find(|(id, _, _)| **id == b.profile)
        .or_else(|| choices.first());
    if let Some((id, p, models)) = selected {
        let model = models
            .iter()
            .find(|m| strip_1m(&m.id) == strip_1m(&b.model) && **id == b.profile)
            .or_else(|| {
                models
                    .iter()
                    .find(|m| strip_1m(&m.id) == strip_1m(&p.default_model))
            })
            .unwrap_or(&models[0]);
        apply_locked(paths, id, Some(&model.id))?;
    } else {
        apply_locked(paths, "", None)?;
    }
    Ok(())
}
pub fn status(paths: &AppPaths) -> Result<String> {
    let home = home()?;
    if paths.state_dir.join("pi-transaction.json").exists() {
        return Ok("Pi interrupted transaction; retry apply or disconnect".into());
    }
    let Some(b) = binding(paths)? else {
        return Ok("Pi not connected · p sync".into());
    };
    if check(&b, paths, &home, &documents(&home)?).is_ok()
        && source_hash(&config::load_client(&paths.config, config::Client::Pi)?)? != b.source
    {
        return Ok("Pi pending sync · p retry".into());
    }
    match check(&b, paths, &home, &documents(&home)?) {
        Ok(()) => Ok(format!(
            "Pi connected · {} / {} · direct API · p resync",
            b.profile, b.model
        )),
        Err(_) => Ok("Pi conflict: target or managed configuration changed · D disconnect".into()),
    }
}
pub fn disconnect(paths: &AppPaths) -> Result<()> {
    let _state_lock = state_lock(paths)?;
    let target = binding(paths)?.map(|b| b.home).unwrap_or(home()?);
    let _lock = lock(&target)?;
    recover(paths, &target)?;
    let Some(b) = binding(paths)? else {
        return Ok(());
    };
    let docs = restored(&b, documents(&b.home)?);
    commit(paths, &b.home, &docs, None)
}
pub fn import(paths: &AppPaths, dry_run: bool) -> Result<String> {
    let home = home()?;
    let source = read(&home.join("models.json"))?;
    let auth = read(&home.join("auth.json"))?;
    let pi_settings = read(&home.join("settings.json"))?;
    let Some(providers) = source.get("providers").and_then(Value::as_object) else {
        return Ok("No custom Pi providers to import".into());
    };
    let mut reports = Vec::new();
    let edit = |config: &mut config::Config| -> Result<()> {
        for (name, provider) in providers {
            if name.starts_with("ccsw-") {
                continue;
            }
            let parsed = parse_provider(name, provider, &auth);
            let Ok(mut profile) = parsed else {
                reports.push(format!("Skipped {name}: {}", parsed.unwrap_err()));
                continue;
            };
            if pi_settings["defaultProvider"] == *name
                && let Some(model) = pi_settings["defaultModel"].as_str()
                && profile.models.iter().any(|m| m.id == model)
            {
                profile.default_model = model.into();
            }
            let source_key = format!("{}::{name}", home.display());
            let id = if let Some(id) = config.pi.imports.get(&source_key) {
                id.clone()
            } else {
                let base = format!(
                    "pi-{}",
                    name.chars()
                        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                            c
                        } else {
                            '-'
                        })
                        .collect::<String>()
                );
                let mut id = base.clone();
                let mut n = 2;
                while config.profiles.contains_key(&id) {
                    id = format!("{base}-{n}");
                    n += 1;
                }
                id
            };
            config::validate_profile_id(&id)?;
            let mut extras = provider.clone();
            extras
                .as_object_mut()
                .unwrap()
                .retain(|k, _| matches!(k.as_str(), "compat" | "headers" | "models"));
            if let Some(headers) = extras.get_mut("headers").and_then(Value::as_object_mut) {
                headers.retain(|k, _| {
                    !["authorization", "x-api-key", "api-key"]
                        .contains(&k.to_ascii_lowercase().as_str())
                });
            }
            for field in ["apiKey", "baseUrl", "api", "oauth", "authHeader"] {
                extras.as_object_mut().unwrap().remove(field);
            }
            extras["ccswNativeBaseUrl"] = json!(profile.base_url);
            extras["ccswNativeApi"] = json!(profile.api_format);
            config
                .pi
                .extras
                .insert(id.clone(), serde_json::to_string(&extras)?);
            config.pi.imports.insert(source_key, id.clone());
            config.profiles.insert(id.clone(), profile);
            reports.push(format!("Imported {name} as {id}"));
        }
        Ok(())
    };
    if dry_run {
        let mut edit = edit;
        edit(&mut config::load_client(&paths.config, config::Client::Pi)?)?;
    } else {
        config::update_client(&paths.config, config::Client::Pi, edit)?;
        sync_if_connected(paths).context(
            "Pi import saved, but synchronization failed; resolve conflict and retry apply",
        )?;
    }
    Ok(format!(
        "{}{}",
        if dry_run { "Preview only\n" } else { "" },
        reports.join("\n")
    ))
}
fn parse_provider(name: &str, value: &Value, auth: &Value) -> Result<Profile> {
    let base_url = value["baseUrl"]
        .as_str()
        .context("built-in override or missing endpoint")?;
    if value.get("oauth").is_some() {
        bail!("OAuth provider is not an API-key provider");
    }
    if value.get("headers").is_some_and(|v| !v.is_object()) {
        bail!("headers must be an object");
    }
    let models = value["models"]
        .as_array()
        .context("missing custom model list")?;
    let api = value["api"]
        .as_str()
        .or_else(|| models.first()?.get("api")?.as_str())
        .context("missing API protocol")?;
    let api_format = match api {
        "anthropic-messages" => ApiFormat::Anthropic,
        "openai-completions" => ApiFormat::OpenaiChat,
        "openai-responses" => ApiFormat::OpenaiResponses,
        _ => bail!("unsupported API protocol"),
    };
    let raw_key = value["apiKey"].as_str().or_else(|| {
        (auth[name]["type"] == "api_key")
            .then(|| auth[name]["key"].as_str())
            .flatten()
    });
    fn dynamic(v: &Value) -> bool {
        match v {
            Value::String(s) => s.starts_with('!') || s.contains('$'),
            Value::Object(m) => m.values().any(dynamic),
            Value::Array(a) => a.iter().any(dynamic),
            _ => false,
        }
    }
    if raw_key.is_some_and(|s| s.starts_with('!') || s.contains('$'))
        || value.get("headers").is_some_and(dynamic)
    {
        bail!("command/environment credentials require manual configuration");
    }
    if raw_key.is_none() && auth[name]["type"] == "oauth" {
        bail!("OAuth credentials are not imported");
    }
    let mut entries = Vec::new();
    for m in models {
        if m["api"].as_str().is_some_and(|v| v != api) {
            bail!("mixed per-model protocols require separate providers");
        }
        if m.get("apiKey").is_some() || m.get("baseUrl").is_some() || m.get("headers").is_some() {
            bail!("per-model endpoint/authentication requires a separate provider");
        }
        for field in ["contextWindow", "maxTokens"] {
            if m.get(field).is_some_and(|v| v.as_u64().is_none()) {
                bail!("token limits must be positive integers");
            }
        }
        let entry = ModelEntry {
            id: m["id"].as_str().context("model ID missing")?.into(),
            label: m["name"].as_str().map(String::from),
            description: None,
            context_window: m["contextWindow"].as_u64().map(u32::try_from).transpose()?,
            max_output_tokens: m["maxTokens"].as_u64().map(u32::try_from).transpose()?,
        };
        entry.validate()?;
        entries.push(entry);
    }
    let default_model = entries.first().context("empty model list")?.id.clone();
    let explicit = value
        .get("headers")
        .and_then(Value::as_object)
        .and_then(|h| {
            h.iter().find(|(k, _)| {
                ["authorization", "x-api-key", "api-key"].contains(&k.to_ascii_lowercase().as_str())
            })
        });
    let credential = if let Some((header, v)) = explicit {
        let key = v.as_str().context("auth header must be a literal string")?;
        match header.to_ascii_lowercase().as_str() {
            "authorization" => Credential::Bearer {
                value: key
                    .strip_prefix("Bearer ")
                    .context("only Bearer authorization headers can be imported")?
                    .into(),
            },
            "x-api-key" => Credential::XApiKey { value: key.into() },
            _ => Credential::ApiKey { value: key.into() },
        }
    } else {
        match raw_key {
            Some(key) if api_format == ApiFormat::Anthropic && value["authHeader"] != true => {
                Credential::XApiKey { value: key.into() }
            }
            Some(value) => Credential::Bearer {
                value: value.into(),
            },
            None => Credential::None,
        }
    };
    let profile = Profile {
        name: value["name"].as_str().unwrap_or(name).into(),
        enabled: true,
        base_url: base_url.into(),
        api_format,
        credential,
        default_model,
        aliases: Default::default(),
        subagent_model: None,
        fallback_models: Vec::new(),
        enabled_models: entries.iter().map(|m| m.id.clone()).collect(),
        disabled_models: Vec::new(),
        models: entries,
    };
    profile
        .validate()
        .map_err(|_| anyhow::anyhow!("invalid provider or model configuration"))?;
    Ok(profile)
}

fn strip_1m(model: &str) -> &str {
    model.strip_suffix("[1m]").unwrap_or(model)
}
pub(crate) struct DetachPlan {
    pub home: PathBuf,
    before: [Value; 2],
    after: [Value; 2],
}
pub(crate) fn prepare_detach(paths: &AppPaths) -> Result<Option<DetachPlan>> {
    if paths.state_dir.join("pi-transaction.json").exists() {
        bail!("Recover interrupted Pi transaction with pi disconnect before uninstall");
    }
    let Some(b) = binding(paths)? else {
        return Ok(None);
    };
    crate::uninstall::checked(&b.home)?;
    for name in ["models.json", "settings.json", ".ccsw-pi.lock"] {
        crate::uninstall::checked(&b.home.join(name))?;
    }
    if b.config
        != fs::canonicalize(&paths.config).or_else(|_| std::path::absolute(&paths.config))?
    {
        bail!("Pi binding belongs to another CCSW configuration");
    }
    let before = documents(&b.home)?;
    let after = restored(&b, before.clone());
    Ok(Some(DetachPlan {
        home: b.home,
        before,
        after,
    }))
}
pub(crate) fn execute_detach(plan: &DetachPlan) -> Result<()> {
    if documents(&plan.home)? != plan.before {
        bail!("Pi files changed during uninstall");
    }
    for (i, name) in ["models.json", "settings.json"].iter().enumerate() {
        if plan.before[i] != plan.after[i] {
            save(&plan.home.join(name), &plan.after[i])?;
        }
    }
    Ok(())
}

fn source_hash(config: &config::Config) -> Result<String> {
    use sha2::{Digest, Sha256};
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(
            &config.profiles,
            &config.pi.imports,
            &config.pi.extras
        ))?)
    ))
}

fn state_lock(paths: &AppPaths) -> Result<std::fs::File> {
    fs::create_dir_all(&paths.state_dir)?;
    let path = paths.state_dir.join("pi.lock");
    if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        bail!("Invalid Pi state lock");
    }
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)?;
    config::set_private(&path)?;
    file.try_lock_exclusive()
        .context("Pi operation busy; retry")?;
    Ok(file)
}

fn native_base_url(profile: &Profile, extras: &Value) -> Result<String> {
    if extras["ccswNativeBaseUrl"] == profile.base_url
        && extras["ccswNativeApi"] == json!(profile.api_format)
    {
        return Ok(profile.base_url.clone());
    }
    let mut url = crate::proxy::models_endpoint(&profile.base_url, profile.api_format)?;
    let mut root = url
        .path()
        .strip_suffix("/models")
        .context("Invalid API endpoint")?
        .to_owned();
    if profile.api_format == ApiFormat::Anthropic {
        root = root.strip_suffix("/v1").unwrap_or(&root).to_owned();
    }
    url.set_path(&root);
    Ok(url.to_string().trim_end_matches('/').to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_preserves_external_edits_and_restores_exact_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths {
            config: temp.path().join("config.toml"),
            state_dir: temp.path().join("state"),
            cache: temp.path().join("cache"),
        };
        let home = temp.path().join("pi");
        fs::create_dir(&home).unwrap();
        let before = b"{ \"theme\": \"before\" }\n".to_vec();
        let after = b"{\"theme\":\"after\"}".to_vec();
        fs::write(home.join("settings.json"), &after).unwrap();
        let journal = paths.state_dir.join("pi-transaction.json");
        save(
            &journal,
            &Transaction {
                home: home.clone(),
                before: [None, Some(before.clone()), None],
                after: [None, Some(after.clone()), None],
            },
        )
        .unwrap();
        fs::write(home.join("settings.json"), b"{\"external\":true}").unwrap();
        assert!(recover(&paths, &home).is_err());
        assert_eq!(
            fs::read(home.join("settings.json")).unwrap(),
            b"{\"external\":true}"
        );
        fs::write(home.join("settings.json"), after).unwrap();
        recover(&paths, &home).unwrap();
        assert_eq!(fs::read(home.join("settings.json")).unwrap(), before);
        assert!(!journal.exists());
    }
    #[test]
    fn auth_headers_and_unsupported_model_overrides_are_explicit() {
        let mut p = json!({"baseUrl":"https://example.invalid","api":"anthropic-messages","authHeader":true,"apiKey":"literal","models":[{"id":"m"}]});
        assert!(matches!(
            parse_provider("p", &p, &json!({})).unwrap().credential,
            Credential::Bearer { .. }
        ));
        p["headers"] = json!({"x-api-key":"header-secret"});
        assert_eq!(
            parse_provider("p", &p, &json!({})).unwrap().credential,
            Credential::XApiKey {
                value: "header-secret".into()
            }
        );
        p["models"][0]["headers"] = json!({"x":"!command"});
        assert!(parse_provider("p", &p, &json!({})).is_err());
        p["models"][0].as_object_mut().unwrap().remove("headers");
        p["models"][0]["contextWindow"] = json!(-1);
        assert!(parse_provider("p", &p, &json!({})).is_err());
    }
}
