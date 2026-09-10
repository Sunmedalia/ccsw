//! Codex settings and account operations shared by the CLI and TUI.
pub mod accounts;
mod rpc;
use crate::{
    config::{self, AppPaths},
    proxy,
};
use anyhow::{Context, Result, bail};
use clap::Subcommand;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use toml_edit::{DocumentMut, Item, value};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub profiles: BTreeMap<String, config::Profile>,
    #[serde(default)]
    pub accounts: BTreeMap<String, accounts::Account>,
    pub active: Option<Selection>,
    pub api_model: Option<String>,
    pub subscription_model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub subscription_reasoning: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Selection {
    Api { profile: String, model: String },
    Account { id: String },
}
#[derive(Subcommand)]
pub enum Command {
    /// Show desired and on-disk Codex configuration (no secrets)
    Status,
    /// Apply an existing provider and model to Codex CLI and desktop
    Apply {
        #[arg(long)]
        profile: String,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        reasoning: Option<String>,
    },
    /// Restore the settings and credentials saved before CCSW took control
    Disconnect,
    /// Roll back an interrupted Codex file transaction
    Recover,
    /// Manage subscription accounts
    Accounts {
        #[command(subcommand)]
        command: AccountCommand,
    },
}
#[derive(Subcommand)]
pub enum AccountCommand {
    List,
    Login {
        #[arg(long)]
        name: String,
        #[arg(long)]
        device: bool,
    },
    Import {
        #[arg(long)]
        name: String,
        #[arg(long)]
        file: Option<PathBuf>,
    },
    Use {
        id: String,
    },
    Rename {
        id: String,
        name: String,
    },
    Remove {
        id: String,
    },
    Refresh {
        id: String,
    },
}
pub fn run(paths: &AppPaths, command: Command) -> Result<()> {
    match command {
        Command::Recover => {
            recover(paths)?;
            println!("Interrupted Codex transaction recovered; restart clients.");
        }
        Command::Status => println!("{}", status(paths)?),
        Command::Apply {
            profile,
            model,
            reasoning,
        } => {
            apply(paths, &profile, model.as_deref(), reasoning.as_deref())?;
            println!(
                "Codex configuration applied. Restart CLI / ChatGPT App; existing chats retain their settings."
            );
        }
        Command::Disconnect => {
            disconnect(paths)?;
            println!("Restored previous Codex settings. Restart CLI / ChatGPT App.");
        }
        Command::Accounts { command } => match command {
            AccountCommand::List => {
                for (id, a) in config::load(&paths.config)?.codex.accounts {
                    println!(
                        "{id}\t{}\t{}\t{}",
                        a.name,
                        a.email,
                        a.plan.as_deref().unwrap_or("unknown")
                    );
                }
            }
            AccountCommand::Login { name, device } => {
                let id = accounts::login(
                    paths,
                    &name,
                    device,
                    &std::sync::atomic::AtomicBool::new(false),
                    |message| println!("{message}"),
                )?;
                println!("Saved account {id}; use `ccsw codex accounts use {id}` to switch.");
            }
            AccountCommand::Import { name, file } => println!(
                "Saved account {}",
                accounts::import(paths, &name, file.as_deref())?
            ),
            AccountCommand::Use { id } => {
                accounts::activate(paths, &id)?;
                println!(
                    "Account selected. Restart CLI / ChatGPT App, then check `ccsw codex status`."
                );
            }
            AccountCommand::Rename { id, name } => accounts::rename(paths, &id, &name)?,
            AccountCommand::Remove { id } => accounts::remove(paths, &id)?,
            AccountCommand::Refresh { id } => {
                accounts::refresh(paths, &id)?;
                println!("{}", accounts::summary(paths, &id)?);
            }
        },
    }
    Ok(())
}
pub fn home() -> Result<PathBuf> {
    let path = std::env::var_os("CODEX_HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or(crate::platform::home()?.join(".codex"));
    Ok(std::path::absolute(path)?)
}
pub(super) fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
pub(super) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("Missing parent directory")?;
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    config::set_private(temp.path())?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
pub(super) struct OperationLock {
    _state: File,
    _home: File,
}
pub(super) fn lock(paths: &AppPaths) -> Result<OperationLock> {
    fs::create_dir_all(&paths.state_dir)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(paths.state_dir.join("codex.lock"))?;
    config::set_private(&paths.state_dir.join("codex.lock"))?;
    file.try_lock_exclusive()
        .context("Another Codex operation is running; retry when it finishes")?;
    let codex_home = home()?;
    fs::create_dir_all(&codex_home)?;
    let home_path = codex_home.join(".ccsw.lock");
    let home_lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&home_path)?;
    config::set_private(&home_path)?;
    home_lock
        .try_lock_exclusive()
        .context("Another CCSW configuration is editing this CODEX_HOME")?;
    Ok(OperationLock {
        _state: file,
        _home: home_lock,
    })
}
pub(super) fn document(home: &Path) -> Result<DocumentMut> {
    let path = home.join("config.toml");
    let text = if path.exists() {
        fs::read_to_string(path)?
    } else {
        String::new()
    };
    text.parse()
        .context("Invalid Codex config.toml; no files changed")
}
const KEYS: &[&str] = &[
    "model",
    "model_provider",
    "model_reasoning_effort",
    "model_catalog_json",
    "model_context_window",
    "model_auto_compact_token_limit",
    "cli_auth_credentials_store",
    "model_providers.ccsw",
    "openai_base_url",
    "web_search",
];
fn get(doc: &DocumentMut, key: &str) -> Option<String> {
    let root: toml::Value = doc.to_string().parse().ok()?;
    let item = if key == "model_providers.ccsw" {
        root.get("model_providers")?.get("ccsw")?
    } else {
        root.get(key)?
    };
    serde_json::to_string(item).ok()
}
#[derive(Default, Serialize, Deserialize, Clone)]
struct Binding {
    home: PathBuf,
    before: String,
    original_auth: Option<Value>,
    managed: BTreeMap<String, Option<String>>,
    expected_account: Option<String>,
}
fn binding_path(paths: &AppPaths) -> PathBuf {
    paths.state_dir.join("codex-binding.json")
}
fn binding(paths: &AppPaths) -> Result<Option<Binding>> {
    let path = binding_path(paths);
    if path.exists() {
        Ok(Some(
            serde_json::from_slice(&fs::read(path)?).context("Invalid Codex binding state")?,
        ))
    } else {
        Ok(None)
    }
}
fn check_managed(binding: &Binding, home: &Path, doc: &DocumentMut) -> Result<()> {
    if binding.home != home {
        bail!("CODEX_HOME changed; disconnect the previous home before applying to another home");
    }
    for (key, expected) in &binding.managed {
        if !matches!(key.as_str(), "model" | "model_reasoning_effort") && get(doc, key) != *expected
        {
            bail!(
                "Codex setting {key} changed outside CCSW; restore it or run ccsw codex disconnect (TUI: D) before applying"
            );
        }
    }
    Ok(())
}
#[derive(Serialize, Deserialize)]
struct Transaction {
    home: PathBuf,
    old_config: String,
    new_config: String,
    old_auth: Option<Value>,
    new_auth: Option<Value>,
    old_binding: Option<Binding>,
    old_selection: Option<Selection>,
    #[serde(default)]
    old_settings: Settings,
    new_selection: Selection,
}
fn journal_path(paths: &AppPaths) -> PathBuf {
    paths.state_dir.join("codex-transaction.json")
}
fn restore_auth(home: &Path, doc: &DocumentMut, auth: Option<&Value>) -> Result<()> {
    if let Some(auth) = auth {
        accounts::write_live_auth(home, doc, auth)
    } else {
        accounts::clear_live_auth(home, doc)
    }
}
fn rollback(paths: &AppPaths, transaction: &Transaction) -> Result<()> {
    let current = document(&transaction.home)?;
    let old: DocumentMut = transaction.old_config.parse()?;
    let new: DocumentMut = transaction.new_config.parse()?;
    // Preserve unrelated edits made while the interrupted operation was running.
    let mut restored = current.clone();
    for key in KEYS {
        if get(&current, key) != get(&new, key) && get(&current, key) != get(&old, key) {
            bail!(
                "Recovery conflict in {key}; preserve the recovery journal and resolve external edits first"
            );
        }
        restore_key(&mut restored, &old, key);
    }
    let live = accounts::read_live_auth(&transaction.home, &current)?;
    if transaction.new_auth.is_some() {
        if live != transaction.new_auth && live != transaction.old_auth {
            bail!(
                "Recovery found credentials changed by another client; close clients before resolving the transaction"
            );
        }
        restore_auth(&transaction.home, &old, transaction.old_auth.as_ref())?;
    }
    atomic_write(
        &transaction.home.join("config.toml"),
        restored.to_string().as_bytes(),
    )?;
    if let Some(binding) = &transaction.old_binding {
        atomic_write(&binding_path(paths), &serde_json::to_vec(binding)?)?;
    } else if binding_path(paths).exists() {
        fs::remove_file(binding_path(paths))?;
    }
    if config::load(&paths.config)?.codex.active != transaction.old_selection {
        config::update(&paths.config, |config| {
            if config.codex.active != Some(transaction.new_selection.clone())
                && config.codex.active != transaction.old_selection
            {
                bail!("CCSW selection changed during recovery");
            }
            config.codex.active = transaction.old_selection.clone();
            config.codex.api_model = transaction.old_settings.api_model.clone();
            config.codex.subscription_model = transaction.old_settings.subscription_model.clone();
            config.codex.reasoning_effort = transaction.old_settings.reasoning_effort.clone();
            config.codex.subscription_reasoning =
                transaction.old_settings.subscription_reasoning.clone();
            Ok(())
        })?;
    }
    fs::remove_file(journal_path(paths))?;
    Ok(())
}
pub fn recover(paths: &AppPaths) -> Result<()> {
    let _guard = lock(paths)?;
    let transaction: Transaction = serde_json::from_slice(
        &fs::read(journal_path(paths)).context("No recovery journal exists")?,
    )?;
    if transaction.home != home()? {
        bail!("Use the original CODEX_HOME to recover this transaction");
    }
    rollback(paths, &transaction)
}
fn ensure_providers(doc: &mut DocumentMut) -> Result<()> {
    if doc.get("model_providers").is_none() {
        doc["model_providers"] = Item::Table(toml_edit::Table::new());
    }
    if let Some(item) = doc.get_mut("model_providers") {
        if let Some(inline) = item.as_inline_table() {
            *item = Item::Table(inline.clone().into_table());
        }
        if !item.is_table() {
            bail!("model_providers must be a table");
        }
    }
    Ok(())
}
fn restore_key(current: &mut DocumentMut, original: &DocumentMut, key: &str) {
    if key == "model_providers.ccsw" {
        if current.get("model_providers").is_none() && original.get("model_providers").is_none() {
            return;
        }
        let _ = ensure_providers(current);
        if let Some(table) = current
            .get_mut("model_providers")
            .and_then(Item::as_table_mut)
        {
            table.remove("ccsw");
        }
        if let Some(item) = original.get("model_providers").and_then(|i| i.get("ccsw")) {
            current["model_providers"]["ccsw"] = item.clone();
        }
        if original.get("model_providers").is_none()
            && current
                .get("model_providers")
                .and_then(Item::as_table)
                .is_some_and(toml_edit::Table::is_empty)
        {
            current.remove("model_providers");
        }
    } else if let Some(item) = original.get(key) {
        current[key] = item.clone();
    } else {
        current.remove(key);
    }
}
pub(super) fn commit(
    paths: &AppPaths,
    home: &Path,
    old: &DocumentMut,
    new: &DocumentMut,
    auth: Option<&Value>,
    selection: Selection,
) -> Result<()> {
    if journal_path(paths).exists() {
        bail!("An interrupted transaction needs recovery: ccsw codex recover");
    }
    if document(home)?.to_string() != old.to_string() {
        bail!("Codex configuration changed during preparation; retry");
    }
    let previous_auth = accounts::read_live_auth(home, old)?;
    let old_binding = binding(paths)?;
    let old_settings = config::load(&paths.config)?.codex;
    let old_selection = old_settings.active.clone();
    let mut state = old_binding.clone().unwrap_or(Binding {
        home: home.to_path_buf(),
        before: old.to_string(),
        original_auth: previous_auth.clone(),
        ..Default::default()
    });
    check_managed(&state, home, old)?;
    // Explicit selections may replace a model chosen in Codex. Preserve that
    // external choice as the baseline for a later disconnect.
    let mut baseline: DocumentMut = state.before.parse()?;
    for key in ["model", "model_reasoning_effort"] {
        if state
            .managed
            .get(key)
            .is_some_and(|expected| get(old, key) != *expected)
        {
            restore_key(&mut baseline, old, key);
        }
    }
    state.before = baseline.to_string();
    let replacing_same_account = auth.is_some()
        && matches!(&selection,
        Selection::Account { id } if previous_auth.as_ref().and_then(|a| accounts::identity(a).ok()).is_some_and(|a| &a.id == id));
    if !replacing_same_account {
        accounts::capture_current(paths, previous_auth.as_ref())?;
    }
    state.managed = KEYS
        .iter()
        .map(|key| ((*key).into(), get(new, key)))
        .collect();
    if let Selection::Account { id } = &selection {
        state.expected_account = Some(id.clone());
    }
    let transaction = Transaction {
        home: home.to_path_buf(),
        old_config: old.to_string(),
        new_config: new.to_string(),
        old_auth: previous_auth.clone(),
        new_auth: auth.cloned(),
        old_binding,
        old_selection,
        old_settings,
        new_selection: selection.clone(),
    };
    atomic_write(&journal_path(paths), &serde_json::to_vec(&transaction)?)?;
    let result = (|| {
        if let Some(auth) = auth {
            accounts::write_live_auth(home, new, auth)?;
        }
        atomic_write(&home.join("config.toml"), new.to_string().as_bytes())?;
        atomic_write(&binding_path(paths), &serde_json::to_vec(&state)?)?;
        config::update(&paths.config, |config| {
            if let Selection::Api { model, .. } = &selection {
                config.codex.api_model = Some(model.clone());
                config.codex.reasoning_effort = new
                    .get("model_reasoning_effort")
                    .and_then(Item::as_str)
                    .map(str::to_owned);
            }
            if !matches!(config.codex.active, Some(Selection::Api { .. }))
                && matches!(selection, Selection::Api { .. })
                && old
                    .get("model_provider")
                    .and_then(Item::as_str)
                    .is_none_or(|p| p == "openai")
                && old.get("openai_base_url").is_none()
            {
                config.codex.subscription_model =
                    old.get("model").and_then(Item::as_str).map(str::to_owned);
                config.codex.subscription_reasoning = old
                    .get("model_reasoning_effort")
                    .and_then(Item::as_str)
                    .map(str::to_owned);
            }
            config.codex.active = Some(selection);
            Ok(())
        })?;
        Ok::<_, anyhow::Error>(())
    })();
    if let Err(error) = result {
        rollback(paths, &transaction)
            .context("Could not roll back; run ccsw codex recover before further changes")?;
        return Err(error);
    }
    fs::remove_file(journal_path(paths))?;
    Ok(())
}
pub fn apply(
    paths: &AppPaths,
    profile_id: &str,
    model: Option<&str>,
    reasoning: Option<&str>,
) -> Result<()> {
    let _guard = lock(paths)?;
    let config = config::load_client(&paths.config, config::Client::Codex)?;
    let profile = config
        .profiles
        .get(profile_id)
        .context("Provider does not exist")?;
    if !profile.enabled {
        bail!("Enable this provider before applying it to Codex");
    }
    let wanted = config::canonical_model_id(model.unwrap_or(&profile.default_model));
    let models = crate::discovery::active_models(profile, &[]);
    let entry = models
        .iter()
        .find(|m| config::canonical_model_id(&m.id) == wanted)
        .context("Model is disabled or not configured")?;
    if let Some(reasoning) = reasoning {
        validate_reasoning(reasoning)?;
    }
    let home = home()?;
    let old = document(&home)?;
    if let Some(binding) = binding(paths)? {
        check_managed(&binding, &home, &old)?;
    }
    let (url, token) = proxy::codex_route(paths, profile_id)?;
    let mut new = old.clone();
    new["model"] = value(wanted);
    new["model_provider"] = value("ccsw");
    let catalog = write_model_catalog(paths, &models)?;
    new["model_catalog_json"] = value(catalog.to_string_lossy().as_ref());
    ensure_providers(&mut new)?;
    new["model_providers"]["ccsw"] = Item::Table(toml_edit::Table::new());
    for (key, val) in [
        ("name", "CCSW"),
        ("base_url", url.as_str()),
        ("wire_api", "responses"),
        ("experimental_bearer_token", token.as_str()),
    ] {
        new["model_providers"]["ccsw"][key] = value(val);
    }
    new["model_providers"]["ccsw"]["requires_openai_auth"] = value(false);
    new["model_providers"]["ccsw"]["supports_websockets"] = value(false);
    if let Some(effort) = reasoning.or(config.codex.reasoning_effort.as_deref()) {
        new["model_reasoning_effort"] = value(effort);
    }
    if profile.api_format != config::ApiFormat::OpenaiResponses {
        new["web_search"] = value("disabled");
    }
    if let Some(context) = entry.context_window {
        new["model_context_window"] = value(i64::from(context));
        new["model_auto_compact_token_limit"] = value(i64::from(context) * 9 / 10);
    } else {
        new.remove("model_context_window");
        new.remove("model_auto_compact_token_limit");
    }
    commit(
        paths,
        &home,
        &old,
        &new,
        None,
        Selection::Api {
            profile: profile_id.into(),
            model: wanted.into(),
        },
    )
}
pub fn validate_reasoning(value: &str) -> Result<()> {
    if !["none", "minimal", "low", "medium", "high", "xhigh"].contains(&value) {
        bail!("Reasoning must be none, minimal, low, medium, high or xhigh");
    }
    Ok(())
}
pub fn status(paths: &AppPaths) -> Result<String> {
    let home = home()?;
    let config = config::load(&paths.config)?;
    let doc = document(&home)?;
    let mut message = format!(
        "Codex home: {}\nDesired: {:?}\nProvider: {}\nModel: {}",
        home.display(),
        config.codex.active,
        doc.get("model_provider")
            .and_then(Item::as_str)
            .unwrap_or("openai"),
        doc.get("model")
            .and_then(Item::as_str)
            .unwrap_or("Codex default")
    );
    message.push_str(&format!("\n{}", accounts::live_login()?.1));
    for key in ["OPENAI_API_KEY", "CODEX_ACCESS_TOKEN", "CODEX_AUTH"] {
        if std::env::var_os(key).is_some_and(|v| !v.is_empty()) {
            message.push_str(&format!(
                "\nEnvironment override present: {key}; verify the client authentication method."
            ));
        }
    }
    if paths.state_dir.join("codex-transaction.json").exists() {
        message.push_str("\nInterrupted transaction: run ccsw codex recover.");
    }
    if let Some(binding) = binding(paths)? {
        if let Err(error) = check_managed(&binding, &home, &doc) {
            message.push_str(&format!("\nConflict: {error}"));
        }
        if let Some(id) = binding.expected_account {
            let actual = accounts::read_live_auth(&home, &doc)?
                .and_then(|auth| accounts::identity(&auth).ok().map(|a| a.id));
            if actual.as_deref() != Some(&id) {
                message.push_str("\nLogin conflict: an old client may have restored another account. Restart clients and apply again.");
            }
        }
        message.push_str("\nOn-disk configuration only; restart CLI / ChatGPT App and verify a new chat. Project/launch overrides may take precedence.");
    }
    Ok(message)
}
pub(crate) struct DetachPlan {
    pub home: PathBuf,
    before: String,
    after: String,
    live_auth: Option<Value>,
    restore_auth: Option<Option<Value>>,
}
pub(crate) fn prepare_detach(paths: &AppPaths) -> Result<Option<DetachPlan>> {
    let Some(state) = binding(paths)? else {
        return Ok(None);
    };
    if journal_path(paths).exists() {
        bail!("Run ccsw codex recover before disconnecting or uninstalling");
    }
    let mut current = document(&state.home)?;
    let before = current.to_string();
    let original: DocumentMut = state.before.parse()?;
    let live = accounts::read_live_auth(&state.home, &current)?;
    for key in KEYS {
        if get(&current, key) == state.managed.get(*key).cloned().flatten() {
            restore_key(&mut current, &original, key);
        }
    }
    let live_id = live
        .as_ref()
        .and_then(|auth| accounts::identity(auth).ok().map(|i| i.id));
    let original_id = state
        .original_auth
        .as_ref()
        .and_then(|auth| accounts::identity(auth).ok().map(|i| i.id));
    let restore_auth = if state.expected_account.is_some() && live_id == state.expected_account {
        Some(if original_id.is_some() && original_id == live_id {
            live.clone()
        } else {
            accounts::latest_snapshot(paths, state.original_auth.as_ref())?
        })
    } else {
        None
    };
    Ok(Some(DetachPlan {
        home: state.home,
        before,
        after: current.to_string(),
        live_auth: live,
        restore_auth,
    }))
}
pub(crate) fn execute_detach(plan: &DetachPlan) -> Result<()> {
    let current = document(&plan.home)?;
    if current.to_string() != plan.before
        || accounts::read_live_auth(&plan.home, &current)? != plan.live_auth
    {
        bail!("Codex changed during detach; no files changed, retry");
    }
    let after: DocumentMut = plan.after.parse()?;
    if let Some(auth) = &plan.restore_auth {
        restore_auth(&plan.home, &after, auth.as_ref())?;
    }
    if let Err(error) = atomic_write(&plan.home.join("config.toml"), plan.after.as_bytes()) {
        if plan.restore_auth.is_some() {
            restore_auth(&plan.home, &current, plan.live_auth.as_ref())?;
        }
        return Err(error);
    }
    Ok(())
}
pub fn disconnect(paths: &AppPaths) -> Result<()> {
    let _guard = lock(paths)?;
    let plan = prepare_detach(paths)?.context("Codex is not managed by CCSW")?;
    if plan.home != home()? {
        bail!("Use the original CODEX_HOME to disconnect");
    }
    accounts::capture_current(paths, plan.live_auth.as_ref())?;
    execute_detach(&plan)?;
    config::update(&paths.config, |config| {
        config.codex.active = None;
        Ok(())
    })?;
    fs::remove_file(binding_path(paths))?;
    Ok(())
}

