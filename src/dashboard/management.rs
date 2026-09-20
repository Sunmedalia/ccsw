//! Editable projections with revision checks. Secrets only enter through bounded stdin.
use super::*;
use crate::config::{ApiFormat, Credential, ModelEntry, RoleModels};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;

#[derive(Serialize, Deserialize)]
pub struct EditableModel {
    #[serde(flatten)]
    entry: ModelEntry,
    enabled: bool,
}
#[derive(Serialize, Deserialize)]
pub struct EditableProvider {
    client: String,
    id: String,
    revision: Option<String>,
    name: String,
    enabled: bool,
    base_url: String,
    #[serde(default)]
    endpoint_redacted: bool,
    api_format: ApiFormat,
    credential_kind: String,
    #[serde(default)]
    credential_present: bool,
    // Never serialize replacement credentials back to a frontend.
    #[serde(default, skip_serializing)]
    credential_change: Option<String>,
    #[serde(default, skip_serializing)]
    secret: Option<String>,
    default_model: String,
    models: Vec<EditableModel>,
    #[serde(default)]
    aliases: RoleModels,
    subagent_model: Option<String>,
    #[serde(default)]
    fallback_models: Vec<String>,
}
#[derive(Deserialize)]
pub struct DeleteProvider {
    client: String,
    id: String,
    revision: String,
}
#[derive(Serialize)]
struct Management {
    schema_version: u32,
    providers: Vec<EditableProvider>,
    readonly_pi: Vec<String>,
    errors: Vec<Failure>,
}
fn revision(profile: &Profile) -> Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(profile)?)
    ))
}
fn display_endpoint(value: &str) -> (String, bool) {
    let Ok(mut url) = url::Url::parse(value) else {
        return (String::new(), true);
    };
    let redacted = !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some();
    if !redacted {
        return (value.to_owned(), false);
    }
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    (url.to_string(), true)
}
fn project(client: &str, id: &str, p: &Profile) -> Result<EditableProvider> {
    let (base_url, endpoint_redacted) = display_endpoint(&p.base_url);
    let mut effective = p.clone();
    effective.enabled = true;
    let active = discovery::active_models(&effective, &[]);
    let models = if client == "pi" {
        p.models.clone()
    } else {
        discovery::configured_models(p, &[])
    };
    Ok(EditableProvider {
        client: client.into(),
        id: id.into(),
        revision: Some(revision(p)?),
        name: p.name.clone(),
        enabled: p.enabled,
        base_url,
        endpoint_redacted,
        api_format: p.api_format,
        credential_kind: match p.credential {
            Credential::None => "none",
            Credential::Bearer { .. } => "bearer",
            Credential::XApiKey { .. } => "x-api-key",
            Credential::ApiKey { .. } => "api-key",
        }
        .into(),
        credential_present: p.credential.value().is_some(),
        credential_change: None,
        secret: None,
        default_model: p.default_model.clone(),
        models: models
            .into_iter()
            .map(|entry| {
                let enabled = client == "pi"
                    || active.iter().any(|m| {
                        config::canonical_model_id(&m.id) == config::canonical_model_id(&entry.id)
                    });
                EditableModel { entry, enabled }
            })
            .collect(),
        aliases: p.aliases.clone(),
        subagent_model: p.subagent_model.clone(),
        fallback_models: p.fallback_models.clone(),
    })
}
pub fn read(paths: &AppPaths) -> Result<()> {
    let mut providers = vec![];
    let mut errors = vec![];
    match config::load(&paths.config) {
        Ok(c) => {
            for (client, profiles) in [("claude", &c.profiles), ("codex", &c.codex.profiles)] {
                for (id, p) in profiles {
                    providers.push(project(client, id, p)?);
                }
            }
        }
        Err(_) => errors.push(Failure {
            section: "config",
            code: "read_failed",
        }),
    }
    let mut readonly_pi = vec![];
    match pi::home().and_then(|home| pi::native::load_readonly(&home)) {
        Ok(c) => {
            for (id, p) in &c.profiles {
                providers.push(project("pi", id, p)?);
            }
            readonly_pi.extend(c.pi.extras.into_keys());
        }
        Err(_) => errors.push(Failure {
            section: "pi",
            code: "read_failed",
        }),
    }
    println!(
        "{}",
        serde_json::to_string(&Management {
            schema_version: 1,
            providers,
            readonly_pi,
            errors
        })?
    );
    Ok(())
}
pub(super) fn input<T: serde::de::DeserializeOwned>() -> Result<T> {
    let mut bytes = vec![];
    std::io::stdin().take(1_048_577).read_to_end(&mut bytes)?;
    if bytes.len() > 1_048_576 {
        bail!("invalid_input");
    }
    serde_json::from_slice(&bytes).context("invalid_input")
}
fn verify(current: Option<&Profile>, expected: Option<&str>) -> Result<()> {
    let actual = current.map(revision).transpose()?;
    if actual.as_deref() != expected {
        bail!("edit_conflict");
    }
    Ok(())
}
fn build(draft: &EditableProvider, old: Option<&Profile>) -> Result<Profile> {
    config::validate_profile_id(&draft.id)?;
    if draft.models.is_empty()
        || !draft.models.iter().any(|m| {
            config::canonical_model_id(&m.entry.id)
                == config::canonical_model_id(&draft.default_model)
        })
    {
        bail!("invalid_input");
    }
    let base_url =
        if let Some(old) = old.filter(|p| display_endpoint(&p.base_url).0 == draft.base_url) {
            old.base_url.clone()
        } else {
            draft.base_url.clone()
        };
    let credential = match draft.credential_change.as_deref().unwrap_or("keep") {
        "keep" => old.map(|p| p.credential.clone()).unwrap_or_default(),
        "replace" => {
            let value = draft.secret.clone().unwrap_or_default();
            match draft.credential_kind.as_str() {
                "none" => Credential::None,
                "bearer" if !value.is_empty() => Credential::Bearer { value },
                "x-api-key" if !value.is_empty() => Credential::XApiKey { value },
                "api-key" if !value.is_empty() => Credential::ApiKey { value },
                _ => bail!("invalid_input"),
            }
        }
        _ => bail!("invalid_input"),
    };
    let p = Profile {
        name: draft.name.clone(),
        enabled: draft.enabled,
        base_url,
        api_format: draft.api_format,
        credential,
        default_model: draft.default_model.clone(),
        aliases: draft.aliases.clone(),
        subagent_model: draft.subagent_model.clone(),
        fallback_models: draft.fallback_models.clone(),
        models: draft.models.iter().map(|m| m.entry.clone()).collect(),
        enabled_models: draft
            .models
            .iter()
            .filter(|m| m.enabled)
            .map(|m| m.entry.id.clone())
            .collect(),
        disabled_models: draft
            .models
            .iter()
            .filter(|m| !m.enabled)
            .map(|m| m.entry.id.clone())
            .collect(),
    };
    // Prevent removed role targets from silently reappearing as implicit models.
    if p.required_model_ids().iter().any(|id| {
        !p.models
            .iter()
            .any(|m| config::canonical_model_id(&m.id) == config::canonical_model_id(id))
    }) {
        bail!("invalid_input");
    }
    if draft.client == "pi" && (!p.enabled || !p.disabled_models.is_empty()) {
        bail!("invalid_input");
    }
    p.validate().context("invalid_input")?;
    Ok(p)
}
fn scope(client: &str) -> Result<config::Client> {
    match client {
        "claude" => Ok(config::Client::Claude),
        "codex" => Ok(config::Client::Codex),
        _ => bail!("invalid_input"),
    }
}
pub(super) fn sync_claude(paths: &AppPaths, client: &str) -> Result<()> {
    if client != "claude" {
        return Ok(());
    }
    let settings = claude_config::settings_path()?;
    match sync::inspect(paths, &settings)? {
        sync::Status::Synced | sync::Status::Pending => {
            sync::apply(paths, &settings, None, false)
                .context("dashboard_configuration_saved_sync_failed")?;
        }
        sync::Status::Paused => bail!("configuration_saved_sync_paused"),
        _ => {}
    }
    Ok(())
}
pub fn save(paths: &AppPaths) -> Result<bool> {
    let draft: EditableProvider = input()?;
    if draft.client == "pi" {
        pi::native::update(&pi::home()?, |cfg| {
            let old = cfg.profiles.get(&draft.id);
            verify(old, draft.revision.as_deref())?;
            let profile = build(&draft, old)?;
            cfg.profiles.insert(draft.id.clone(), profile);
            Ok(())
        })?;
    } else {
        config::update_client(&paths.config, scope(&draft.client)?, |cfg| {
            let old = cfg.profiles.get(&draft.id);
            verify(old, draft.revision.as_deref())?;
            let profile = build(&draft, old)?;
            cfg.profiles.insert(draft.id.clone(), profile);
            Ok(())
        })?;
    }
    sync_claude(paths, &draft.client)?;
    // Saving a Codex provider doesn't apply it. Applying is an explicit separate action.
    Ok(false)
}
pub fn delete(paths: &AppPaths) -> Result<bool> {
    let draft: DeleteProvider = input()?;
    let edit = |cfg: &mut config::Config| {
        verify(cfg.profiles.get(&draft.id), Some(&draft.revision))?;
        cfg.profiles.remove(&draft.id);
        Ok(())
    };
    if draft.client == "pi" {
        pi::native::update(&pi::home()?, edit)?;
    } else {
        config::update_client(&paths.config, scope(&draft.client)?, edit)?;
    }
    sync_claude(paths, &draft.client)?;
    Ok(false)
}

