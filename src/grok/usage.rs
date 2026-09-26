//! Grok account credits and explicit minimal wake requests, following official CLI contracts:
//! https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-shell/src/extensions/billing.rs
use super::auth;
use anyhow::{Context, Result, bail};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path, time::Duration};
const ENDPOINT: &str = "https://cli-chat-proxy.grok.com/v1/billing?format=credits";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Credits {
    pub percent: Option<f64>,
    pub period: Option<String>,
    pub reset_at: Option<String>,
    pub used_cents: Option<i64>,
    pub limit_cents: Option<i64>,
    pub prepaid_cents: Option<i64>,
    pub on_demand_used_cents: Option<i64>,
    pub on_demand_cap_cents: Option<i64>,
    pub plan: Option<String>,
    pub unified: Option<bool>,
}
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub account: String,
    pub credits: Credits,
    pub fetched_at: i64,
}
fn account_id(entry: &Value) -> String {
    let identity = entry["user_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| entry["key"].as_str())
        .or_else(|| entry["access_token"].as_str())
        .unwrap_or("");
    format!(
        "{:x}",
        Sha256::digest(format!(
            "{}\0{}\0{}",
            entry["oidc_issuer"].as_str().unwrap_or(""),
            identity,
            entry["principal_id"].as_str().unwrap_or("")
        ))
    )
}
pub fn account(home: &Path) -> Result<Option<String>> {
    Ok(auth::saved_entry(home)?.as_ref().map(account_id))
}
fn cents(value: &Value) -> Result<Option<i64>> {
    if value.is_null() {
        return Ok(None);
    }
    let object = value.as_object().context("Invalid Grok credit amount")?;
    let val = object.get("val");
    match val {
        None => Ok(Some(0)), // Proto3 omits zero-valued scalars.
        Some(v) => v
            .as_i64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            .map(Some)
            .context("Invalid Grok credit amount"),
    }
}
fn timestamp(value: &Value) -> Option<String> {
    value
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|time| time.to_rfc3339())
}
pub fn parse(value: &Value) -> Result<Credits> {
    let config = value
        .get("config")
        .filter(|v| v.is_object())
        .context("Grok returned no account credit allowance")?;
    let used_cents = cents(&config["used"])?;
    let limit_cents = cents(&config["monthlyLimit"])?;
    let percent = match config.get("creditUsagePercent").filter(|v| !v.is_null()) {
        Some(v) => Some(
            v.as_f64()
                .filter(|v| v.is_finite() && *v >= 0.0)
                .context("Invalid Grok credit usage percentage")?,
        ),
        None => used_cents
            .zip(limit_cents)
            .filter(|(used, limit)| *used >= 0 && *limit > 0)
            .map(|(used, limit)| used as f64 / limit as f64 * 100.0),
    };
    let period = match config
        .pointer("/currentPeriod/type")
        .and_then(Value::as_str)
    {
        Some("USAGE_PERIOD_TYPE_WEEKLY") => Some("Weekly".into()),
        Some("USAGE_PERIOD_TYPE_MONTHLY") => Some("Monthly".into()),
        _ if config.get("monthlyLimit").is_some_and(|v| !v.is_null()) => Some("Monthly".into()),
        _ => None,
    };
    let credits = Credits {
        unified: config["isUnifiedBillingUser"].as_bool(),
        percent,
        period,
        reset_at: timestamp(&config["currentPeriod"]["end"])
            .or_else(|| timestamp(&config["billingPeriodEnd"])),
        used_cents,
        limit_cents,
        prepaid_cents: cents(&config["prepaidBalance"])?,
        on_demand_used_cents: cents(&config["onDemandUsed"])?,
        on_demand_cap_cents: cents(&config["onDemandCap"])?,
        plan: value
            .get("subscription_tier")
            .or_else(|| value.get("subscriptionTier"))
            .and_then(Value::as_str)
            .map(|s| s.chars().filter(|c| !c.is_control()).take(100).collect()),
    };
    if credits.percent.is_none()
        && credits.used_cents.is_none()
        && credits.limit_cents.is_none()
        && credits.prepaid_cents.is_none()
        && credits.on_demand_used_cents.is_none()
    {
        bail!("Grok returned no account credit usage data");
    }
    Ok(credits)
}
fn fetch_at(endpoint: &str, entry: &Value) -> Result<Credits> {
    let token = entry["key"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| entry["access_token"].as_str().filter(|s| !s.is_empty()))
        .context("No Grok OAuth access token; sign in again")?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| anyhow::anyhow!("Cannot create Grok usage client"))?;
    let mut request = client
        .get(endpoint)
        .bearer_auth(token)
        .header("X-XAI-Token-Auth", "xai-grok-cli")
        .header("Accept", "application/json")
        .header("x-grok-client-version", "1.0.41")
        .header("x-grok-client-identifier", "grok-shell");
    if let Some(user) = entry["user_id"].as_str().filter(|s| !s.is_empty()) {
        request = request.header("x-userid", user);
    }
    let response = request
        .send()
        .map_err(|_| anyhow::anyhow!("Cannot reach Grok usage service; retry refresh"))?;
    let code = response.status();
    if matches!(code.as_u16(), 401 | 403) {
        bail!("Grok usage authorization rejected; refresh your login in Grok or sign in again");
    }
    if !code.is_success() {
        bail!("Grok usage service returned HTTP {}", code.as_u16());
    }
    let mut bytes = Vec::new();
    response
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow::anyhow!("Cannot read Grok usage response"))?;
    if bytes.len() > 1024 * 1024 {
        bail!("Grok usage response exceeds size limit");
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("Invalid Grok usage response"))?;
    parse(&value)
}
fn validated_entry(home: &Path) -> Result<Value> {
    let entry = auth::saved_entry(home)?.context("Sign in to Grok OAuth to view account usage")?;
    if entry["expires_at"]
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .is_some_and(|time| time < chrono::Utc::now())
    {
        bail!("Grok OAuth access token expired; refresh your login in Grok or sign in again");
    }
    if let Some(issuer) = entry["oidc_issuer"].as_str().filter(|s| !s.is_empty()) {
        let trusted = url::Url::parse(issuer).ok().is_some_and(|url| {
            url.scheme() == "https"
                && matches!(url.host_str(), Some("auth.x.ai" | "accounts.x.ai"))
                && url.username().is_empty()
                && url.password().is_none()
        });
        if !trusted {
            bail!("Grok cloud usage is unavailable for this custom OAuth issuer");
        }
    }
    Ok(entry)
}
pub fn fetch(home: &Path) -> Result<Snapshot> {
    let entry = validated_entry(home)?;
    let account = account_id(&entry);
    let credits = fetch_at(ENDPOINT, &entry)?;
    Ok(Snapshot {
        account,
        credits,
        fetched_at: chrono::Utc::now().timestamp(),
    })
}
/// Explicit user-triggered single request. No tools, files, retries or token refresh.
pub fn wake(home: &Path, expected: &str) -> Result<()> {
    let entry = validated_entry(home)?;
    if account_id(&entry) != expected {
        bail!("Grok account changed; try again");
    }
    wake_at(
        "https://cli-chat-proxy.grok.com/v1/chat/completions",
        &entry,
    )
}
fn wake_at(endpoint: &str, entry: &Value) -> Result<()> {
    let token = entry["key"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| entry["access_token"].as_str().filter(|s| !s.is_empty()))
        .context("No Grok OAuth access token")?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut request = client.post(endpoint).bearer_auth(token)
        .header("X-XAI-Token-Auth", "xai-grok-cli")
        .header("x-grok-model-override", "grok-build")
        .header("x-grok-client-identifier", "grok-shell")
        .header("x-grok-client-version", "1.0.41")
        .json(&serde_json::json!({"model":"grok-build", "messages":[{"role":"user","content":"Reply OK."}], "max_tokens":1, "stream":false}));
    if let Some(user) = entry["user_id"].as_str().filter(|s| !s.is_empty()) {
        request = request.header("x-userid", user);
    }
    let response = request
        .send()
        .map_err(|_| anyhow::anyhow!("Grok wake request failed; no automatic retry"))?;
    if !response.status().is_success() {
        bail!("Grok wake rejected (HTTP {})", response.status().as_u16());
    }
    let mut bytes = Vec::new();
    response
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow::anyhow!("Cannot read Grok wake response"))?;
    if bytes.len() > 1024 * 1024 {
        bail!("Grok wake response exceeds size limit");
    }
    let response: Value = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("Invalid Grok wake response"))?;
    if response["choices"]
        .as_array()
        .is_none_or(|choices| choices.is_empty())
    {
        bail!("Grok returned no wake completion");
    }
    Ok(())
}
fn dollars(cents: i64) -> String {
    format!("${:.2}", cents as f64 / 100.0)
}
impl Snapshot {
    pub fn summary(&self) -> String {
        let c = &self.credits;
        let mut lines = Vec::new();
        if c.unified == Some(true) {
            lines.push("Shared account credit allowance".into());
        }
        if let Some(plan) = &c.plan {
            lines.push(format!("Plan: {plan}"));
        }
        let reset = c
            .reset_at
            .as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|time| {
                format!(
                    "resets {}",
                    time.with_timezone(&chrono::Utc)
                        .format("%Y-%m-%d %H:%M UTC")
                )
            })
            .unwrap_or("reset time unavailable".into());
        if let Some(percent) = c.percent {
            lines.push(format!(
                "{} credits: {:.1}% used · {reset}",
                c.period.as_deref().unwrap_or("Included"),
                percent
            ));
            lines.push(format!(
                "Remaining allowance: {:.1}%",
                (100.0 - percent).max(0.0)
            ));
        } else {
            lines.push("Included credit usage percentage unavailable".into());
        }
        if let Some(used) = c.used_cents {
            lines.push(format!(
                "Included used: {}{}",
                dollars(used),
                c.limit_cents
                    .map(|limit| format!(" / {}", dollars(limit)))
                    .unwrap_or_default()
            ));
        }
        if let Some(balance) = c.prepaid_cents {
            lines.push(format!("Prepaid balance: {}", dollars(balance)));
        }
        if let Some(used) = c.on_demand_used_cents {
            lines.push(format!(
                "On-demand used: {}{}",
                dollars(used),
                c.on_demand_cap_cents
                    .map(|cap| format!(" / {} cap", dollars(cap)))
                    .unwrap_or_default()
            ));
        }
        if c.percent.is_none() && c.reset_at.is_some() {
            lines.push(reset);
        }
        if let Some(time) = chrono::DateTime::from_timestamp(self.fetched_at, 0) {
            lines.push(format!(
                "Last refresh: {} · r refresh",
                time.format("%Y-%m-%d %H:%M UTC")
            ));
        }
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
    };
    #[test]
    fn modern_and_legacy_credit_shapes_preserve_missing_and_zero_values() {
        let current = parse(&json!({"config": {"creditUsagePercent": 25.5, "currentPeriod": {"type": "USAGE_PERIOD_TYPE_WEEKLY", "end": "2026-10-01T00:00:00Z"}, "prepaidBalance": {"val": 1234}, "onDemandUsed": {}, "onDemandCap": {"val": "5000"}}, "subscription_tier":"SuperGrok"})).unwrap();
        assert_eq!(current.percent, Some(25.5));
        assert_eq!(current.period.as_deref(), Some("Weekly"));
        assert_eq!(current.on_demand_used_cents, Some(0));
        assert_eq!(current.used_cents, None);
        let summary = Snapshot {
            account: "a".into(),
            credits: current,
            fetched_at: 1,
        }
        .summary();
        assert!(summary.contains("25.5% used") && summary.contains("74.5%"));
        assert!(summary.contains("$12.34") && summary.contains("$0.00 / $50.00"));
        assert!(summary.contains("2026-10-01 00:00 UTC"));
        let legacy = parse(&json!({"config":{"monthlyLimit":{"val": 10000},"used":{"val":2500},"billingPeriodEnd":"2026-10-01T00:00:00Z"}})).unwrap();
        assert_eq!(legacy.percent, Some(25.0));
        assert_eq!(legacy.period.as_deref(), Some("Monthly"));
        assert_eq!(
            parse(&json!({"config":{"monthlyLimit":{},"used":{}}}))
                .unwrap()
                .percent,
            None
        );
        for response in [
            json!({"config":null}),
            json!({"config":{}}),
            json!({"config":{"creditUsagePercent":-1}}),
            json!({"config":{"used":{"val":"SECRET"}}}),
        ] {
            let error = parse(&response).unwrap_err().to_string();
            assert!(!error.contains("SECRET"));
        }
    }
    fn server(status: &str, body: String) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!(
            "http://{}/billing?format=credits",
            listener.local_addr().unwrap()
        );
        let status = status.to_owned();
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(socket.try_clone().unwrap());
            let mut request = String::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                request.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let content_length = request
                .lines()
                .find_map(|line| {
                    line.to_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|value| value.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            let mut body_bytes = vec![0; content_length];
            reader.read_exact(&mut body_bytes).unwrap();
            request.push_str(&String::from_utf8(body_bytes).unwrap());
            write!(socket,"HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            request
        });
        (endpoint, worker)
    }
    #[test]
    fn wake_posts_only_a_bounded_prompt_and_never_echoes_error_bodies() {
        let entry = json!({"key":"TEST-TOKEN","user_id":"user"});
        let (endpoint, worker) = server(
            "200 OK",
            json!({"choices":[{"finish_reason":"length"}]}).to_string(),
        );
        wake_at(&endpoint, &entry).unwrap();
        let request = worker.join().unwrap();
        assert!(request.starts_with("POST "));
        assert!(
            request
                .to_lowercase()
                .contains("x-grok-model-override: grok-build")
        );
        let body: Value = serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(body["max_tokens"], 1);
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
        assert_eq!(body["messages"][0]["content"], "Reply OK.");
        assert_eq!(body["stream"], false);
        assert!(body.get("tools").is_none());
        for status in ["401 Unauthorized", "302 Found", "503 Unavailable"] {
            let (endpoint, worker) = server(status, "TEST-TOKEN".into());
            let error = wake_at(&endpoint, &entry).unwrap_err().to_string();
            assert!(!error.contains("TEST-TOKEN"));
            worker.join().unwrap();
        }
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join("auth.json"),
            json!({"auth_mode":"oidc","key":"TEST-TOKEN"}).to_string(),
        )
        .unwrap();
        assert!(
            wake(home.path(), "different-account")
                .unwrap_err()
                .to_string()
                .contains("changed")
        );
    }

    #[test]
    fn billing_get_authenticates_without_exposing_tokens_or_error_bodies() {
        let entry = json!({"key":"TEST-TOKEN","user_id":"test-user"});
        let (endpoint, server) = server(
            "200 OK",
            json!({"config":{"creditUsagePercent": 40}}).to_string(),
        );
        assert_eq!(fetch_at(&endpoint, &entry).unwrap().percent, Some(40.0));
        let request = server.join().unwrap().to_lowercase();
        assert!(request.starts_with("get /billing?format=credits "));
        assert!(request.contains("authorization: bearer test-token"));
        assert!(request.contains("x-xai-token-auth: xai-grok-cli"));
        assert!(request.contains("x-userid: test-user"));
        for (code, body) in [
            ("401 Unauthorized", "TEST-TOKEN"),
            ("503 Unavailable", "TEST-TOKEN"),
            ("200 OK", "TEST-TOKEN invalid JSON"),
            ("302 Found", "TEST-TOKEN"),
        ] {
            let (endpoint, worker) = super::tests::server(code, body.into());
            let error = fetch_at(&endpoint, &entry).unwrap_err().to_string();
            worker.join().unwrap();
            assert!(!error.contains("TEST-TOKEN"));
        }
    }
    #[test]
    fn account_cache_identity_changes_only_when_account_changes() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("auth.json");
        assert!(account(home.path()).unwrap().is_none());
        fs_write(
            &path,
            &json!({"auth_mode":"oidc","key":"SECRET","user_id":"one"}),
        );
        let original = account(home.path()).unwrap();
        fs_write(
            &path,
            &json!({"auth_mode":"oidc","key":"NEW-TOKEN","user_id":"one"}),
        );
        assert_eq!(account(home.path()).unwrap(), original);
        fs_write(
            &path,
            &json!({"auth_mode":"oidc","key":"NEW-TOKEN","user_id":"two"}),
        );
        assert_ne!(account(home.path()).unwrap(), original);
        fs_write(&path, &json!({"auth_mode":"api_key","key":"SECRET"}));
        assert!(account(home.path()).unwrap().is_none());
    }
    fn fs_write(path: &Path, value: &Value) {
        std::fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
    }
}

