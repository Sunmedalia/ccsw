//! Direct editing of Pi's custom model files, without a CCSW provider mirror.
use super::*;

pub fn load(home: &Path) -> Result<config::Config> {
    if home.join(".ccsw-native-transaction.json").exists() {
        let _lock = lock(home)?;
        recover_native(home)?;
    }
    project(
        &read(&home.join("models.json"))?,
        &read(&home.join("settings.json"))?,
        &read(&home.join("auth.json"))?,
    )
}

fn project(models: &Value, settings: &Value, auth: &Value) -> Result<config::Config> {
    let mut config = config::Config::default();
    if let Some(providers) = models.get("providers") {
        let providers = providers
            .as_object()
            .context("Pi providers must be an object")?;
        for (id, value) in providers {
            match parse_provider(id, value, auth) {
                Ok(mut profile) => {
                    for entry in &mut profile.models {
                        entry.description = value["models"]
                            .as_array()
                            .and_then(|models| models.iter().find(|m| m["id"] == entry.id))
                            .and_then(|m| m["description"].as_str())
                            .map(str::to_owned);
                    }
                    if settings["defaultProvider"] == *id
                        && let Some(model) = settings["defaultModel"].as_str()
                        && profile.models.iter().any(|m| m.id == model)
                    {
                        profile.default_model = model.into();
                    }
                    config.profiles.insert(id.clone(), profile);
                }
                Err(error) => {
                    // Unsupported native entries stay in the file, never replaced by an import.
                    config
                        .pi
                        .extras
                        .insert(id.clone(), format!("Read-only: {error}"));
                }
            }
        }
    }
    Ok(config)
}

pub fn description(home: &Path, config: &config::Config) -> String {
    let readonly = config
        .pi
        .extras
        .iter()
        .map(|(id, reason)| format!("{id}: {reason}"))
        .collect::<Vec<_>>()
        .join(" · ");
    format!(
        "Pi files: {} · {} editable providers{}",
        home.display(),
        config.profiles.len(),
        if readonly.is_empty() {
            String::new()
        } else {
            format!(" · {readonly}")
        }
    )
}

