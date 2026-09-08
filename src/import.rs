use std::{collections::BTreeSet, fs, path::PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

use crate::config::{Credential, ModelEntry, Profile, RoleModels, deduplicate_model_entries};

#[derive(Debug, Clone)]
pub struct ImportCandidate {
    pub source: PathBuf,
    pub profile: Profile,
}

impl ImportCandidate {
    pub fn summary(&self) -> Vec<String> {
        vec![
            format!("Source       {}", self.source.display()),
            format!("Base URL     {}", self.profile.base_url),
            format!("Credential   {}", self.profile.credential.masked()),
            format!("Default      {}", self.profile.default_model),
            format!("Models       {} detected", self.profile.models.len()),
        ]
    }
}

pub fn detect() -> Result<Option<ImportCandidate>> {
    let source = crate::claude_config::settings_path()?;
    if !source.exists() {
        return Ok(None);
    }
    let value: Value = serde_json::from_slice(&fs::read(&source)?)
        .with_context(|| format!("failed to parse {}", source.display()))?;
    let env = value.get("env").and_then(Value::as_object);
    let get = |key: &str| {
        env.and_then(|env| env.get(key))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let Some(base_url) = get("ANTHROPIC_BASE_URL") else {
        return Ok(None);
    };
    let credential = if let Some(value) = get("ANTHROPIC_AUTH_TOKEN") {
        Credential::Bearer { value }
    } else if let Some(value) = get("ANTHROPIC_API_KEY") {
        Credential::XApiKey { value }
    } else {
        Credential::None
    };
    let aliases = RoleModels {
        opus: get("ANTHROPIC_DEFAULT_OPUS_MODEL"),
        sonnet: get("ANTHROPIC_DEFAULT_SONNET_MODEL"),
        haiku: get("ANTHROPIC_DEFAULT_HAIKU_MODEL"),
        fable: get("ANTHROPIC_DEFAULT_FABLE_MODEL"),
    };
    let default_model = value
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| get("ANTHROPIC_MODEL"))
        .or_else(|| aliases.sonnet.clone())
        .or_else(|| aliases.opus.clone())
        .unwrap_or_else(|| "sonnet".into());
    let subagent_model = get("CLAUDE_CODE_SUBAGENT_MODEL");
    let mut ids = BTreeSet::new();
    ids.insert(default_model.clone());
    ids.extend(aliases.iter().map(|(_, model)| model.to_owned()));
    ids.extend(subagent_model.iter().cloned());
    let models = imported_models(env, ids);

    Ok(Some(ImportCandidate {
        source,
        profile: Profile {
            name: "Imported Claude settings".into(),
            enabled: true,
            base_url,
            api_format: crate::config::ApiFormat::Anthropic,
            credential,
            default_model,
            aliases,
            subagent_model,
            fallback_models: vec![],
            enabled_models: vec![],
            disabled_models: vec![],
            models,
        },
    }))
}

fn imported_models(
    env: Option<&serde_json::Map<String, Value>>,
    ids: impl IntoIterator<Item = String>,
) -> Vec<ModelEntry> {
    deduplicate_model_entries(ids.into_iter().map(|id| {
        let label = alias_label(env, &id);
        ModelEntry {
            max_output_tokens: None,
            context_window: None,
            id,
            label,
            description: None,
        }
    }))
}

fn alias_label(env: Option<&serde_json::Map<String, Value>>, model: &str) -> Option<String> {
    let roles = ["OPUS", "SONNET", "HAIKU", "FABLE"];
    for role in roles {
        let id_key = format!("ANTHROPIC_DEFAULT_{role}_MODEL");
        let name_key = format!("ANTHROPIC_DEFAULT_{role}_MODEL_NAME");
        let id = env.and_then(|env| env.get(&id_key)).and_then(Value::as_str);
        if id == Some(model) {
            return env
                .and_then(|env| env.get(&name_key))
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_masks_token() {
        let candidate = ImportCandidate {
            source: "/tmp/settings.json".into(),
            profile: Profile {
                name: "x".into(),
                enabled: true,
                base_url: "http://localhost".into(),
                api_format: crate::config::ApiFormat::Anthropic,
                credential: Credential::Bearer {
                    value: "top-secret-token".into(),
                },
                default_model: "m".into(),
                aliases: RoleModels::default(),
                subagent_model: None,
                fallback_models: vec![],
                enabled_models: vec![],
                disabled_models: vec![],
                models: vec![],
            },
        };
        let summary = candidate.summary().join("\n");
        assert!(!summary.contains("top-secret-token"));
    }

    #[test]
    fn import_deduplicates_base_and_1m_variants() {
        let env = serde_json::json!({
            "ANTHROPIC_DEFAULT_OPUS_MODEL": "model-a",
            "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME": "Model A",
            "ANTHROPIC_DEFAULT_SONNET_MODEL": "model-a[1m]"
        });
        let env = env.as_object().unwrap();
        let models = imported_models(Some(env), ["model-a".to_owned(), "model-a[1m]".to_owned()]);

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "model-a");
        assert_eq!(models[0].label.as_deref(), Some("Model A"));
    }
}