#[cfg(test)]
mod fetch_tests {
    use super::*;
    #[test]
    fn missing_expired_and_custom_issuer_credentials_are_not_sent_or_modified() {
        let home = tempfile::tempdir().unwrap();
        assert!(
            fetch(home.path())
                .unwrap_err()
                .to_string()
                .contains("Sign in")
        );
        for (json, message) in [
            (
                r#"{"auth_mode":"oidc","key":"SECRET","expires_at":"2000-01-01T00:00:00Z"}"#,
                "expired",
            ),
            (
                r#"{"auth_mode":"oidc","key":"SECRET","oidc_issuer":"https://enterprise.example"}"#,
                "custom OAuth issuer",
            ),
        ] {
            let path = home.path().join("auth.json");
            std::fs::write(&path, json).unwrap();
            let error = fetch(home.path()).unwrap_err().to_string();
            assert!(error.contains(message) && !error.contains("SECRET"));
            assert_eq!(std::fs::read_to_string(path).unwrap(), json);
        }
    }
    #[test]
    #[ignore = "read-only live billing query using the saved Grok OAuth login"]
    fn grok_live_usage() {
        let home = crate::grok::home().unwrap();
        let before = std::fs::read(home.join("auth.json")).unwrap();
        let snapshot = fetch(&home).unwrap();
        assert!(snapshot.credits.percent.is_some() || snapshot.credits.prepaid_cents.is_some());
        assert_eq!(std::fs::read(home.join("auth.json")).unwrap(), before);
    }
}