pub fn update(
    home: &Path,
    edit: impl FnOnce(&mut config::Config) -> Result<()>,
) -> Result<config::Config> {
    let _lock = lock(home)?;
    recover_native(home)?;
    let mut models = read(&home.join("models.json"))?;
    let mut settings = read(&home.join("settings.json"))?;
    let auth = read(&home.join("auth.json"))?;
    let original_documents = [models.clone(), settings.clone()];
    let before = project(&models, &settings, &auth)?;
    let mut after = before.clone();
    edit(&mut after)?;
    if before.profiles == after.profiles {
        return Ok(after);
    }
    if models.get("providers").is_none() {
        models["providers"] = json!({});
    }
    let removed = before
        .profiles
        .keys()
        .filter(|id| !after.profiles.contains_key(*id))
        .collect::<Vec<_>>();
    let added = after
        .profiles
        .keys()
        .filter(|id| !before.profiles.contains_key(*id))
        .collect::<Vec<_>>();
    let rename = if removed.len() == 1 && added.len() == 1 {
        Some((
            removed[0].clone(),
            added[0].clone(),
            models["providers"][removed[0]].clone(),
        ))
    } else {
        None
    };
    for id in before
        .profiles
        .keys()
        .filter(|id| !after.profiles.contains_key(*id))
    {
        models["providers"].as_object_mut().unwrap().remove(id);
    }
    for (id, profile) in &after.profiles {
        if before.profiles.get(id) == Some(profile) {
            continue;
        }
        if !before.profiles.contains_key(id) && models["providers"].get(id).is_some() {
            bail!("Pi provider '{id}' already exists as a read-only entry");
        }
        if !profile.enabled || !profile.disabled_models.is_empty() {
            bail!(
                "Pi has no enabled flag; delete a model or provider to remove it from models.json"
            );
        }
        profile.validate()?;
        let renamed = rename.as_ref().filter(|(_, new, _)| new == id);
        let old = before
            .profiles
            .get(id)
            .or_else(|| renamed.and_then(|(old, _, _)| before.profiles.get(old)));
        let mut provider = models["providers"]
            .get(id)
            .cloned()
            .or_else(|| renamed.map(|(_, _, raw)| raw.clone()))
            .unwrap_or(json!({}));
        let old_models = provider["models"].as_array().cloned().unwrap_or_default();
        provider["baseUrl"] = json!(profile.base_url);
        provider["name"] = json!(profile.name);
        provider["api"] = json!(match profile.api_format {
            ApiFormat::Anthropic => "anthropic-messages",
            ApiFormat::OpenaiChat => "openai-completions",
            ApiFormat::OpenaiResponses => "openai-responses",
        });
        if old.is_none_or(|p| p.credential != profile.credential) || renamed.is_some() {
            for key in ["apiKey", "authHeader"] {
                provider.as_object_mut().unwrap().remove(key);
            }
            if let Some(headers) = provider.get_mut("headers").and_then(Value::as_object_mut) {
                headers.retain(|k, _| {
                    !["authorization", "x-api-key", "api-key"]
                        .contains(&k.to_ascii_lowercase().as_str())
                });
            }
            match &profile.credential {
                Credential::None => {}
                Credential::Bearer { value } => {
                    provider["apiKey"] = json!(literal(value));
                    provider["authHeader"] = json!(true);
                }
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
            }
        }
        let mut entries = profile.models.clone();
        if !entries
            .iter()
            .any(|m| strip_1m(&m.id) == strip_1m(&profile.default_model))
        {
            entries.push(ModelEntry {
                id: profile.default_model.clone(),
                label: None,
                description: None,
                context_window: None,
                max_output_tokens: None,
            });
        }
        provider["models"] = Value::Array(
            entries
                .iter()
                .map(|entry| {
                    let id = strip_1m(&entry.id);
                    let removed = old_models
                        .iter()
                        .filter(|m| !entries.iter().any(|e| m["id"] == strip_1m(&e.id)))
                        .collect::<Vec<_>>();
                    let added_count = entries
                        .iter()
                        .filter(|e| !old_models.iter().any(|m| m["id"] == strip_1m(&e.id)))
                        .count();
                    let mut model = old_models
                        .iter()
                        .find(|m| m["id"] == id)
                        .or_else(|| (removed.len() == 1 && added_count == 1).then(|| removed[0]))
                        .cloned()
                        .unwrap_or(json!({}));
                    model["id"] = json!(id);
                    model["name"] = json!(entry.label.as_deref().unwrap_or(id));
                    if let Some(description) = &entry.description {
                        model["description"] = json!(description);
                    } else {
                        model.as_object_mut().unwrap().remove("description");
                    }
                    // A changed provider protocol must not be shadowed by an old per-model override.
                    if old.is_some_and(|p| p.api_format != profile.api_format) {
                        model.as_object_mut().unwrap().remove("api");
                    }
                    for (key, number) in [
                        (
                            "contextWindow",
                            entry
                                .context_window
                                .or_else(|| entry.id.ends_with("[1m]").then_some(1_000_000)),
                        ),
                        ("maxTokens", entry.max_output_tokens),
                    ] {
                        if let Some(number) = number {
                            model[key] = json!(number);
                        } else {
                            model.as_object_mut().unwrap().remove(key);
                        }
                    }
                    model
                })
                .collect(),
        );
        models["providers"][id] = provider;
    }
    if let Some((old, new, _)) = &rename
        && settings["defaultProvider"] == *old
    {
        settings["defaultProvider"] = json!(new);
    }
    if let Some(id) = settings["defaultProvider"].as_str().map(str::to_owned)
        && (before.profiles.contains_key(&id) || after.profiles.contains_key(&id))
    {
        match after.profiles.get(&id) {
            None => {
                settings.as_object_mut().unwrap().remove("defaultProvider");
                settings.as_object_mut().unwrap().remove("defaultModel");
            }
            Some(profile)
                if !models["providers"][&id]["models"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|m| m["id"] == settings["defaultModel"]) =>
            {
                settings["defaultModel"] = json!(strip_1m(&profile.default_model));
            }
            _ => {}
        }
    }
    let result = project(&models, &settings, &auth)?;
    for id in after.profiles.keys() {
        if !result.profiles.contains_key(id) {
            bail!("Edited Pi provider '{id}' cannot be loaded; no changes saved");
        }
    }
    if documents(home)? != original_documents {
        bail!("Pi files changed during editing; reload and retry");
    }
    commit_native(home, [models, settings])?;
    Ok(result)
}

