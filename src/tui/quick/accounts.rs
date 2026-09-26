//! Read-only account summaries for the Pulse monitor. No credentials enter UI state.
use crate::{
    codex,
    config::{self, AppPaths},
    grok,
};
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Default)]
pub(super) struct Info {
    pub lines: Vec<String>,
    pub card: Option<Card>,
}
#[derive(Clone, Debug, Default)]
pub(super) struct Card {
    pub name: String,
    pub email: String,
    pub badge: String,
    pub rows: Vec<(String, String)>,
    pub gauges: Vec<(String, f64, String)>,
    pub models: Vec<String>,
}
#[derive(Clone, Debug, Default)]
pub(super) struct Accounts {
    pub codex: Info,
    pub grok: Info,
}
fn safe(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(300).collect()
}
fn codex_info(config: &config::Config, live: Option<&str>, local: &str) -> Info {
    let active = match &config.codex.active {
        Some(codex::Selection::Account { id }) => Some(id.as_str()),
        _ => None,
    };
    let mut lines = vec![format!("Saved accounts: {}", config.codex.accounts.len())];
    lines.push(safe(local));
    let chosen = active
        .or(live)
        .and_then(|id| config.codex.accounts.get(id).map(|account| (id, account)));
    if let Some((id, account)) = chosen {
        lines.push(format!("Account: {}", safe(&account.name)));
        lines.push(format!("Email: {}", safe(&account.email)));
        lines.push(format!(
            "Plan: {}",
            safe(account.plan.as_deref().unwrap_or("unknown"))
        ));
        lines.push(format!(
            "State: {}{}",
            if active == Some(id) {
                "Applied by CCSW"
            } else {
                "Saved local login"
            },
            if live == Some(id) {
                " · Local login"
            } else {
                ""
            }
        ));
        let mut public = account.clone();
        public.error = None;
        lines.extend(
            codex::accounts::cached_summary(&public)
                .lines()
                .skip(1)
                .map(safe),
        );
        if account.error.is_some() {
            lines.push("Last quota refresh failed · cached data".into());
        }
    } else if active.is_none() {
        lines.push("CCSW mode: API providers".into());
    }
    for (id, account) in &config.codex.accounts {
        if chosen.is_some_and(|(chosen, _)| chosen == id) {
            continue;
        }
        lines.push(format!(
            "Saved: {} · {}{}",
            safe(&account.name),
            safe(&account.email),
            if live == Some(id.as_str()) {
                " · Local login"
            } else {
                ""
            }
        ));
    }
    lines.push("Quota is cached · e → Accounts to refresh".into());
    let mut card = Card {
        name: "API providers".into(),
        badge: "API".into(),
        ..Default::default()
    };
    if let Some((id, account)) = chosen {
        card.name = safe(&account.name);
        card.email = safe(&account.email);
        card.badge = safe(account.plan.as_deref().unwrap_or("Account"));
        card.rows.push((
            "Login".into(),
            if live == Some(id) {
                "● Local"
            } else {
                "○ Saved"
            }
            .into(),
        ));
        card.rows
            .push(("Accounts".into(), config.codex.accounts.len().to_string()));
        card.rows.push((
            "Updated".into(),
            account
                .refreshed_at
                .map(|t| {
                    format!(
                        "{}m ago",
                        chrono::Utc::now().timestamp().max(0) as u64 / 60
                            - t.min(chrono::Utc::now().timestamp().max(0) as u64) / 60
                    )
                })
                .unwrap_or("—".into()),
        ));
        for text in &lines {
            if let Some((label, tail)) = text.split_once(": ")
                && let Some((percent, reset)) = tail.split_once("% used · resets ")
                && let Ok(percent) = percent.parse::<f64>()
            {
                card.gauges.push((
                    label.into(),
                    percent,
                    reset.split(" (").next().unwrap_or(reset).into(),
                ));
            }
        }
    }
    Info {
        lines,
        card: Some(card),
    }
}
fn grok_info(
    config: &config::Config,
    home: &std::path::Path,
    usage: Option<&grok::usage::Snapshot>,
    error: Option<&str>,
) -> Info {
    let mut lines = vec![
        grok::auth::status(home)
            .map(|s| s.description())
            .unwrap_or_else(|_| "Cannot read Grok login".into()),
    ];
    if let Some(snapshot) = usage {
        lines.extend(snapshot.summary().lines().map(safe));
    } else {
        lines.push("Account usage not loaded · r refresh".into());
    }
    if let Some(error) = error {
        lines.push(format!(
            "Usage: {}{}",
            safe(error),
            if usage.is_some() { " · cached" } else { "" }
        ));
    }
    let count: usize = config
        .grok
        .profiles
        .values()
        .map(|profile| crate::discovery::active_models(profile, &[]).len())
        .sum();
    lines.push(format!(
        "API providers: {} · {count} enabled models",
        config.grok.profiles.len()
    ));
    lines.push(format!(
        "Default: {}",
        safe(
            config
                .grok
                .preferences
                .default
                .as_deref()
                .unwrap_or("Native default")
        )
    ));
    for (id, profile) in &config.grok.profiles {
        for model in crate::discovery::active_models(profile, &[]) {
            lines.push(format!(
                "{} · {}",
                safe(&profile.name),
                safe(&grok::model_key(&config.grok, id, &model.id))
            ));
        }
    }
    lines.push("Direct API traffic is not in the gateway ledger".into());
    let status = grok::auth::status(home).unwrap_or_default();
    let mut card = Card {
        name: "Grok CLI".into(),
        email: status.email.unwrap_or_default(),
        badge: if status.expired {
            "Expired"
        } else if status.saved {
            "OAuth"
        } else {
            "API"
        }
        .into(),
        ..Default::default()
    };
    if let Some(snapshot) = usage {
        let c = &snapshot.credits;
        if let Some(plan) = &c.plan {
            card.badge = safe(plan);
        }
        if let Some(percent) = c.percent {
            card.gauges.push((
                match c.period.as_deref().unwrap_or("") {
                    period if period.to_lowercase().contains("week") => "Weekly credits".into(),
                    period if period.to_lowercase().contains("month") => "Monthly credits".into(),
                    period if period.to_lowercase().contains("day") => "Daily credits".into(),
                    _ => "Credits".into(),
                },
                percent,
                c.reset_at
                    .as_deref()
                    .and_then(|time| chrono::DateTime::parse_from_rfc3339(time).ok())
                    .map(|time| {
                        time.with_timezone(&chrono::Local)
                            .format("%m/%d %H:%M")
                            .to_string()
                    })
                    .unwrap_or_default(),
            ));
        }
        if let Some(balance) = c.prepaid_cents {
            card.rows
                .push(("Balance".into(), format!("${:.2}", balance as f64 / 100.0)));
        }
        if let Some(used) = c.on_demand_used_cents {
            card.rows
                .push(("On demand".into(), format!("${:.2}", used as f64 / 100.0)));
        }
    } else {
        card.rows.push(("Credits".into(), "— · r refresh".into()));
    }
    if error.is_some() {
        card.rows.push((
            "Refresh".into(),
            if usage.is_some() {
                "! Cached"
            } else {
                "! Unavailable"
            }
            .into(),
        ));
    }
    card.rows.push(("Models".into(), count.to_string()));
    card.models = config
        .grok
        .profiles
        .iter()
        .flat_map(|(id, profile)| {
            crate::discovery::active_models(profile, &[])
                .into_iter()
                .map(move |model| safe(&grok::model_key(&config.grok, id, &model.id)))
        })
        .collect();
    card.rows.push((
        "Default".into(),
        safe(
            config
                .grok
                .preferences
                .default
                .as_deref()
                .unwrap_or("Native"),
        ),
    ));
    Info {
        lines,
        card: Some(card),
    }
}
pub(super) fn spawn(
    paths: AppPaths,
    updates: mpsc::SyncSender<Accounts>,
    requests: mpsc::Receiver<(usize, bool)>,
    initial: usize,
) {
    std::thread::spawn(move || {
        let mut client = initial;
        let mut force = false;
        let mut cached: Option<grok::usage::Snapshot> = None;
        let mut checked: Option<Instant> = None;
        let mut usage_error: Option<String> = None;
        let mut identity = None;
        loop {
            let config = config::load(&paths.config);
            let info = match config {
                Ok(config) => {
                    let (live, local) = codex::accounts::live_login()
                        .unwrap_or_else(|_| (None, "Local Codex login unavailable".into()));
                    let codex = codex_info(&config, live.as_deref(), &local);
                    let grok = match grok::home() {
                        Ok(home) => {
                            let account = grok::usage::account(&home).ok().flatten();
                            if identity != account {
                                identity = account;
                                cached = None;
                                checked = None;
                                usage_error = None;
                            }
                            if client == 2
                                && identity.is_some()
                                && (force
                                    || checked.is_none_or(|time| {
                                        time.elapsed() >= Duration::from_secs(60)
                                    }))
                            {
                                checked = Some(Instant::now());
                                match grok::usage::fetch(&home) {
                                    Ok(snapshot)
                                        if Some(&snapshot.account) == identity.as_ref()
                                            && grok::usage::account(&home).ok().flatten()
                                                == identity =>
                                    {
                                        cached = Some(snapshot);
                                        usage_error = None;
                                    }
                                    Ok(_) => {
                                        cached = None;
                                        usage_error = Some("Account changed · r refresh".into());
                                    }
                                    Err(error) => usage_error = Some(error.to_string()),
                                }
                            }
                            grok_info(&config, &home, cached.as_ref(), usage_error.as_deref())
                        }
                        Err(_) => Info {
                            lines: vec!["Grok home unavailable".into()],
                            ..Default::default()
                        },
                    };
                    Accounts { codex, grok }
                }
                Err(_) => Accounts {
                    codex: Info {
                        lines: vec!["Cannot read CCSW account configuration".into()],
                        ..Default::default()
                    },
                    grok: Info {
                        lines: vec!["Cannot read Grok configuration".into()],
                        ..Default::default()
                    },
                },
            };
            if updates.send(info).is_err() {
                break;
            }
            force = false;
            match requests.recv_timeout(Duration::from_secs(2)) {
                Ok((selected, refresh)) => {
                    client = selected;
                    force = refresh;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_codex_account_summary_distinguishes_applied_and_local_and_omits_errors() {
        let mut config = config::Config::default();
        config.codex.accounts.insert(
            "one".into(),
            codex::accounts::Account {
                name: "Personal".into(),
                email: "one@example.com".into(),
                plan: Some("plus".into()),
                error: Some("SECRET-TOKEN".into()),
                ..Default::default()
            },
        );
        config.codex.accounts.insert(
            "two".into(),
            codex::accounts::Account {
                name: "Work".into(),
                email: "two@example.com".into(),
                ..Default::default()
            },
        );
        config.codex.active = Some(codex::Selection::Account { id: "one".into() });
        let summary = codex_info(
            &config,
            Some("two"),
            "Provider: openai · Local login: two@example.com",
        )
        .lines
        .join("\n");
        for text in [
            "Personal",
            "one@example.com",
            "plus",
            "Applied by CCSW",
            "two@example.com",
            "Local login",
            "cached",
        ] {
            assert!(summary.contains(text));
        }
        assert!(!summary.contains("SECRET-TOKEN"));
    }
    #[test]
    fn grok_summary_shows_login_usage_and_models_without_credentials() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join("auth.json"),
            r#"{"auth_mode":"oidc","key":"SECRET","email":"grok@example.com"}"#,
        )
        .unwrap();
        let snapshot = grok::usage::Snapshot {
            account: "test".into(),
            credits: grok::usage::Credits {
                percent: Some(30.0),
                ..Default::default()
            },
            fetched_at: 1,
        };
        let text = grok_info(
            &config::Config::default(),
            home.path(),
            Some(&snapshot),
            Some("Unavailable"),
        )
        .lines
        .join("\n");
        assert!(
            text.contains("grok@example.com")
                && text.contains("30.0% used")
                && text.contains("70.0%")
        );
        assert!(text.contains("cached") && !text.contains("SECRET"));
    }
}
