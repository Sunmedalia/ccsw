use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use fs2::FileExt;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tempfile::NamedTempFile;

use crate::config::{
    Credential, ModelEntry, Profile, canonical_model_id, deduplicate_model_entries, set_private,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelCache {
    #[serde(default)]
    pub profiles: BTreeMap<String, CachedModels>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedModels {
    pub fetched_at: u64,
    pub models: Vec<ModelEntry>,
}

pub fn discover(profile: &Profile) -> Result<Vec<ModelEntry>> {
    if !profile.enabled {
        bail!("provider is disabled");
    }
    discover_with_client(
        &Client::builder().timeout(Duration::from_secs(8)).build()?,
        profile,
    )
}

fn discover_with_client(client: &Client, profile: &Profile) -> Result<Vec<ModelEntry>> {
    let endpoint = crate::proxy::models_endpoint(&profile.base_url, profile.api_format)?;
    let mut request = client
        .get(endpoint)
        .header("anthropic-version", "2023-06-01");
    request = match &profile.credential {
        Credential::Bearer { value } => request.bearer_auth(value),
        Credential::XApiKey { value } => request.header("x-api-key", value),
        Credential::ApiKey { value } => request.header("api-key", value),
        Credential::None => request,
    };
    let response = request.send().context("model discovery request failed")?;
    let status = response.status();
    let bytes = response
        .bytes()
        .context("model discovery response could not be read")?;
    if !status.is_success() {
        let limit = bytes.len().min(2048);
        let detail = String::from_utf8_lossy(&bytes[..limit]);
        bail!("model discovery returned HTTP {status}: {detail}");
    }
    let value: Value =
        serde_json::from_slice(&bytes).context("model discovery returned invalid JSON")?;
    parse_models(&value)
}

pub fn parse_models(value: &Value) -> Result<Vec<ModelEntry>> {
    let rows = value
        .get("data")
        .or_else(|| value.get("models"))
        .and_then(Value::as_array)
        .context("response has neither a data nor models array")?;
    let mut models = Vec::new();
    for row in rows {
        let Some(id) = row.get("id").and_then(Value::as_str) else {
            continue;
        };
        let label = row
            .get("display_name")
            .or_else(|| row.get("name"))
            .and_then(Value::as_str)
            .filter(|label| *label != id)
            .map(str::to_owned);
        let description = row
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned);
        models.push(ModelEntry {
            id: id.to_owned(),
            label,
            description,
        });
    }
    if models.is_empty() {
        bail!("model discovery returned no model ids");
    }
    Ok(deduplicate_model_entries(models))
}

pub fn merged_models(profile: &Profile, discovered: &[ModelEntry]) -> Vec<ModelEntry> {
    let references = profile
        .required_model_ids()
        .into_iter()
        .chain(profile.enabled_models.iter().cloned())
        .chain(profile.disabled_models.iter().cloned())
        .collect::<Vec<_>>();
    // A discovered or catalog model may contain both the base ID and its [1m]
    // spelling. The user's configured references decide which spelling is active;
    // catalog metadata is only a fallback for models that are not configured yet.
    let mut one_m_by_model = BTreeMap::<String, bool>::new();
    let configured_ids = std::iter::once(profile.default_model.as_str())
        .chain(profile.aliases.iter().map(|(_, id)| id))
        .chain(profile.subagent_model.iter().map(String::as_str))
        .chain(profile.fallback_models.iter().map(String::as_str))
        .chain(profile.enabled_models.iter().map(String::as_str))
        .chain(profile.disabled_models.iter().map(String::as_str));
    for id in configured_ids
        .chain(profile.models.iter().map(|model| model.id.as_str()))
        .chain(discovered.iter().map(|model| model.id.as_str()))
    {
        one_m_by_model
            .entry(canonical_model_id(id).to_owned())
            .or_insert_with(|| canonical_model_id(id) != id);
    }

    let mut models: BTreeMap<String, ModelEntry> = BTreeMap::new();
    for model in deduplicate_model_entries(discovered.iter().cloned()) {
        models.insert(canonical_model_id(&model.id).to_owned(), model);
    }
    for model in &profile.models {
        let canonical = canonical_model_id(&model.id).to_owned();
        let entry = models.entry(canonical).or_insert_with(|| model.clone());
        if model.label.is_some() {
            entry.label.clone_from(&model.label);
        }
        if model.description.is_some() {
            entry.description.clone_from(&model.description);
        }
    }
    for id in references {
        let canonical = canonical_model_id(&id).to_owned();
        models
            .entry(canonical.clone())
            .or_insert_with(|| ModelEntry {
                id: canonical,
                label: None,
                description: None,
            });
    }
    models
        .into_iter()
        .map(|(canonical, mut model)| {
            let use_one_m = one_m_by_model.get(&canonical).copied().unwrap_or(false);
            model.id = if use_one_m {
                format!("{canonical}[1m]")
            } else {
                canonical
            };
            if let Some(label) = &mut model.label {
                let base = label
                    .strip_suffix(" · 1M")
                    .or_else(|| label.strip_suffix(" 1M"))
                    .unwrap_or(label)
                    .to_owned();
                *label = if use_one_m {
                    format!("{base} · 1M")
                } else {
                    base
                };
            }
            model
        })
        .collect()
}

pub fn active_models(profile: &Profile, discovered: &[ModelEntry]) -> Vec<ModelEntry> {
    if !profile.enabled {
        return Vec::new();
    }
    let required = profile
        .required_model_ids()
        .into_iter()
        .map(|id| canonical_id(&id))
        .collect::<BTreeSet<_>>();
    let enabled = profile
        .enabled_models
        .iter()
        .map(|id| canonical_id(id))
        .collect::<BTreeSet<_>>();
    let disabled = profile
        .disabled_models
        .iter()
        .map(|id| canonical_id(id))
        .collect::<BTreeSet<_>>();
    merged_models(profile, discovered)
        .into_iter()
        .filter(|model| {
            let id = canonical_id(&model.id);
            !disabled.contains(&id) && (required.contains(&id) || enabled.contains(&id))
        })
        .collect()
}

fn canonical_id(id: &str) -> String {
    canonical_model_id(id).to_owned()
}

pub fn configured_models(profile: &Profile, discovered: &[ModelEntry]) -> Vec<ModelEntry> {
    let disc_map: BTreeMap<String, ModelEntry> = discovered
        .iter()
        .cloned()
        .map(|model| (canonical_id(&model.id), model))
        .collect();

    let manual_map: BTreeMap<String, ModelEntry> = profile
        .models
        .iter()
        .cloned()
        .map(|m| (canonical_id(&m.id), m))
        .collect();

    let mut added_ids = Vec::new();
    let mut seen = BTreeSet::new();

    let mut add_id = |id: &str| {
        let base = canonical_id(id);
        if !base.is_empty() && seen.insert(base.clone()) {
            added_ids.push((id.to_owned(), base));
        }
    };

    if !profile.default_model.is_empty() {
        add_id(&profile.default_model);
    }
    for model in &profile.models {
        add_id(&model.id);
    }
    for en in &profile.enabled_models {
        add_id(en);
    }
    for disabled in &profile.disabled_models {
        add_id(disabled);
    }
    for (_, alias) in profile.aliases.iter() {
        add_id(alias);
    }
    if let Some(sub) = &profile.subagent_model {
        add_id(sub);
    }
    for fb in &profile.fallback_models {
        add_id(fb);
    }

    let mut result = Vec::new();
    for (orig_id, base) in added_ids {
        if let Some(m) = manual_map.get(&base) {
            result.push(m.clone());
        } else if let Some(disc) = disc_map.get(&base) {
            result.push(disc.clone());
        } else {
            result.push(ModelEntry {
                id: orig_id,
                label: None,
                description: None,
            });
        }
    }
    result
}

pub fn load_cache(path: &Path) -> ModelCache {
    let mut cache: ModelCache = fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    for cached in cache.profiles.values_mut() {
        cached.models = deduplicate_model_entries(std::mem::take(&mut cached.models));
    }
    cache
}

pub fn save_cache(path: &Path, cache: &ModelCache) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut temp = NamedTempFile::new_in(path.parent().context("cache path has no parent")?)?;
    temp.write_all(&serde_json::to_vec_pretty(cache)?)?;
    set_private(temp.path())?;
    temp.persist(path).map_err(|error| error.error)?;
    set_private(path)?;
    Ok(())
}

