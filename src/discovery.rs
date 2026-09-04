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

use crate::config::{Credential, ModelEntry, Profile, set_private};

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
    let mut models = BTreeMap::new();
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
        models.insert(
            id.to_owned(),
            ModelEntry {
                id: id.to_owned(),
                label,
                description,
            },
        );
    }
    if models.is_empty() {
        bail!("model discovery returned no model ids");
    }
    Ok(models.into_values().collect())
}

pub fn merged_models(profile: &Profile, discovered: &[ModelEntry]) -> Vec<ModelEntry> {
    let mut models: BTreeMap<String, ModelEntry> = discovered
        .iter()
        .cloned()
        .map(|model| (model.id.clone(), model))
        .collect();
    for model in &profile.models {
        models.insert(model.id.clone(), model.clone());
    }
    for (_, id) in profile.aliases.iter() {
        models.entry(id.to_owned()).or_insert_with(|| ModelEntry {
            id: id.to_owned(),
            label: None,
            description: None,
        });
    }
    for id in profile
        .required_model_ids()
        .into_iter()
        .chain(profile.enabled_models.iter().cloned())
    {
        models.entry(id.clone()).or_insert_with(|| ModelEntry {
            id,
            label: None,
            description: None,
        });
    }
    models.into_values().collect()
}

pub fn active_models(profile: &Profile, discovered: &[ModelEntry]) -> Vec<ModelEntry> {
    let required = profile.required_model_ids();
    let enabled: std::collections::BTreeSet<_> = profile.enabled_models.iter().collect();
    merged_models(profile, discovered)
        .into_iter()
        .filter(|model| required.contains(&model.id) || enabled.contains(&model.id))
        .collect()
}

fn canonical_id(id: &str) -> String {
    if id.to_ascii_lowercase().ends_with("[1m]") {
        id[..id.len().saturating_sub(4)].to_owned()
    } else {
        id.to_owned()
    }
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
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
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
            base_url: "https://example.com".into(),
            api_format: crate::config::ApiFormat::Anthropic,
            credential: Credential::None,
            default_model: "model-a".into(),
            aliases: Default::default(),
            subagent_model: None,
            fallback_models: vec![],
            enabled_models: vec!["model-c".into()],
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
    }

    #[test]
    fn configured_models_only_include_added_and_profile_models() {
        let profile = Profile {
            name: "test".into(),
            base_url: "https://example.com".into(),
            api_format: crate::config::ApiFormat::Anthropic,
            credential: Credential::None,
            default_model: "model-a".into(),
            aliases: Default::default(),
            subagent_model: None,
            fallback_models: vec![],
            enabled_models: vec!["model-c".into()],
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
