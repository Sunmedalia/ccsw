//! Real CLI processes against isolated homes; never uses a developer's Codex login.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};
struct Sandbox {
    root: tempfile::TempDir,
}
impl Sandbox {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("config.toml"), "version = 2\n[profiles.local]\nname='Local'\nbase_url='https://example.invalid/v1'\napi_format='openai-responses'\ndefault_model='test-model[1m]'\n").unwrap();
        fs::create_dir(root.path().join("codex")).unwrap();
        fs::write(root.path().join("codex/config.toml"), "# Keep my comments\nmodel = 'original-model'\nmodel_reasoning_effort = 'low'\n[features]\nkeep_me = true\n").unwrap();
        Self { root }
    }
    fn home(&self) -> PathBuf {
        self.root.path().join("codex")
    }
    fn command(&self, args: &[&str]) -> Output {
        Command::new(assert_cmd::cargo::cargo_bin("ccsw"))
            .args(args)
            .env("HOME", self.root.path())
            .env("USERPROFILE", self.root.path())
            .env("CODEX_HOME", self.home())
            .env("CCSW_CONFIG", self.root.path().join("config.toml"))
            .env("XDG_STATE_HOME", self.root.path().join("state"))
            .env("XDG_CACHE_HOME", self.root.path().join("cache"))
            .env("APPDATA", self.root.path().join("roaming"))
            .env("LOCALAPPDATA", self.root.path().join("local"))
            .env("CLAUDE_CONFIG_DIR", self.root.path().join("claude"))
            .output()
            .unwrap()
    }
    fn ok(&self, args: &[&str]) -> String {
        let result = self.command(args);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8(result.stdout).unwrap()
    }
    fn auth(&self) -> Value {
        serde_json::from_slice(&fs::read(self.home().join("auth.json")).unwrap()).unwrap()
    }
    fn save_auth(&self, auth: &Value) {
        fs::write(self.home().join("auth.json"), auth.to_string()).unwrap();
    }
    fn add(&self, name: &str, auth: &Value) -> String {
        let file = self.root.path().join(format!("{name}.json"));
        fs::write(&file, auth.to_string()).unwrap();
        let result = self.ok(&[
            "codex",
            "accounts",
            "import",
            "--name",
            name,
            "--file",
            file.to_str().unwrap(),
        ]);
        result
            .trim()
            .strip_prefix("Saved account ")
            .unwrap()
            .to_owned()
    }
}
fn auth(subject: &str, workspace: &str, refresh: &str) -> Value {
    let claims = json!({"sub":subject,"email":format!("{subject}@example.test"),"https://api.openai.com/auth":{"chatgpt_account_id":workspace,"chatgpt_plan_type":"plus"}});
    let payload = URL_SAFE_NO_PAD.encode(claims.to_string());
    json!({"auth_mode":"chatgpt","OPENAI_API_KEY":null,"tokens":{"id_token":format!("e30.{payload}.sig"),"access_token":"test-access-secret","refresh_token":refresh,"account_id":workspace},"last_refresh":"2026-01-01T00:00:00Z"})
}
#[test]
fn account_switches_preserve_latest_tokens_and_external_configuration() {
    let s = Sandbox::new();
    let a = auth("a", "workspace-a", "refresh-a");
    let b = auth("b", "workspace-b", "refresh-b");
    s.save_auth(&a);
    let aid = s.add("Account A", &a);
    let bid = s.add("Account B", &b);
    // Duplicate import updates metadata, not identity.
    assert_eq!(s.add("Renamed A", &a), aid);
    s.ok(&["codex", "accounts", "use", &aid]);
    let mut refreshed = a.clone();
    refreshed["tokens"]["refresh_token"] = json!("new-refresh-a");
    s.save_auth(&refreshed);
    s.ok(&["codex", "accounts", "use", &bid]);
    assert_eq!(s.auth(), b);
    s.ok(&["codex", "accounts", "use", &aid]);
    assert_eq!(s.auth(), refreshed);
    let config = fs::read_to_string(s.home().join("config.toml")).unwrap();
    assert!(config.contains("# Keep my comments"));
    assert!(config.contains("keep_me = true"));
    let listing = s.ok(&["codex", "accounts", "list"]);
    assert!(listing.contains("a@example.test"));
    assert!(!listing.contains("refresh-a"));
    assert!(!listing.contains("test-access-secret"));
    let rejected = s.command(&["codex", "accounts", "remove", &aid]);
    assert!(!rejected.status.success());
    s.ok(&["codex", "disconnect"]);
    assert_eq!(s.auth(), refreshed);
    let restored: toml::Value =
        toml::from_str(&fs::read_to_string(s.home().join("config.toml")).unwrap()).unwrap();
    assert_eq!(restored["model"].as_str(), Some("original-model"));
    assert_eq!(restored["model_reasoning_effort"].as_str(), Some("low"));
    s.ok(&["codex", "accounts", "remove", &bid]);
    assert!(
        !s.root
            .path()
            .join("state/ccsw/codex-accounts")
            .join(bid)
            .join("auth.json")
            .exists()
    );
}
#[test]
fn switch_conflicts_are_visible_and_disconnect_preserves_external_changes() {
    let s = Sandbox::new();
    let a = auth("a", "workspace-a", "refresh-a");
    let b = auth("b", "workspace-b", "refresh-b");
    let aid = s.add("A", &a);
    s.ok(&["codex", "accounts", "use", &aid]);
    s.save_auth(&b);
    assert!(s.ok(&["codex", "status"]).contains("Login conflict"));
    let path = s.home().join("config.toml");
    let text = fs::read_to_string(&path)
        .unwrap()
        .replace("model_provider = \"openai\"", "model_provider = \"other\"");
    fs::write(&path, text).unwrap();
    assert!(
        !s.command(&["codex", "accounts", "use", &aid])
            .status
            .success()
    );
    s.ok(&["codex", "disconnect"]);
    assert_eq!(s.auth(), b);
    assert!(
        fs::read_to_string(path)
            .unwrap()
            .contains("model_provider = \"other\"")
    );
}
#[test]
fn distinct_workspaces_and_bad_imports_do_not_overwrite_accounts() {
    let s = Sandbox::new();
    let a = s.add("Personal", &auth("same-user", "personal", "one"));
    let b = s.add("Work", &auth("same-user", "work", "two"));
    assert_ne!(a, b);
    let path = s.root.path().join("bad.json");
    fs::write(&path, r#"{"OPENAI_API_KEY":"secret"}"#).unwrap();
    let result = s.command(&[
        "codex",
        "accounts",
        "import",
        "--name",
        "Bad",
        "--file",
        path.to_str().unwrap(),
    ]);
    assert!(!result.status.success());
    assert!(!String::from_utf8_lossy(&result.stderr).contains("secret"));
    let config: toml::Value =
        toml::from_str(&fs::read_to_string(s.root.path().join("config.toml")).unwrap()).unwrap();
    assert_eq!(config["version"].as_integer(), Some(4));
    assert_eq!(config["codex"]["accounts"].as_table().unwrap().len(), 2);
}
#[test]
fn api_and_subscription_modes_restore_models_without_touching_claude() {
    let s = Sandbox::new();
    let aid = s.add("A", &auth("a", "workspace", "refresh"));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    drop(listener);
    s.ok(&["proxy", "start", "--listen", &address]);
    // Ensure the proxy is stopped even when an assertion fails.
    struct Stop<'a>(&'a Sandbox);
    impl Drop for Stop<'_> {
        fn drop(&mut self) {
            let _ = self.0.command(&["proxy", "stop"]);
        }
    }
    let _stop = Stop(&s);
    s.ok(&[
        "codex",
        "apply",
        "--profile",
        "local",
        "--reasoning",
        "high",
    ]);
    let api: toml::Value =
        toml::from_str(&fs::read_to_string(s.home().join("config.toml")).unwrap()).unwrap();
    assert_eq!(api["model"].as_str(), Some("test-model"));
    assert_eq!(api["model_provider"].as_str(), Some("ccsw"));
    assert!(
        api["model_providers"]["ccsw"]["base_url"]
            .as_str()
            .unwrap()
            .contains(&address)
    );
    assert!(!s.root.path().join("claude/settings.json").exists());
    // A model chosen directly in Codex must not block explicit API reapplication.
    let path = s.home().join("config.toml");
    let mut changed: toml_edit::DocumentMut = fs::read_to_string(&path).unwrap().parse().unwrap();
    changed["model"] = toml_edit::value("external-api-model");
    changed["model_reasoning_effort"] = toml_edit::value("low");
    fs::write(&path, changed.to_string()).unwrap();
    s.ok(&["codex", "apply", "--profile", "local"]);
    let reapplied: toml::Value = toml::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(reapplied["model"].as_str(), Some("test-model"));

    s.ok(&["codex", "accounts", "use", &aid]);
    let sub: toml::Value =
        toml::from_str(&fs::read_to_string(s.home().join("config.toml")).unwrap()).unwrap();
    assert_eq!(sub["model"].as_str(), Some("original-model"));
    s.ok(&["codex", "apply", "--profile", "local"]);
    s.ok(&["codex", "disconnect"]);
    assert!(
        !s.home().join("auth.json").exists(),
        "Disconnect restores absent credentials"
    );
}