fn write_model_catalog(paths: &AppPaths, models: &[config::ModelEntry]) -> Result<PathBuf> {
    use sha2::{Digest, Sha256};
    let entries: Vec<Value> = models.iter().map(|entry| {
        let context = entry.context_window.unwrap_or(if entry.id.ends_with("[1m]") { 1_000_000 } else { 128_000 });
        json!({
            "slug":config::canonical_model_id(&entry.id), "display_name":entry.label(),
            "description":"User-configured model managed by CCSW",
            "default_reasoning_level":"none", "supported_reasoning_levels":[],
            "shell_type":"unified_exec", "visibility":"list", "supported_in_api":true,
            "priority":0, "base_instructions":"You are a coding assistant. Inspect the workspace, use tools to perform the requested work, and verify your changes.",
            "supports_reasoning_summaries":false,"support_verbosity":false,
            "truncation_policy":{"mode":"tokens","limit":10000},
            "context_window":context,"effective_context_window_percent":90,
            "input_modalities":["text"],"experimental_supported_tools":[],
            "supports_search_tool":false,"use_responses_lite":false
        })
    }).collect();
    let bytes = serde_json::to_vec_pretty(&json!({"models":entries}))?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let dir = paths.state_dir.join("codex-model-catalogs");
    fs::create_dir_all(&dir)?;
    let path = std::path::absolute(dir.join(format!("{digest}.json")))?;
    atomic_write(&path, &bytes)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, AppPaths, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let paths = AppPaths {
            config: root.path().join("ccsw.toml"),
            state_dir: root.path().join("state"),
            cache: root.path().join("cache"),
        };
        config::update(&paths.config, |c| {
            *c = Default::default();
            Ok(())
        })
        .unwrap();
        private_dir(&paths.state_dir).unwrap();
        let home = root.path().join("codex");
        private_dir(&home).unwrap();
        atomic_write(
            &home.join("config.toml"),
            b"# Comment\nmodel = 'old'\n[features]\nkeep = true\n",
        )
        .unwrap();
        (root, paths, home)
    }
    #[test]
    fn managed_comparison_ignores_toml_formatting_and_table_order() {
        let a: DocumentMut =
            "model='m'\n[model_providers.ccsw]\nname='CCSW'\nbase_url='http://localhost'\n"
                .parse()
                .unwrap();
        let b:DocumentMut="model = \"m\" # comment\n[model_providers.ccsw]\nbase_url = \"http://localhost\"\nname = \"CCSW\"\n".parse().unwrap();
        assert_eq!(get(&a, "model"), get(&b, "model"));
        assert_eq!(
            get(&a, "model_providers.ccsw"),
            get(&b, "model_providers.ccsw")
        );
    }
    #[test]
    fn rollback_restores_absent_auth_and_preserves_unrelated_changes() {
        let (_root, paths, home) = fixture();
        let old = document(&home).unwrap();
        let mut new = old.clone();
        new["model"] = value("new");
        let auth = json!({"OPENAI_API_KEY":"fake-local-only"});
        let transaction = Transaction {
            home: home.clone(),
            old_config: old.to_string(),
            new_config: new.to_string(),
            old_auth: None,
            new_auth: Some(auth.clone()),
            old_binding: None,
            old_selection: None,
            old_settings: Default::default(),
            new_selection: Selection::Api {
                profile: "test".into(),
                model: "new".into(),
            },
        };
        atomic_write(
            &journal_path(&paths),
            &serde_json::to_vec(&transaction).unwrap(),
        )
        .unwrap();
        new["features"]["other"] = value(true);
        atomic_write(&home.join("config.toml"), new.to_string().as_bytes()).unwrap();
        accounts::write_live_auth(&home, &new, &auth).unwrap();
        rollback(&paths, &transaction).unwrap();
        let restored = document(&home).unwrap();
        assert_eq!(restored["model"].as_str(), Some("old"));
        assert_eq!(restored["features"]["other"].as_bool(), Some(true));
        assert!(!home.join("auth.json").exists());
        assert!(!journal_path(&paths).exists());
    }
    #[test]
    fn failed_commit_rolls_back_binding_and_external_files() {
        let (_root, paths, home) = fixture();
        let old = document(&home).unwrap();
        let mut new = old.clone();
        new["model"] = value("new");
        // Make CCSW config locking fail after external settings and the binding were written.
        fs::remove_file(paths.config.with_extension("toml.lock")).unwrap();
        fs::create_dir(paths.config.with_extension("toml.lock")).unwrap();
        let result = commit(
            &paths,
            &home,
            &old,
            &new,
            None,
            Selection::Api {
                profile: "test".into(),
                model: "new".into(),
            },
        );
        assert!(result.is_err());
        assert_eq!(document(&home).unwrap().to_string(), old.to_string());
    }
    #[test]
    fn quota_cache_round_trips_nullable_fields_without_leaking_credentials() {
        let mut config = config::Config::default();
        config.codex.accounts.insert("account".into(),accounts::Account{ name:"Test".into(),limits:json!({"rateLimits":{"primary":null,"secondary":{"usedPercent":100,"resetsAt":null}}}),..Default::default() });
        let text = toml::to_string(&config).unwrap();
        let decoded: config::Config = toml::from_str(&text).unwrap();
        assert_eq!(decoded, config);
    }
}