/// Unsaved drafts may fetch/test before a name, ID or model list exists.
pub(super) fn probe_profile(paths: &AppPaths) -> Result<Profile> {
    let mut draft: EditableProvider = input()?;
    let config = if draft.client == "pi" {
        pi::native::load_readonly(&pi::home()?)?
    } else {
        config::load_client(&paths.config, scope(&draft.client)?)?
    };
    let old = if draft.revision.is_some() {
        let old = config.profiles.get(&draft.id);
        verify(old, draft.revision.as_deref())?;
        old
    } else {
        None
    };
    draft.id = "probe".into();
    draft.name = "Probe".into();
    draft.enabled = true;
    draft.default_model = "probe".into();
    draft.models = vec![EditableModel {
        entry: ModelEntry {
            id: "probe".into(),
            label: None,
            description: None,
            max_output_tokens: None,
            context_window: None,
            reasoning_max: None,
        },
        enabled: true,
    }];
    draft.aliases = RoleModels::default();
    draft.subagent_model = None;
    draft.fallback_models.clear();
    // Like the TUI, a base URL can be checked before entering authentication.
    if draft.secret.as_deref().is_none_or(str::is_empty)
        && draft.credential_change.as_deref() == Some("replace")
        && old.is_none()
    {
        draft.credential_kind = "none".into();
    }
    build(&draft, old)
}
