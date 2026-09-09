use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use std::{
    sync::atomic::AtomicBool,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub name: String,
    pub email: String,
    pub workspace: String,
    pub subject: String,
    pub plan: Option<String>,
    #[serde(default, with = "json_string")]
    pub limits: Value,
    pub refreshed_at: Option<u64>,
    pub error: Option<String>,
}
pub struct Identity {
    pub id: String,
    pub account: Account,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn account_dir(paths: &AppPaths, id: &str) -> Result<PathBuf> {
    if id.len() != 32 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("Invalid account ID");
    }
    Ok(paths.state_dir.join("codex-accounts").join(id))
}
fn decode_claims(token: &str) -> Result<Value> {
    let payload = token.split('.').nth(1).context("Invalid login token")?;
    let decoded = URL_SAFE_NO_PAD
        .decode(payload)
        .context("Invalid login token encoding")?;
    serde_json::from_slice(&decoded).context("Invalid login token claims")
}
// Claims identify local snapshots only. Codex validates credentials against OpenAI.
pub fn identity(auth: &Value) -> Result<Identity> {
    let tokens = auth
        .get("tokens")
        .context("Not a ChatGPT subscription login; use API Providers for API keys")?;
    let claims = decode_claims(tokens["id_token"].as_str().context("Missing ID token")?)?;
    if tokens["access_token"].as_str().is_none_or(str::is_empty)
        || tokens["refresh_token"].as_str().is_none_or(str::is_empty)
    {
        bail!("Incomplete subscription login; sign in again");
    }
    let subject = claims["sub"]
        .as_str()
        .context("Missing account subject")?
        .to_owned();
    let details = &claims["https://api.openai.com/auth"];
    let workspace = tokens["account_id"]
        .as_str()
        .or(details["chatgpt_account_id"].as_str())
        .context("Missing workspace identity")?
        .to_owned();
    let id = format!("{:x}", Sha256::digest(format!("{subject}\0{workspace}")))[..32].to_owned();
    Ok(Identity {
        id,
        account: Account {
            name: String::new(),
            email: claims["email"].as_str().unwrap_or("unknown").into(),
            workspace,
            subject,
            plan: details["chatgpt_plan_type"].as_str().map(str::to_owned),
            ..Default::default()
        },
    })
}
fn keyring_entry(home: &Path) -> Result<keyring::Entry> {
    let home = fs::canonicalize(home).unwrap_or(home.to_path_buf());
    let hash = format!("{:x}", Sha256::digest(home.to_string_lossy().as_bytes()));
    keyring::Entry::new("Codex Auth", &format!("cli|{}", &hash[..16]))
        .context("Cannot access Codex credential store")
}
fn store(doc: &DocumentMut) -> &str {
    doc.get("cli_auth_credentials_store")
        .and_then(Item::as_str)
        .unwrap_or("file")
}
pub(super) fn read_live_auth(home: &Path, doc: &DocumentMut) -> Result<Option<Value>> {
    let storage = store(doc);
    if storage == "ephemeral" {
        bail!(
            "In-memory Codex authentication cannot be imported or switched; choose persistent storage in Codex first"
        );
    }
    if storage == "keyring" || storage == "auto" {
        match keyring_entry(home)?.get_password() {
            Ok(secret) => {
                return Ok(Some(
                    serde_json::from_str(&secret).context("Invalid Codex credential store data")?,
                ));
            }
            Err(keyring::Error::NoEntry) => {
                if storage == "keyring" {
                    return Ok(None);
                }
            }
            Err(_) if storage == "auto" => {}
            Err(_) => bail!("Codex system credential store is unavailable or locked"),
        }
    }
    let path = home.join("auth.json");
    if path.exists() {
        read_auth(&path).map(Some)
    } else {
        Ok(None)
    }
}
pub(super) fn write_live_auth(home: &Path, doc: &DocumentMut, auth: &Value) -> Result<()> {
    if store(doc) == "ephemeral" {
        bail!("Cannot persist an ephemeral Codex login");
    }
    if ["keyring", "auto"].contains(&store(doc)) {
        match keyring_entry(home)?.set_password(&serde_json::to_string(auth)?) {
            Ok(()) => return Ok(()),
            Err(_) if store(doc) == "auto" => {}
            Err(_) => bail!("Cannot write Codex system credential store"),
        }
    }
    atomic_write(&home.join("auth.json"), &serde_json::to_vec_pretty(auth)?)
}
fn read_auth(path: &Path) -> Result<Value> {
    if fs::metadata(path)?.len() > 1024 * 1024 {
        bail!("Auth file is too large");
    }
    serde_json::from_slice(&fs::read(path)?).context("Invalid auth.json")
}
fn save_snapshot(paths: &AppPaths, name: &str, auth: &Value) -> Result<String> {
    if name.trim().is_empty() {
        bail!("Account name is required");
    }
    let Identity { id, mut account } = identity(auth)?;
    let dir = account_dir(paths, &id)?;
    private_dir(&dir)?;
    atomic_write(&dir.join("auth.json"), &serde_json::to_vec_pretty(auth)?)?;
    atomic_write(
        &dir.join("config.toml"),
        b"cli_auth_credentials_store = \"file\"\nmodel_provider = \"openai\"\n",
    )?;
    account.name = name.trim().into();
    config::update(&paths.config, |config| {
        if let Some(old) = config.codex.accounts.get(&id) {
            account.limits = old.limits.clone();
            account.refreshed_at = old.refreshed_at;
        }
        config.codex.accounts.insert(id.clone(), account);
        Ok(())
    })?;
    Ok(id)
}
pub fn import(paths: &AppPaths, name: &str, file: Option<&Path>) -> Result<String> {
    let _guard = lock(paths)?;
    let auth = if let Some(file) = file {
        read_auth(file)?
    } else {
        let home = home()?;
        read_live_auth(&home, &document(&home)?)?.context("No local Codex login found")?
    };
    save_snapshot(paths, name, &auth)
}
pub fn login(
    paths: &AppPaths,
    name: &str,
    device: bool,
    cancel: &AtomicBool,
    notify: impl Fn(String),
) -> Result<String> {
    if name.trim().is_empty() {
        bail!("Account name is required");
    }
    private_dir(&paths.state_dir)?;
    let temp = tempfile::tempdir_in(&paths.state_dir)?;
    private_dir(temp.path())?;
    atomic_write(
        &temp.path().join("config.toml"),
        b"cli_auth_credentials_store = \"file\"\n",
    )?;
    let mut client = rpc::Client::start(temp.path())?;
    let response = client.call(
        "account/login/start",
        json!({"type":if device {"chatgptDeviceCode"} else {"chatgpt"}}),
    )?;
    let login_id = response["loginId"]
        .as_str()
        .context("Codex did not return a login ID")?;
    if device {
        notify(format!(
            "Open {} and enter {}",
            response["verificationUrl"]
                .as_str()
                .unwrap_or("the Codex login page"),
            response["userCode"].as_str().unwrap_or("")
        ));
    } else {
        let url = response["authUrl"]
            .as_str()
            .context("Codex did not return a browser login URL")?;
        // The URL originates from the installed official Codex client; never invoke a shell.
        let parsed = url::Url::parse(url)?;
        if parsed.scheme() != "https"
            || !parsed.host_str().is_some_and(|h| {
                h == "auth.openai.com" || h == "chatgpt.com" || h.ends_with(".openai.com")
            })
        {
            bail!("Codex returned an unexpected login URL");
        }
        notify(format!("Complete browser login: {url}"));
        let _ = open_browser(url);
    }
    client.wait_login(login_id, cancel)?;
    let auth = read_auth(&temp.path().join("auth.json"))?;
    let _guard = lock(paths)?;
    save_snapshot(paths, name, &auth)
}
fn open_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut c = std::process::Command::new("rundll32.exe");
        c.arg("url.dll,FileProtocolHandler");
        c
    };
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let mut command = std::process::Command::new("xdg-open");
    command
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    Ok(())
}
pub(super) fn capture_current(paths: &AppPaths, auth: Option<&Value>) -> Result<()> {
    let Some(auth) = auth else {
        return Ok(());
    };
    let Ok(identity) = identity(auth) else {
        return Ok(());
    };
    let config = config::load(&paths.config)?;
    if config.codex.accounts.contains_key(&identity.id) {
        let dir = account_dir(paths, &identity.id)?;
        private_dir(&dir)?;
        atomic_write(&dir.join("auth.json"), &serde_json::to_vec_pretty(auth)?)?;
    }
    Ok(())
}
pub fn activate(paths: &AppPaths, id: &str) -> Result<()> {
    let _guard = lock(paths)?;
    let config = config::load(&paths.config)?;
    if !config.codex.accounts.contains_key(id) {
        bail!("Account does not exist");
    }
    let home = home()?;
    let old = document(&home)?;
    capture_current(paths, read_live_auth(&home, &old)?.as_ref())?;
    let auth = read_auth(&account_dir(paths, id)?.join("auth.json"))?;
    if identity(&auth)?.id != id {
        bail!("Saved credentials belong to another account; reimport this account");
    }
    if let Some(method) = old.get("forced_login_method").and_then(Item::as_str)
        && method != "chatgpt"
    {
        bail!("Codex forces API login; subscription switching is disabled");
    }
    if let Some(workspace) = old
        .get("forced_chatgpt_workspace_id")
        .and_then(Item::as_str)
        && workspace != config.codex.accounts[id].workspace
    {
        bail!("Account does not match Codex's enforced workspace");
    }
    let mut new = old.clone();
    new["model_provider"] = value("openai");
    new.remove("openai_base_url");
    if let Some(binding) = super::binding(paths)? {
        let baseline: DocumentMut = binding.before.parse()?;
        super::restore_key(&mut new, &baseline, "web_search");
    }
    let previous = super::binding(paths)?
        .map(|binding| binding.before.parse::<DocumentMut>())
        .transpose()?
        .unwrap_or(old.clone());
    if let Some(model) = config
        .codex
        .subscription_model
        .as_deref()
        .or_else(|| previous.get("model").and_then(Item::as_str))
    {
        new["model"] = value(model);
    } else {
        new.remove("model");
    }
    for key in [
        "model_context_window",
        "model_auto_compact_token_limit",
        "model_reasoning_effort",
    ] {
        super::restore_key(&mut new, &previous, key);
    }
    if let Some(effort) = &config.codex.subscription_reasoning {
        new["model_reasoning_effort"] = value(effort);
    }
    commit(
        paths,
        &home,
        &old,
        &new,
        Some(&auth),
        Selection::Account { id: id.into() },
    )
}
pub fn rename(paths: &AppPaths, id: &str, name: &str) -> Result<()> {
    if name.trim().is_empty() {
        bail!("Account name is required");
    }
    let _guard = lock(paths)?;
    config::update(&paths.config, |config| {
        config
            .codex
            .accounts
            .get_mut(id)
            .context("Account does not exist")?
            .name = name.trim().into();
        Ok(())
    })?;
    Ok(())
}
pub fn remove(paths: &AppPaths, id: &str) -> Result<()> {
    let _guard = lock(paths)?;
    let config = config::load(&paths.config)?;
    if config.codex.active == Some(Selection::Account { id: id.into() }) {
        bail!("Switch accounts or disconnect before deleting the selected account");
    }
    let dir = account_dir(paths, id)?;
    config::update(&paths.config, |config| {
        config
            .codex
            .accounts
            .remove(id)
            .context("Account does not exist")?;
        Ok(())
    })?;
    for file in ["auth.json", "config.toml"] {
        let path = dir.join(file);
        if path.exists() {
            fs::remove_file(path)?;
        }
    }
    let _ = fs::remove_dir(dir);
    Ok(())
}
pub fn refresh(paths: &AppPaths, id: &str) -> Result<()> {
    let _guard = lock(paths)?;
    let home = home()?;
    let doc = document(&home)?;
    capture_current(paths, read_live_auth(&home, &doc)?.as_ref())?;
    let original = read_auth(&account_dir(paths, id)?.join("auth.json"))?;
    if identity(&original)?.id != id {
        bail!("Account snapshot identity mismatch");
    }
    let temp = tempfile::tempdir_in(&paths.state_dir)?;
    private_dir(temp.path())?;
    atomic_write(
        &temp.path().join("auth.json"),
        &serde_json::to_vec(&original)?,
    )?;
    atomic_write(
        &temp.path().join("config.toml"),
        b"cli_auth_credentials_store = \"file\"\n",
    )?;
    let result = (|| {
        let mut client = rpc::Client::start(temp.path())?;
        let info = client.call("account/read", json!({"refreshToken":true}))?;
        if info["account"]["type"] != "chatgpt" {
            bail!("Subscription login expired; sign in again");
        }
        let limits = client.call("account/rateLimits/read", json!({}))?;
        Ok::<_, anyhow::Error>((info, limits))
    })();
    // Always retain refreshed tokens, even if the quota endpoint failed.
    let refreshed = read_auth(&temp.path().join("auth.json"))?;
    if identity(&refreshed)?.id != id {
        bail!("Codex refreshed a different account; no credentials changed");
    }
    atomic_write(
        &account_dir(paths, id)?.join("auth.json"),
        &serde_json::to_vec(&refreshed)?,
    )?;
    // Compare before writing: an externally switched account must never be replaced.
    if read_live_auth(&home, &doc)?.as_ref() == Some(&original) && refreshed != original {
        write_live_auth(&home, &doc, &refreshed)?;
    }
    config::update(&paths.config, |config| {
        let account = config
            .codex
            .accounts
            .get_mut(id)
            .context("Account no longer exists")?;
        match &result {
            Ok((info, limits)) => {
                account.plan = info["account"]["planType"]
                    .as_str()
                    .map(str::to_owned)
                    .or(account.plan.clone());
                account.limits = limits.clone();
                account.refreshed_at = Some(now());
                account.error = None;
            }
            Err(_) => {
                account.error = Some(
                    "Refresh failed; cached limits may be stale. Check network or sign in again."
                        .into(),
                )
            }
        }
        Ok(())
    })?;
    result.map(|_| ())
}
pub fn summary(paths: &AppPaths, id: &str) -> Result<String> {
    let config = config::load(&paths.config)?;
    let account = config
        .codex
        .accounts
        .get(id)
        .context("Account does not exist")?;
    let mut lines = vec![
        format!(
            "{} · {} · {}",
            account.name,
            account.email,
            account.plan.as_deref().unwrap_or("unknown plan")
        ),
        format!("Workspace: {}", account.workspace),
    ];
    let buckets = account.limits["rateLimitsByLimitId"]
        .as_object()
        .cloned()
        .unwrap_or_else(|| {
            let mut map = serde_json::Map::new();
            if account.limits["rateLimits"].is_object() {
                map.insert("codex".into(), account.limits["rateLimits"].clone());
            }
            map
        });
    if buckets.is_empty() {
        lines.push("Limits: unknown · r refresh".into());
    }
    for (name, bucket) in buckets {
        for window in ["primary", "secondary"] {
            if let Some(used) = bucket[window]["usedPercent"].as_i64() {
                let reset = bucket[window]["resetsAt"]
                    .as_u64()
                    .map(|time| format!("in {} min", time.saturating_sub(now()).div_ceil(60)))
                    .unwrap_or("unknown".into());
                lines.push(format!("{name} {window}: {used}% used · resets {reset}"));
            }
        }
    }
    if let Some(time) = account.refreshed_at {
        lines.push(format!(
            "Updated {} min ago",
            now().saturating_sub(time) / 60
        ));
    }
    if let Some(error) = &account.error {
        lines.push(error.clone());
    }
    Ok(lines.join("\n"))
}

mod json_string {
    use serde::{Deserialize, Deserializer, Serializer};
    use serde_json::Value;
    pub fn serialize<S: Serializer>(value: &Value, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Value, D::Error> {
        let value = String::deserialize(deserializer)?;
        serde_json::from_str(&value).map_err(serde::de::Error::custom)
    }
}

pub(super) fn latest_snapshot(paths: &AppPaths, original: Option<&Value>) -> Result<Option<Value>> {
    if let Some(auth) = original
        && let Ok(identity) = identity(auth)
    {
        let path = account_dir(paths, &identity.id)?.join("auth.json");
        if path.exists() {
            let saved = read_auth(&path)?;
            if self::identity(&saved)?.id == identity.id {
                return Ok(Some(saved));
            }
        }
    }
    Ok(original.cloned())
}
pub(super) fn clear_live_auth(home: &Path, doc: &DocumentMut) -> Result<()> {
    if ["keyring", "auto"].contains(&store(doc)) {
        match keyring_entry(home)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(_) => bail!("Cannot clear managed Codex credentials from the system store"),
        }
    }
    let path = home.join("auth.json");
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}