pub fn set_default(home: &Path, provider: &str, model: &str) -> Result<()> {
    let _lock = lock(home)?;
    recover_native(home)?;
    let config = load(home)?;
    let profile = config
        .profiles
        .get(provider)
        .context("Pi provider missing")?;
    let model = strip_1m(model);
    if !profile.models.iter().any(|m| strip_1m(&m.id) == model) {
        bail!("Pi model missing");
    }
    let mut settings = read(&home.join("settings.json"))?;
    settings["defaultProvider"] = json!(provider);
    settings["defaultModel"] = json!(model);
    save(&home.join("settings.json"), &settings)
}

#[derive(Serialize, Deserialize)]
struct NativeTransaction {
    before: [Option<Vec<u8>>; 2],
    after: [Option<Vec<u8>>; 2],
}
fn bytes(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
fn recover_native(home: &Path) -> Result<()> {
    let journal = home.join(".ccsw-native-transaction.json");
    let Some(raw) = bytes(&journal)? else {
        return Ok(());
    };
    let tx: NativeTransaction = serde_json::from_slice(&raw)?;
    for (i, name) in ["models.json", "settings.json"].iter().enumerate() {
        let current = bytes(&home.join(name))?;
        if current != tx.before[i] && current != tx.after[i] {
            bail!(
                "Pi file changed after interrupted save; preserve the external edit before recovery"
            );
        }
    }
    for (i, name) in ["models.json", "settings.json"].iter().enumerate() {
        let path = home.join(name);
        if let Some(raw) = &tx.before[i] {
            write(&path, raw)?;
        } else if path.exists() {
            fs::remove_file(path)?;
        }
    }
    fs::remove_file(journal)?;
    Ok(())
}
fn commit_native(home: &Path, docs: [Value; 2]) -> Result<()> {
    let names = ["models.json", "settings.json"];
    let before = [bytes(&home.join(names[0]))?, bytes(&home.join(names[1]))?];
    let mut after = before.clone();
    for i in 0..2 {
        let old: Value = before[i]
            .as_ref()
            .map(|raw| serde_json::from_slice(raw))
            .transpose()?
            .unwrap_or(json!({}));
        if docs[i] != old {
            after[i] = Some(serde_json::to_vec_pretty(&docs[i])?);
        }
    }
    let journal = home.join(".ccsw-native-transaction.json");
    save(
        &journal,
        &NativeTransaction {
            before: before.clone(),
            after: after.clone(),
        },
    )?;
    for i in 0..2 {
        if after[i] != before[i]
            && let Some(raw) = &after[i]
            && let Err(error) = write(&home.join(names[i]), raw)
        {
            recover_native(home).context("Pi rollback failed; reload to recover")?;
            return Err(error);
        }
    }
    fs::remove_file(journal)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(home: &Path) {
        save(&home.join("models.json"), &json!({
            "extension": {"keep": true},
            "providers": {
                "native": {"baseUrl": "https://native.invalid/v1", "api": "openai-completions",
                    "compat": {"supportsStore": false}, "extra": 7,
                    "models": [{"id": "one", "contextWindow": 128000, "cost": {"input": 1}, "reasoning": true}, {"id": "two"}]},
                "builtin": {"models": [{"id": "override"}]}
            }
        })).unwrap();
        save(
            &home.join("settings.json"),
            &json!({"defaultProvider": "native", "defaultModel": "one", "theme": "dark"}),
        )
        .unwrap();
        save(
            &home.join("auth.json"),
            &json!({"native": {"type": "api_key", "key": "fixture-secret"}}),
        )
        .unwrap();
    }

    #[test]
    fn native_crud_preserves_extras_credentials_and_default_references() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        fixture(home);
        let auth = fs::read(home.join("auth.json")).unwrap();
        let initial = load(home).unwrap();
        assert!(initial.profiles.contains_key("native"));
        assert!(initial.pi.extras.contains_key("builtin"));
        update(home, |c| {
            c.profiles.get_mut("native").unwrap().models[0].max_output_tokens = Some(8192);
            Ok(())
        })
        .unwrap();
        let models = read(&home.join("models.json")).unwrap();
        assert_eq!(
            models["providers"]["native"]["models"][0]["cost"]["input"],
            1
        );
        assert_eq!(
            models["providers"]["native"]["models"][0]["reasoning"],
            true
        );
        assert_eq!(
            models["providers"]["native"]["compat"]["supportsStore"],
            false
        );
        assert!(models["providers"]["native"].get("apiKey").is_none());
        set_default(home, "native", "two").unwrap();
        update(home, |c| {
            let mut profile = c.profiles.remove("native").unwrap();
            profile.name = "Renamed".into();
            c.profiles.insert("renamed".into(), profile);
            Ok(())
        })
        .unwrap();
        let models = read(&home.join("models.json")).unwrap();
        assert_eq!(models["providers"]["renamed"]["extra"], 7);
        assert_eq!(
            read(&home.join("settings.json")).unwrap()["defaultProvider"],
            "renamed"
        );
        update(home, |c| {
            c.profiles.remove("renamed");
            Ok(())
        })
        .unwrap();
        let models = read(&home.join("models.json")).unwrap();
        assert!(models["providers"].get("renamed").is_none());
        assert!(models["providers"].get("builtin").is_some());
        assert_eq!(models["extension"]["keep"], true);
        let settings = read(&home.join("settings.json")).unwrap();
        assert!(settings.get("defaultProvider").is_none());
        assert_eq!(settings["theme"], "dark");
        assert_eq!(fs::read(home.join("auth.json")).unwrap(), auth);
    }

    #[test]
    fn invalid_and_concurrent_edits_do_not_overwrite_files() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        fixture(home);
        let before = fs::read(home.join("models.json")).unwrap();
        assert!(
            update(home, |c| {
                c.profiles.get_mut("native").unwrap().enabled = false;
                Ok(())
            })
            .is_err()
        );
        assert_eq!(fs::read(home.join("models.json")).unwrap(), before);
        assert!(
            update(home, |c| {
                c.profiles.get_mut("native").unwrap().name = "Edited".into();
                save(&home.join("settings.json"), &json!({"external": true}))?;
                Ok(())
            })
            .is_err()
        );
        assert_eq!(fs::read(home.join("models.json")).unwrap(), before);
        assert_eq!(read(&home.join("settings.json")).unwrap()["external"], true);
    }

    #[test]
    fn native_add_and_interrupted_save_recovery() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        fixture(home);
        update(home, |c| {
            let mut profile = c.profiles["native"].clone();
            profile.name = "Added provider".into();
            c.profiles.insert("added".into(), profile);
            Ok(())
        })
        .unwrap();
        assert!(load(home).unwrap().profiles.contains_key("added"));
        let before = [
            bytes(&home.join("models.json")).unwrap(),
            bytes(&home.join("settings.json")).unwrap(),
        ];
        let mut after = before.clone();
        after[0] = Some(serde_json::to_vec(&json!({"providers": {}})).unwrap());
        save(
            &home.join(".ccsw-native-transaction.json"),
            &NativeTransaction {
                before: before.clone(),
                after: after.clone(),
            },
        )
        .unwrap();
        write(&home.join("models.json"), after[0].as_ref().unwrap()).unwrap();
        assert!(load(home).unwrap().profiles.contains_key("added"));
        assert_eq!(bytes(&home.join("models.json")).unwrap(), before[0]);
        assert!(!home.join(".ccsw-native-transaction.json").exists());
    }
}