#[test]
fn uninstall_detaches_codex_and_removes_only_registered_account_files() {
    let s = Sandbox::new();
    let aid = s.add("A", &auth("a", "workspace-a", "refresh-a"));
    s.ok(&["codex", "accounts", "use", &aid]);
    fs::write(s.home().join("history.jsonl"), "precious history").unwrap();
    let before = fs::read(s.home().join("config.toml")).unwrap();
    let preview = s.ok(&["uninstall", "--dry-run"]);
    assert!(preview.contains("Detach Codex"));
    assert_eq!(fs::read(s.home().join("config.toml")).unwrap(), before);
    s.ok(&["uninstall", "--yes"]);
    assert!(!s.home().join("auth.json").exists());
    assert_eq!(
        fs::read_to_string(s.home().join("history.jsonl")).unwrap(),
        "precious history"
    );
    assert!(!s.root.path().join("state/ccsw/codex-accounts").exists());
    let restored: toml::Value =
        toml::from_str(&fs::read_to_string(s.home().join("config.toml")).unwrap()).unwrap();
    assert_eq!(restored["model"].as_str(), Some("original-model"));
}

#[cfg(unix)]
#[test]
fn official_rpc_shape_supports_login_refresh_and_stale_quota_errors() {
    use std::os::unix::fs::PermissionsExt;
    let s = Sandbox::new();
    let fixture = s.root.path().join("mock-auth.json");
    fs::write(&fixture, auth("a", "workspace", "refresh-a").to_string()).unwrap();
    let script = s.root.path().join("codex-mock");
    fs::write(&script, include_str!("fixtures/codex_rpc.py")).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    let run = |args: &[&str]| {
        Command::new(assert_cmd::cargo::cargo_bin("ccsw"))
            .args(args)
            .env("HOME", s.root.path())
            .env("USERPROFILE", s.root.path())
            .env("CODEX_HOME", s.home())
            .env("CCSW_CONFIG", s.root.path().join("config.toml"))
            .env("XDG_STATE_HOME", s.root.path().join("state"))
            .env("XDG_CACHE_HOME", s.root.path().join("cache"))
            .env("CCSW_CODEX_BIN", &script)
            .env("CCSW_MOCK_AUTH", &fixture)
            .output()
            .unwrap()
    };
    let login = run(&["codex", "accounts", "login", "--name", "A", "--device"]);
    assert!(
        login.status.success(),
        "{}",
        String::from_utf8_lossy(&login.stderr)
    );
    assert!(
        !s.home().join("auth.json").exists(),
        "Adding an account must not switch global login"
    );
    let list = s.ok(&["codex", "accounts", "list"]);
    let id = list.split('\t').next().unwrap();
    s.ok(&["codex", "accounts", "use", id]);
    let refresh = run(&["codex", "accounts", "refresh", id]);
    assert!(
        refresh.status.success(),
        "{}",
        String::from_utf8_lossy(&refresh.stderr)
    );
    assert!(String::from_utf8_lossy(&refresh.stdout).contains("25% used"));
    assert_eq!(s.auth()["tokens"]["refresh_token"], "refreshed-in-fixture");
    fs::write(fixture.with_extension("fail"), "").unwrap();
    let failure = run(&["codex", "accounts", "refresh", id]);
    assert!(!failure.status.success());
    assert!(!String::from_utf8_lossy(&failure.stderr).contains("SECRET_SHOULD_NEVER_BE_LOGGED"));
    let config: toml::Value =
        toml::from_str(&fs::read_to_string(s.root.path().join("config.toml")).unwrap()).unwrap();
    assert!(
        config["codex"]["accounts"][id]["error"]
            .as_str()
            .unwrap()
            .contains("stale")
    );
    let limits: Value =
        serde_json::from_str(config["codex"]["accounts"][id]["limits"].as_str().unwrap()).unwrap();
    assert_eq!(limits["rateLimits"]["primary"]["usedPercent"], 25);
}

