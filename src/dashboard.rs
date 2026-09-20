//! Versioned, allow-listed IPC for the native menu. Never serialize source configs.
mod management;
mod workspace;

use crate::{
    claude_config, codex,
    config::{self, AppPaths, Profile},
    discovery, pi, proxy, sync, usage,
};
use anyhow::{Context, Result, bail};
use clap::Subcommand;
use serde::Serialize;
use serde_json::Value;
use std::{collections::BTreeMap, fs, path::Path};

#[derive(Subcommand)]
pub enum Command {
    Workspace {
        #[arg(long)]
        json: bool,
    },
    UsageDetail {
        #[arg(long)]
        json: bool,
    },
    Probe {
        kind: String,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        json: bool,
    },
    Login {
        #[arg(long)]
        name: String,
        #[arg(long)]
        device: bool,
        #[arg(long)]
        json: bool,
    },
    Management {
        #[arg(long)]
        json: bool,
    },
    Snapshot {
        #[arg(long)]
        json: bool,
    },
    Action {
        #[command(subcommand)]
        action: Action,
        #[arg(long, global = true)]
        json: bool,
    },
}
#[derive(Subcommand)]
pub enum Action {
    Manage {
        operation: String,
    },
    SaveProfile,
    DeleteProfile,
    RefreshAccount {
        id: String,
    },
    UseAccount {
        id: String,
    },
    Apply {
        #[arg(long)]
        client: String,
        #[arg(long)]
        profile: String,
        #[arg(long)]
        model: String,
    },
    ProxyStart,
    ProxyStop,
}
#[derive(Serialize)]
struct Failure {
    section: &'static str,
    code: &'static str,
}
#[derive(Serialize)]
struct Window {
    id: String,
    bucket: String,
    window: &'static str,
    used_percent: Option<f64>,
    duration_minutes: Option<u64>,
    resets_at: Option<u64>,
}
#[derive(Serialize)]
struct Account {
    id: String,
    name: String,
    email: String,
    workspace: String,
    plan: Option<String>,
    selected: bool,
    local_login: bool,
    refreshed_at: Option<u64>,
    refresh_failed: bool,
    windows: Vec<Window>,
}
#[derive(Serialize)]
struct Model {
    id: String,
    name: String,
}
#[derive(Serialize)]
struct Provider {
    id: String,
    name: String,
    enabled: bool,
    default_model: String,
    models: Vec<Model>,
}
#[derive(Serialize)]
struct Client {
    id: &'static str,
    provider: Option<String>,
    model: Option<String>,
    status: &'static str,
    providers: Vec<Provider>,
}
#[derive(Default, Serialize)]
struct Totals {
    calls: i64,
    input: i64,
    output: i64,
    unknown: i64,
    failed: i64,
    pending: i64,
    interrupted: i64,
    cache_read: i64,
    cache_write: i64,
}
impl From<&usage::Totals> for Totals {
    fn from(v: &usage::Totals) -> Self {
        Self {
            calls: v.calls,
            input: v.input,
            output: v.output,
            unknown: v.unknown,
            failed: v.failed,
            pending: v.pending,
            interrupted: v.interrupted,
            cache_read: v.cache_read,
            cache_write: v.cache_write,
        }
    }
}
#[derive(Serialize)]
struct Point {
    label: String,
    totals: Totals,
}
#[derive(Serialize)]
struct Range {
    days: i64,
    client: &'static str,
    totals: Totals,
    points: Vec<Point>,
}
#[derive(Serialize)]
struct Usage {
    available: bool,
    offset_seconds: i32,
    ranges: Vec<Range>,
}
#[derive(Serialize)]
struct Snapshot {
    schema_version: u32,
    generated_at: i64,
    config_path: String,
    accounts: Vec<Account>,
    clients: Vec<Client>,
    proxy: Option<proxy::ProxyStatus>,
    usage: Option<Usage>,
    errors: Vec<Failure>,
}
fn windows(value: &Value) -> Vec<Window> {
    let fallback = BTreeMap::from([("codex".to_string(), value["rateLimits"].clone())]);
    let buckets: BTreeMap<String, Value> = value["rateLimitsByLimitId"]
        .as_object()
        .filter(|v| !v.is_empty())
        .map(|v| v.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or(fallback);
    buckets
        .iter()
        .flat_map(|(bucket, v)| {
            ["primary", "secondary"]
                .into_iter()
                .filter_map(move |window| {
                    let w = &v[window];
                    if !w.is_object() {
                        return None;
                    }
                    Some(Window {
                        id: format!("{bucket}:{window}"),
                        bucket: bucket.clone(),
                        window,
                        used_percent: w["usedPercent"]
                            .as_f64()
                            .filter(|v| v.is_finite() && *v >= 0.0),
                        duration_minutes: w["windowDurationMins"].as_u64(),
                        resets_at: w["resetsAt"].as_u64(),
                    })
                })
        })
        .collect()
}
fn providers(profiles: &BTreeMap<String, Profile>, native: bool) -> Vec<Provider> {
    profiles
        .iter()
        .map(|(id, p)| Provider {
            id: id.clone(),
            name: p.name.clone(),
            enabled: p.enabled,
            default_model: p.default_model.clone(),
            models: if native {
                p.models.clone()
            } else {
                discovery::active_models(p, &[])
            }
            .iter()
            .map(|m| Model {
                id: m.id.clone(),
                name: m.label().into(),
            })
            .collect(),
        })
        .collect()
}
fn json_file(path: &Path) -> Result<Value> {
    if !path.exists() {
        return Ok(serde_json::json!({}));
    }
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn usage_data(paths: &AppPaths) -> Result<Usage> {
    let path = paths.state_dir.join("usage.sqlite3");
    let s = usage::snapshot(&path, &paths.config)?;
    let today = chrono::NaiveDate::parse_from_str(&s.today(), "%Y-%m-%d")?;
    let mut ranges = vec![];
    for days in [1, 7, 30] {
        let start = today - chrono::Duration::days(days - 1);
        for client in ["all", "Claude", "Codex", "Pi"] {
            let mut total = usage::Totals::default();
            let mut points: BTreeMap<String, usage::Totals> = if days == 1 {
                (0..24)
                    .map(|h| (format!("{h:02}"), usage::Totals::default()))
                    .collect()
            } else {
                (0..days)
                    .map(|d| {
                        (
                            (start + chrono::Duration::days(d)).to_string(),
                            usage::Totals::default(),
                        )
                    })
                    .collect()
            };
            for row in &s.rows {
                if row.kind != "generation"
                    || (client != "all" && row.client != client)
                    || row.day < start.to_string()
                    || row.day > today.to_string()
                {
                    continue;
                }
                total.add(&row.totals);
                let label = if days == 1 {
                    format!("{:02}", row.hour)
                } else {
                    row.day.clone()
                };
                if let Some(point) = points.get_mut(&label) {
                    point.add(&row.totals);
                }
            }
            ranges.push(Range {
                days,
                client,
                totals: (&total).into(),
                points: points
                    .into_iter()
                    .map(|(label, t)| Point {
                        label,
                        totals: (&t).into(),
                    })
                    .collect(),
            });
        }
    }
    Ok(Usage {
        available: path.exists(),
        offset_seconds: s.offset,
        ranges,
    })
}
fn snapshot(paths: &AppPaths) -> Snapshot {
    let mut errors = vec![];
    let config = config::load(&paths.config).unwrap_or_else(|_| {
        errors.push(Failure {
            section: "config",
            code: "read_failed",
        });
        Default::default()
    });
    let live = codex::accounts::live_login()
        .map(|v| v.0)
        .unwrap_or_else(|_| {
            errors.push(Failure {
                section: "accounts",
                code: "login_unavailable",
            });
            None
        });
    let accounts = config.codex.accounts.iter().map(|(id,a)| Account {
        id:id.clone(), name:a.name.clone(), email:a.email.clone(), workspace:a.workspace.clone(), plan:a.plan.clone(),
        selected: matches!(&config.codex.active, Some(codex::Selection::Account { id: selected }) if selected == id),
        local_login:live.as_ref() == Some(id), refreshed_at:a.refreshed_at, refresh_failed:a.error.is_some(), windows:windows(&a.limits),
    }).collect();
    let mut clients = vec![];
    let claude = (|| -> Result<Client> {
        let path = claude_config::settings_path()?;
        let doc = json_file(&path)?;
        let model = doc["model"]
            .as_str()
            .or(doc["env"]["ANTHROPIC_MODEL"].as_str())
            .map(str::to_owned);
        let provider = model
            .as_deref()
            .and_then(|m| m.split_once("::"))
            .map(|v| v.0.to_owned());
        let status = match sync::inspect(paths, &path)? {
            sync::Status::Synced => "synced",
            sync::Status::NotConnected => "unmanaged",
            sync::Status::Pending => "pending",
            _ => "conflict",
        };
        Ok(Client {
            id: "claude",
            provider,
            model,
            status,
            providers: providers(&config.profiles, false),
        })
    })();
    clients.push(claude.unwrap_or_else(|_| {
        errors.push(Failure {
            section: "claude",
            code: "read_failed",
        });
        Client {
            id: "claude",
            provider: None,
            model: None,
            status: "unavailable",
            providers: providers(&config.profiles, false),
        }
    }));
    let codex_client = (|| -> Result<Client> {
        let path = codex::home()?.join("config.toml");
        let doc: toml::Value = if path.exists() {
            toml::from_str(&fs::read_to_string(path)?)?
        } else {
            toml::Value::Table(Default::default())
        };
        let model = doc.get("model").and_then(|v| v.as_str()).map(str::to_owned);
        let actual_provider = doc
            .get("model_provider")
            .and_then(|v| v.as_str())
            .unwrap_or("openai");
        let mut provider = Some(actual_provider.to_string());
        let mut status = "unmanaged";
        match &config.codex.active {
            Some(codex::Selection::Api {
                profile,
                model: wanted,
            }) => {
                // Binding validation below also checks the actual route and managed fields.
                if actual_provider == "ccsw"
                    && model.as_deref() == Some(config::canonical_model_id(wanted))
                {
                    provider = Some(profile.clone());
                    status = "synced";
                } else {
                    status = "conflict";
                }
            }
            Some(codex::Selection::Account { id }) => {
                status = if actual_provider == "openai" && live.as_ref() == Some(id) {
                    "synced"
                } else {
                    "conflict"
                };
            }
            None => {}
        }
        if codex::dashboard_conflict(paths)? {
            status = "conflict";
        }
        Ok(Client {
            id: "codex",
            provider,
            model,
            status,
            providers: providers(&config.codex.profiles, false),
        })
    })();
    clients.push(codex_client.unwrap_or_else(|_| {
        errors.push(Failure {
            section: "codex",
            code: "read_failed",
        });
        Client {
            id: "codex",
            provider: None,
            model: None,
            status: "unavailable",
            providers: providers(&config.codex.profiles, false),
        }
    }));
    let native = (|| -> Result<Client> {
        let home = pi::home()?;
        let cfg = pi::native::load_readonly(&home)?;
        let doc = json_file(&home.join("settings.json"))?;
        Ok(Client {
            id: "pi",
            provider: doc["defaultProvider"].as_str().map(str::to_owned),
            model: doc["defaultModel"].as_str().map(str::to_owned),
            status: "native",
            providers: providers(&cfg.profiles, true),
        })
    })();
    clients.push(native.unwrap_or_else(|_| {
        errors.push(Failure {
            section: "pi",
            code: "read_failed",
        });
        Client {
            id: "pi",
            provider: None,
            model: None,
            status: "unavailable",
            providers: vec![],
        }
    }));
    let proxy = proxy::status(paths)
        .map_err(|_| {
            errors.push(Failure {
                section: "proxy",
                code: "read_failed",
            })
        })
        .ok();
    let usage = usage_data(paths)
        .map_err(|_| {
            errors.push(Failure {
                section: "usage",
                code: "read_failed",
            })
        })
        .ok();
    Snapshot {
        schema_version: 1,
        generated_at: chrono::Utc::now().timestamp(),
        config_path: paths.config.display().to_string(),
        accounts,
        clients,
        proxy,
        usage,
        errors,
    }
}

fn action(paths: &AppPaths, action: &Action) -> Result<bool> {
    match action {
        Action::Manage { operation } => workspace::manage(paths, operation),
        Action::SaveProfile => management::save(paths),
        Action::DeleteProfile => management::delete(paths),
        Action::RefreshAccount { id } => {
            codex::accounts::refresh(paths, id)?;
            Ok(false)
        }
        Action::UseAccount { id } => {
            codex::accounts::activate(paths, id)?;
            Ok(true)
        }
        Action::ProxyStart => {
            proxy::start(paths, None)?;
            Ok(false)
        }
        Action::ProxyStop => {
            proxy::stop(paths)?;
            Ok(false)
        }
        Action::Apply {
            client,
            profile,
            model,
        } => {
            match client.as_str() {
                "codex" => codex::apply(paths, profile, Some(model), None)?,
                "pi" => pi::native::set_default(&pi::home()?, profile, model)?,
                "claude" => {
                    config::update(&paths.config, |cfg| {
                        let p = cfg.profiles.get_mut(profile).context("Provider missing")?;
                        if !discovery::active_models(p, &[])
                            .iter()
                            .any(|m| m.id == *model)
                        {
                            bail!("Model unavailable");
                        }
                        p.default_model = model.clone();
                        Ok(())
                    })?;
                    sync::apply(paths, &claude_config::settings_path()?, Some(profile), true)
                        .context("dashboard_configuration_saved_sync_failed")?;
                }
                _ => bail!("Unsupported client"),
            }
            Ok(client == "codex")
        }
    }
}
#[derive(Serialize)]
struct ActionResult {
    schema_version: u32,
    ok: bool,
    restart_required: bool,
    error: Option<&'static str>,
    saved: bool,
    sync_conflicts: Vec<String>,
}
pub fn run(paths: &AppPaths, command: &Command) -> Result<()> {
    match command {
        Command::Workspace { .. } => workspace::read(paths)?,
        Command::UsageDetail { .. } => workspace::usage_detail(paths)?,
        Command::Probe { kind, model, .. } => workspace::probe(paths, kind, model.as_deref())?,
        Command::Login { name, device, .. } => workspace::login(paths, name, *device)?,
        Command::Management { .. } => management::read(paths)?,
        Command::Snapshot { .. } => println!("{}", serde_json::to_string(&snapshot(paths))?),
        Command::Action {
            action: requested, ..
        } => {
            // Serialize menu writers across app instances; existing services retain their own locks.
            let result = (|| {
                let _session = crate::uninstall::session(paths)?;
                fs::create_dir_all(&paths.state_dir)?;
                let guard = fs::OpenOptions::new()
                    .create(true)
                    .truncate(false)
                    .read(true)
                    .write(true)
                    .open(paths.state_dir.join("dashboard.lock"))?;
                fs2::FileExt::try_lock_exclusive(&guard).context("dashboard_busy")?;
                action(paths, requested)
            })();
            let (ok, restart_required, error) = match result {
                Ok(restart) => (true, restart, None),
                Err(e) => (
                    false,
                    false,
                    Some(if e.to_string() == "dashboard_busy" {
                        "busy"
                    } else if e
                        .chain()
                        .any(|cause| cause.to_string() == "configuration_saved_sync_paused")
                    {
                        "configuration_saved_sync_paused"
                    } else if e.to_string() == "dashboard_configuration_saved_sync_failed" {
                        "configuration_saved_sync_failed"
                    } else if e.to_string() == "edit_conflict" {
                        "edit_conflict"
                    } else if e.to_string() == "invalid_input" {
                        "invalid_input"
                    } else {
                        "action_failed"
                    }),
                ),
            };
            println!(
                "{}",
                serde_json::to_string(&ActionResult {
                    schema_version: 1,
                    ok,
                    restart_required,
                    error,
                    saved: ok
                        || matches!(
                            error,
                            Some(
                                "configuration_saved_sync_failed"
                                    | "configuration_saved_sync_paused"
                            )
                        ),
                    sync_conflicts: if matches!(
                        error,
                        Some("configuration_saved_sync_failed" | "configuration_saved_sync_paused")
                    ) {
                        claude_config::settings_path()
                            .and_then(|p| sync::diagnostic(paths, &p))
                            .unwrap_or_default()
                    } else {
                        vec![]
                    },
                })?
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_allowlist_and_unknowns() {
        let v = serde_json::json!({"secret":"never emit","rateLimits":{"primary":{"usedPercent":23,"windowDurationMins":300,"resetsAt":123,"token":"hidden"},"secondary":{}}});
        let w = windows(&v);
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].used_percent, Some(23.0));
        assert_eq!(w[1].used_percent, None);
        let encoded = serde_json::to_string(&w).unwrap();
        assert!(!encoded.contains("secret") && !encoded.contains("hidden"));
        let multiple = serde_json::json!({"rateLimitsByLimitId":{"a":{"primary":{"usedPercent":0}},"b":{"primary":{"usedPercent":100}}}});
        assert_eq!(windows(&multiple).len(), 2);
    }
}
