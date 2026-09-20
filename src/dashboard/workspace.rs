use super::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

fn fingerprint<T: Serialize>(value: &T) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
pub fn read(paths: &AppPaths) -> Result<()> {
    let c = config::load(&paths.config)?;
    let settings = claude_config::settings_path()?;
    let conflicts = sync::diagnostic(paths, &settings).unwrap_or_default();
    let service = proxy::service_status().ok();
    let native = pi::home()?;
    let native_config = pi::native::load_readonly(&native).ok();
    let preferences = json!({"claude":c.claude,"reasoning":c.codex.reasoning_effort});
    let import = crate::import::detect().ok().flatten().filter(|candidate| {
        !candidate.profile.base_url.starts_with("http://127.0.0.1")
    }).map(|candidate| json!({"source":candidate.source,"name":candidate.profile.name,"model":candidate.profile.default_model,"models":candidate.profile.models.len()}));
    println!(
        "{}",
        json!({"schema_version":1,"preferences":preferences,"preferences_revision":fingerprint(&preferences)?,
        "sync_conflicts":conflicts,"claude_settings":settings,"codex_home":codex::home()?,"pi_home":native,
        "proxy_autostart":service.as_ref().is_some_and(|s|s.installed),"proxy_manager":service.as_ref().map(|s|s.manager),
        "pi_readonly":native_config.map(|c|c.pi.extras).unwrap_or_default(),"import_candidate":import,
        "codex_recovery_needed":paths.state_dir.join("codex-transaction.json").exists()})
    );
    Ok(())
}
pub fn usage_detail(paths: &AppPaths) -> Result<()> {
    let path = paths.state_dir.join(usage::FILE);
    let s = usage::snapshot(&path, &paths.config)?;
    let rows: Vec<Value> = s.rows.iter().map(|r|json!({"hour":r.hour,"day":r.day,"client":r.client,"provider":r.provider,"name":r.name,"model":r.model,"kind":r.kind,"totals":Totals::from(&r.totals)})).collect();
    println!(
        "{}",
        json!({"schema_version":1,"available":path.exists(),"today":s.today(),"offset_seconds":s.offset,"since":s.since,"rows":rows})
    );
    Ok(())
}
pub fn probe(paths: &AppPaths, kind: &str, model: Option<&str>) -> Result<()> {
    let result = (|| -> Result<Value> {
        let profile = management::probe_profile(paths)?;
        match kind {
            "models" => {
                let models = discovery::discover(&profile)?;
                Ok(json!({"models":models}))
            }
            "connection" => {
                let (status, ms) = discovery::test_connection(&profile)?;
                Ok(json!({"http_status":status,"elapsed_ms":ms}))
            }
            "model" => {
                let ms = discovery::test_model(&profile, model.context("invalid_input")?)?;
                Ok(json!({"elapsed_ms":ms}))
            }
            _ => bail!("invalid_input"),
        }
    })();
    let value = match result {
        Ok(data) => json!({"schema_version":1,"ok":true,"data":data}),
        Err(e) => {
            json!({"schema_version":1,"ok":false,"error":if e.to_string()=="edit_conflict" {"edit_conflict"} else if e.to_string()=="invalid_input" {"invalid_input"} else {"probe_failed"}})
        }
    };
    println!("{value}");
    Ok(())
}
fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v[key].as_str().context("invalid_input")
}
pub fn manage(paths: &AppPaths, operation: &str) -> Result<bool> {
    let v: Value = management::input()?;
    match operation {
        "sync" => {
            sync::apply(
                paths,
                &claude_config::settings_path()?,
                v["profile"].as_str(),
                true,
            )?;
        }
        "disconnect" => match text(&v, "client")? {
            "claude" => {
                sync::disconnect(paths, &claude_config::settings_path()?)?;
            }
            "codex" => {
                codex::disconnect(paths)?;
                return Ok(true);
            }
            _ => bail!("invalid_input"),
        },
        "proxy-port" => {
            proxy::set_port(
                paths,
                u16::try_from(v["port"].as_u64().context("invalid_input")?)
                    .context("invalid_input")?,
            )?;
        }
        "proxy-install" => {
            proxy::install(paths)?;
        }
        "proxy-uninstall" => {
            proxy::uninstall()?;
        }
        "account-import" => {
            codex::accounts::import(
                paths,
                v["name"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or("ChatGPT"),
                v["file"].as_str().filter(|s| !s.is_empty()).map(Path::new),
            )?;
        }
        "account-rename" => {
            codex::accounts::rename(paths, text(&v, "id")?, text(&v, "name")?)?;
        }
        "account-remove" => {
            codex::accounts::remove(paths, text(&v, "id")?)?;
        }
        "codex-recover" => {
            codex::recover(paths)?;
        }
        "claude-import" => {
            let candidate = crate::import::detect()?.context("invalid_input")?;
            let id = text(&v, "id")?;
            config::validate_profile_id(id)?;
            config::try_update(&paths.config, |c| {
                if c.profiles.contains_key(id) {
                    bail!("edit_conflict");
                }
                c.profiles.insert(id.into(), candidate.profile);
                Ok(())
            })?;
        }
        "preferences" => {
            let settings: crate::claude_preferences::Settings =
                serde_json::from_value(v["claude"].clone()).context("invalid_input")?;
            settings.validate().context("invalid_input")?;
            let reasoning = v["reasoning"]
                .as_str()
                .filter(|v| !v.is_empty())
                .map(str::to_owned);
            if let Some(value) = &reasoning {
                codex::validate_reasoning(value).context("invalid_input")?;
            }
            let old_revision = text(&v, "revision")?;
            config::try_update(&paths.config, |c| {
                if fingerprint(&json!({"claude":c.claude,"reasoning":c.codex.reasoning_effort}))?
                    != old_revision
                {
                    bail!("edit_conflict");
                }
                c.claude = settings;
                c.codex.reasoning_effort = reasoning;
                Ok(())
            })?;
            management::sync_claude(paths, "claude")?;
        }
        _ => bail!("invalid_input"),
    }
    Ok(false)
}
/// NDJSON progress. Closing stdin cancels login through the same service as the TUI.
pub fn login(paths: &AppPaths, name: &str, device: bool) -> Result<()> {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let _session = crate::uninstall::session(paths)?;
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_reader = cancel.clone();
    std::thread::spawn(move || {
        let mut byte = [0];
        let _ = std::io::stdin().read(&mut byte);
        cancel_reader.store(true, Ordering::Relaxed);
    });
    let emit = |value: Value| {
        println!("{value}");
        let _ = std::io::stdout().flush();
    };
    let result = codex::accounts::login(
        paths,
        if name.trim().is_empty() {
            "ChatGPT"
        } else {
            name
        },
        device,
        &cancel,
        |message| emit(json!({"event":"progress","message":message})),
    );
    match result {
        Ok(id) => emit(json!({"event":"done","ok":true,"id":id})),
        Err(_) => emit(
            json!({"event":"done","ok":false,"error":if cancel.load(Ordering::Relaxed){"cancelled"}else{"login_failed"}}),
        ),
    }
    Ok(())
}