pub fn update_cache(path: &Path, edit: impl FnOnce(&mut ModelCache)) -> Result<ModelCache> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let lock_path = path.with_extension("json.lock");
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(lock_path)?;
    lock.lock_exclusive()?;
    let mut latest = load_cache(path);
    edit(&mut latest);
    save_cache(path, &latest)?;
    FileExt::unlock(&lock).ok();
    Ok(latest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    #[test]
    fn parses_both_gateway_shapes() {
        let data = parse_models(&json!({"data": [{"id": "a", "display_name": "A"}]})).unwrap();
        let models = parse_models(&json!({"models": [{"id": "b", "name": "B"}]})).unwrap();
        assert_eq!(data[0].label.as_deref(), Some("A"));
        assert_eq!(models[0].id, "b");
    }

    #[test]
    fn model_discovery_deduplicates_base_and_1m_variants() {
        let models = parse_models(&json!({
            "data": [
                {"id": "model-a[1m]", "display_name": "Model A 1M"},
                {"id": "model-a", "description": "Base model"}
            ]
        }))
        .unwrap();

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "model-a");
        assert_eq!(models[0].label.as_deref(), Some("Model A 1M"));
        assert_eq!(models[0].description.as_deref(), Some("Base model"));
    }

    #[test]
    fn discovery_sends_bearer_auth() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let size = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..size]).to_ascii_lowercase();
            assert!(request.starts_with("get /v1/models "));
            assert!(request.contains("authorization: bearer secret-test-token"));
            let body = r#"{"data":[{"id":"model-a"}]}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(), body
            )
            .unwrap();
        });
        let profile = Profile {
            name: "test".into(),
            enabled: true,
            base_url: format!("http://{address}"),
            api_format: crate::config::ApiFormat::Anthropic,
            credential: Credential::Bearer {
                value: "secret-test-token".into(),
            },
            default_model: "model-a".into(),
            aliases: Default::default(),
            subagent_model: None,
            fallback_models: vec![],
            enabled_models: vec![],
            disabled_models: vec![],
            models: vec![],
        };
        let models = discover(&profile).unwrap();
        server.join().unwrap();
        assert_eq!(models[0].id, "model-a");
    }

    #[test]
    fn active_models_exclude_unselected_catalog_entries() {
        let mut profile = Profile {
            name: "test".into(),
            enabled: true,
            base_url: "https://example.com".into(),
            api_format: crate::config::ApiFormat::Anthropic,
            credential: Credential::None,
            default_model: "model-a".into(),
            aliases: Default::default(),
            subagent_model: None,
            fallback_models: vec![],
            enabled_models: vec!["model-c".into()],
            disabled_models: vec![],
            models: vec![],
        };
        let discovered = ["model-a", "model-b", "model-c"]
            .into_iter()
            .map(|id| ModelEntry {
                id: id.into(),
                label: None,
                description: None,
            })
            .collect::<Vec<_>>();
        let active = active_models(&profile, &discovered);
        assert_eq!(
            active
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            ["model-a", "model-c"]
        );

        profile.enabled_models.clear();
        let active = active_models(&profile, &discovered);
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "model-a");

        profile.disabled_models.push("model-a".into());
        let active = active_models(&profile, &discovered);
        assert!(active.is_empty());

        profile.disabled_models.clear();
        profile.enabled = false;
        assert!(active_models(&profile, &discovered).is_empty());
    }

    #[test]
    fn active_models_count_canonical_models_once_when_1m_is_enabled() {
        let profile = Profile {
            name: "edgefn".into(),
            enabled: true,
            base_url: "https://example.com".into(),
            api_format: crate::config::ApiFormat::OpenaiChat,
            credential: Credential::None,
            default_model: "model-c[1m]".into(),
            aliases: Default::default(),
            subagent_model: None,
            fallback_models: vec![],
            enabled_models: vec!["model-a[1m]".into(), "model-b[1m]".into()],
            disabled_models: vec![],
            models: vec![
                ModelEntry {
                    id: "model-b[1m]".into(),
                    label: Some("Model B · 1M".into()),
                    description: None,
                },
                ModelEntry {
                    id: "model-c[1m]".into(),
                    label: Some("Model C · 1M".into()),
                    description: None,
                },
            ],
        };
        let discovered = ["model-a", "model-b", "model-c"]
            .into_iter()
            .map(|id| ModelEntry {
                id: id.into(),
                label: None,
                description: None,
            })
            .collect::<Vec<_>>();

        let active = active_models(&profile, &discovered);
        assert_eq!(active.len(), 3);
        assert_eq!(
            active
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            ["model-a[1m]", "model-b[1m]", "model-c[1m]"]
        );
    }

    #[test]
    fn configured_context_mode_overrides_catalog_and_discovery_variants() {
        let profile = Profile {
            name: "mixed-context".into(),
            enabled: true,
            base_url: "https://example.com".into(),
            api_format: crate::config::ApiFormat::Anthropic,
            credential: Credential::None,
            default_model: "model-a".into(),
            aliases: Default::default(),
            subagent_model: None,
            fallback_models: vec![],
            enabled_models: vec!["model-b[1m]".into()],
            disabled_models: vec![],
            models: vec![
                ModelEntry {
                    id: "model-a[1m]".into(),
                    label: Some("Model A · 1M".into()),
                    description: None,
                },
                ModelEntry {
                    id: "model-b".into(),
                    label: Some("Model B".into()),
                    description: None,
                },
            ],
        };
        let discovered = ["model-a[1m]", "model-a", "model-b", "model-b[1m]"]
            .into_iter()
            .map(|id| ModelEntry {
                id: id.into(),
                label: None,
                description: None,
            })
            .collect::<Vec<_>>();

        let active = active_models(&profile, &discovered);
        assert_eq!(
            active
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            ["model-a", "model-b[1m]"]
        );
        assert_eq!(active[0].label.as_deref(), Some("Model A"));
        assert_eq!(active[1].label.as_deref(), Some("Model B · 1M"));
    }

    #[test]
    fn configured_models_only_include_added_and_profile_models() {
        let profile = Profile {
            name: "test".into(),
            enabled: true,
            base_url: "https://example.com".into(),
            api_format: crate::config::ApiFormat::Anthropic,
            credential: Credential::None,
            default_model: "model-a".into(),
            aliases: Default::default(),
            subagent_model: None,
            fallback_models: vec![],
            enabled_models: vec!["model-c".into()],
            disabled_models: vec![],
            models: vec![ModelEntry {
                id: "manual-x".into(),
                label: Some("Manual X".into()),
                description: None,
            }],
        };
        let discovered = ["model-a", "model-b", "model-c", "model-d", "model-e"]
            .into_iter()
            .map(|id| ModelEntry {
                id: id.into(),
                label: Some(format!("Discovered {id}")),
                description: None,
            })
            .collect::<Vec<_>>();

        // configured_models includes: default_model (model-a), enabled_models (model-c), profile.models (manual-x)
        // It does NOT include unselected remote models: model-b, model-d, model-e!
        let configured = configured_models(&profile, &discovered);
        let ids: Vec<&str> = configured.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["model-a", "manual-x", "model-c"]);
        assert_eq!(configured[0].label.as_deref(), Some("Discovered model-a"));
        assert_eq!(configured[1].label.as_deref(), Some("Manual X"));
    }
}