#[test]
fn codex_api_uses_its_own_catalog_and_restores_metadata_on_subscription() {
    let s = Sandbox::new();
    let path = s.root.path().join("config.toml");
    fs::write(&path, "version=4\n[profiles.local]\nname='Claude only'\nbase_url='https://claude.invalid'\ndefault_model='claude-only'\n[codex.profiles.local]\nname='Codex only'\nbase_url='https://codex.invalid'\napi_format='openai-chat'\ndefault_model='deepseek-v4-flash'\n").unwrap();
    let aid = s.add("A", &auth("a", "workspace", "refresh-a"));
    let before = fs::read(&path).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    drop(listener);
    s.ok(&["proxy", "start", "--listen", &address]);
    let result = std::panic::catch_unwind(|| {
        s.ok(&["codex", "apply", "--profile", "local"]);
        let applied: toml::Value =
            toml::from_str(&fs::read_to_string(s.home().join("config.toml")).unwrap()).unwrap();
        assert_eq!(applied["model"].as_str(), Some("deepseek-v4-flash"));
        let catalog: Value = serde_json::from_slice(
            &fs::read(applied["model_catalog_json"].as_str().unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(catalog["models"][0]["slug"], "deepseek-v4-flash");
        assert!(!catalog.to_string().contains("claude-only"));
        s.ok(&["codex", "accounts", "use", &aid]);
        let restored: toml::Value =
            toml::from_str(&fs::read_to_string(s.home().join("config.toml")).unwrap()).unwrap();
        assert!(restored.get("model_catalog_json").is_none());
        let original: toml::Value = toml::from_str(std::str::from_utf8(&before).unwrap()).unwrap();
        let after: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(after["profiles"], original["profiles"]);
    });
    s.ok(&["proxy", "stop"]);
    result.unwrap();
}

#[test]
fn external_model_change_allows_account_switch_and_is_restored_on_disconnect() {
    let s = Sandbox::new();
    let aid = s.add("A", &auth("a", "workspace-a", "refresh-a"));
    let bid = s.add("B", &auth("b", "workspace-b", "refresh-b"));
    s.ok(&["codex", "accounts", "use", &aid]);
    let path = s.home().join("config.toml");
    let mut doc: toml_edit::DocumentMut = fs::read_to_string(&path).unwrap().parse().unwrap();
    doc["model"] = toml_edit::value("chosen-in-codex");
    doc["model_reasoning_effort"] = toml_edit::value("high");
    fs::write(&path, doc.to_string()).unwrap();
    s.ok(&["codex", "accounts", "use", &bid]);
    assert!(s.ok(&["codex", "status"]).contains("Local login:"));
    s.ok(&["codex", "disconnect"]);
    let doc: toml_edit::DocumentMut = fs::read_to_string(path).unwrap().parse().unwrap();
    assert_eq!(doc["model"].as_str(), Some("chosen-in-codex"));
    assert_eq!(doc["model_reasoning_effort"].as_str(), Some("high"));
}

#[test]
fn freshly_imported_credentials_replace_revoked_live_copy_of_same_account() {
    let s = Sandbox::new();
    let old = auth("same", "workspace", "revoked-refresh");
    let fresh = auth("same", "workspace", "fresh-refresh");
    s.save_auth(&old);
    let id = s.add("Renewed", &fresh);
    s.ok(&["codex", "accounts", "use", &id]);
    assert_eq!(s.auth(), fresh);
    let saved: Value = serde_json::from_slice(
        &fs::read(
            s.root
                .path()
                .join("state/ccsw/codex-accounts")
                .join(id)
                .join("auth.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(saved, fresh);
}
#[test]
fn chatgpt_selection_does_not_inherit_unmanaged_api_model_settings() {
    let s = Sandbox::new();
    fs::write(s.home().join("config.toml"), "model_provider='custom'\nmodel='third-party'\nmodel_catalog_json='/tmp/custom.json'\nmodel_context_window=1000000\n").unwrap();
    let id = s.add("ChatGPT", &auth("user", "workspace", "fresh"));
    s.ok(&["codex", "accounts", "use", &id]);
    let doc: toml::Value =
        toml::from_str(&fs::read_to_string(s.home().join("config.toml")).unwrap()).unwrap();
    assert_eq!(doc["model_provider"].as_str(), Some("openai"));
    for key in ["model", "model_catalog_json", "model_context_window"] {
        assert!(doc.get(key).is_none());
    }
}
