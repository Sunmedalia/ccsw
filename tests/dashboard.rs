//! Dashboard reads and actions against isolated files, never real accounts.
mod support;
use serde_json::Value;
use std::fs;

fn snapshot(root: &std::path::Path) -> (Value, String) {
    let out = support::command(root)
        .args(["dashboard", "snapshot", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).unwrap();
    (serde_json::from_str(&text).unwrap(), text)
}
fn action(root: &std::path::Path, args: &[&str]) -> Value {
    let out = support::command(root)
        .args(["dashboard", "action", "--json"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success());
    serde_json::from_slice(&out.stdout).unwrap()
}
#[test]
fn empty_snapshot_does_not_initialize_files() {
    let root = tempfile::tempdir().unwrap();
    let (v, _) = snapshot(root.path());
    assert_eq!(v["schema_version"], 1);
    assert_eq!(v["accounts"].as_array().unwrap().len(), 0);
    assert_eq!(v["clients"].as_array().unwrap().len(), 3);
    assert_eq!(v["usage"]["available"], false);
    assert!(!root.path().join("state").exists());
    assert!(!root.path().join("config.toml").exists());
    assert!(!root.path().join("pi").exists());
}
#[test]
fn snapshot_allowlists_fields_and_preserves_partial_results() {
    let root = tempfile::tempdir().unwrap();
    let config = "version=5\n[profiles.demo]\nname='Demo'\nbase_url='https://secret-user:secret-pass@example.invalid?token=url-secret'\ndefault_model='visible-model'\n[profiles.demo.credential]\nkind='bearer'\nvalue='super-private-key'\n[codex.accounts.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa]\nname='Work'\nemail='work@example.test'\nworkspace='workspace'\nsubject='private-subject'\nplan='plus'\nlimits='{}'\n";
    fs::write(root.path().join("config.toml"), config).unwrap();
    fs::create_dir_all(root.path().join("pi")).unwrap();
    fs::write(root.path().join("pi/models.json"), "INVALID private-json").unwrap();
    let (v, text) = snapshot(root.path());
    assert_eq!(v["accounts"][0]["plan"], "plus");
    assert_eq!(
        v["clients"][0]["providers"][0]["models"][0]["id"],
        "visible-model"
    );
    for secret in [
        "secret-user",
        "secret-pass",
        "url-secret",
        "super-private-key",
        "private-subject",
        "private-json",
    ] {
        assert!(!text.contains(secret));
    }
    assert!(
        v["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["section"] == "pi")
    );
    assert_eq!(
        fs::read_to_string(root.path().join("config.toml")).unwrap(),
        config
    );
}
#[test]
fn pi_queries_do_not_recover_and_actions_switch_only_the_default() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("pi");
    fs::create_dir(&home).unwrap();
    let models = r#"{"providers":{"demo":{"baseUrl":"https://example.invalid/v1","api":"openai-completions","apiKey":"secret-pi-key","models":[{"id":"one","name":"One"},{"id":"two","name":"Two"}]}}}"#;
    fs::write(home.join("models.json"), models).unwrap();
    fs::write(
        home.join("settings.json"),
        r#"{"defaultProvider":"demo","defaultModel":"one","theme":"dark"}"#,
    )
    .unwrap();
    fs::write(home.join(".ccsw-native-transaction.json"), "sentinel").unwrap();
    let (v, text) = snapshot(root.path());
    assert_eq!(v["clients"][2]["status"], "unavailable");
    assert!(!text.contains("secret-pi-key"));
    assert_eq!(
        fs::read_to_string(home.join(".ccsw-native-transaction.json")).unwrap(),
        "sentinel"
    );
    fs::remove_file(home.join(".ccsw-native-transaction.json")).unwrap();
    let v = action(
        root.path(),
        &[
            "apply",
            "--client",
            "pi",
            "--profile",
            "demo",
            "--model",
            "two",
        ],
    );
    assert_eq!(v["ok"], true);
    assert_eq!(v["restart_required"], false);
    let saved: Value =
        serde_json::from_slice(&fs::read(home.join("settings.json")).unwrap()).unwrap();
    assert_eq!(saved["defaultModel"], "two");
    assert_eq!(saved["theme"], "dark");
    assert_eq!(
        fs::read_to_string(home.join("models.json")).unwrap(),
        models
    );
    let failed = action(
        root.path(),
        &[
            "apply",
            "--client",
            "pi",
            "--profile",
            "demo",
            "--model",
            "missing",
        ],
    );
    assert_eq!(failed["ok"], false);
    assert_eq!(failed["error"], "action_failed");
    assert_eq!(snapshot(root.path()).0["clients"][2]["model"], "two");
}
#[test]
fn usage_respects_ledger_timezone_scope_and_unknown_tokens() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state/ccsw");
    fs::create_dir_all(&state).unwrap();
    let db = rusqlite::Connection::open(state.join("usage.sqlite3")).unwrap();
    db.execute_batch("CREATE TABLE settings(key TEXT PRIMARY KEY,value INTEGER); INSERT INTO settings VALUES('offset',28800);
        CREATE TABLE requests(id TEXT,config TEXT,client TEXT,provider TEXT,name TEXT,model TEXT,kind TEXT,started INTEGER,day TEXT,outcome TEXT,input INTEGER,output INTEGER,cache_read INTEGER,cache_write INTEGER);").unwrap();
    let now = chrono::Utc::now();
    let today = now
        .with_timezone(&chrono::FixedOffset::east_opt(28800).unwrap())
        .format("%Y-%m-%d")
        .to_string();
    for (id, kind, path, input) in [
        (
            "one",
            "generation",
            root.path().join("config.toml"),
            Some(100),
        ),
        ("two", "generation", root.path().join("config.toml"), None),
        (
            "compact",
            "compaction",
            root.path().join("config.toml"),
            Some(500),
        ),
        (
            "other",
            "generation",
            root.path().join("other.toml"),
            Some(900),
        ),
    ] {
        db.execute("INSERT INTO requests VALUES(?1,?2,'Claude','demo','Demo','model',?3,?4,?5,'success',?6,20,10,0)",rusqlite::params![id,path.to_string_lossy(),kind,now.timestamp(),today,input]).unwrap();
    }
    let v = snapshot(root.path()).0;
    assert_eq!(v["usage"]["offset_seconds"], 28800);
    let today = &v["usage"]["ranges"][0];
    assert_eq!(today["totals"]["calls"], 2);
    assert_eq!(today["totals"]["input"], 100);
    assert_eq!(today["totals"]["unknown"], 1);
    assert_eq!(today["points"].as_array().unwrap().len(), 24);
}
#[test]
fn concurrent_menu_action_is_rejected_without_mutation() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state/ccsw");
    fs::create_dir_all(&state).unwrap();
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(state.join("dashboard.lock"))
        .unwrap();
    fs2::FileExt::lock_exclusive(&lock).unwrap();
    let v = action(root.path(), &["use-account", "missing"]);
    assert_eq!(v["error"], "busy");
    assert!(!root.path().join("codex/auth.json").exists());
}

#[test]
fn claude_codex_and_proxy_actions_round_trip() {
    let root = tempfile::tempdir().unwrap();
    struct Stop<'a>(&'a std::path::Path);
    impl Drop for Stop<'_> {
        fn drop(&mut self) {
            let _ = support::command(self.0).args(["proxy", "stop"]).output();
        }
    }
    let _stop = Stop(root.path());
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let set_port = support::command(root.path())
        .args(["proxy", "port", &port.to_string()])
        .output()
        .unwrap();
    assert!(set_port.status.success());
    fs::write(root.path().join("config.toml"), "version=5\n[profiles.demo]\nname='Demo'\nbase_url='https://example.invalid'\ndefault_model='one'\nenabled_models=['two']\n[codex.profiles.demo]\nname='Codex demo'\nbase_url='https://example.invalid'\napi_format='openai-responses'\ndefault_model='one'\nenabled_models=['two']\n").unwrap();
    let result = action(
        root.path(),
        &[
            "apply",
            "--client",
            "claude",
            "--profile",
            "demo",
            "--model",
            "two",
        ],
    );
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["restart_required"], false);
    let v = snapshot(root.path()).0;
    assert_eq!(v["clients"][0]["model"], "demo::two");
    assert_eq!(v["proxy"]["running"], true);
    let result = action(
        root.path(),
        &[
            "apply",
            "--client",
            "codex",
            "--profile",
            "demo",
            "--model",
            "two",
        ],
    );
    assert_eq!(result["ok"], true, "{result}");
    let v = snapshot(root.path()).0;
    assert_eq!(v["clients"][1]["model"], "two");
    assert_eq!(v["clients"][1]["status"], "synced");
    // External edits must not be described as synchronized.
    let codex_path = root.path().join("codex/config.toml");
    let text = fs::read_to_string(&codex_path)
        .unwrap()
        .replace("model = \"two\"", "model = \"external\"");
    fs::write(codex_path, text).unwrap();
    assert_eq!(snapshot(root.path()).0["clients"][1]["status"], "conflict");
    assert_eq!(action(root.path(), &["proxy-stop"])["ok"], true);
    assert_eq!(snapshot(root.path()).0["proxy"]["running"], false);
    assert_eq!(action(root.path(), &["proxy-start"])["ok"], true);
    assert_eq!(snapshot(root.path()).0["proxy"]["running"], true);
}

fn management(root: &std::path::Path) -> Value {
    let out = support::command(root)
        .args(["dashboard", "management", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    serde_json::from_slice(&out.stdout).unwrap()
}
fn edit(root: &std::path::Path, operation: &str, payload: &Value) -> Value {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = support::command(root)
        .args(["dashboard", "action", operation, "--json"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    serde_json::from_slice(&out.stdout).unwrap()
}
fn draft(client: &str) -> Value {
    serde_json::json!({"client":client,"id":"demo","revision":null,"name":"GUI provider","enabled":true,"base_url":"https://example.invalid/v1","api_format":"openai-chat","credential_kind":"bearer","credential_change":"replace","secret":"new-secret-not-in-argv","default_model":"one","models":[{"id":"one","enabled":true,"context_window":128000,"max_output_tokens":4096},{"id":"two","enabled":true}],"aliases":{},"fallback_models":[]})
}
#[test]
fn gui_crud_is_shared_with_config_and_never_returns_credentials() {
    let root = tempfile::tempdir().unwrap();
    let result = edit(root.path(), "save-profile", &draft("claude"));
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["restart_required"], false);
    let config: toml::Value =
        toml::from_str(&fs::read_to_string(root.path().join("config.toml")).unwrap()).unwrap();
    assert_eq!(
        config["profiles"]["demo"]["models"][0]["max_output_tokens"].as_integer(),
        Some(4096)
    );
    let view = management(root.path());
    assert!(!view.to_string().contains("new-secret-not-in-argv"));
    let mut p = view["providers"][0].clone();
    assert_eq!(p["credential_present"], true);
    p["name"] = "Edited in GUI".into();
    p["models"][1]["enabled"] = false.into();
    assert_eq!(edit(root.path(), "save-profile", &p)["ok"], true);
    let raw = fs::read_to_string(root.path().join("config.toml")).unwrap();
    assert!(raw.contains("new-secret-not-in-argv"));
    assert!(raw.contains("Edited in GUI"));
    assert_eq!(
        snapshot(root.path()).0["clients"][0]["providers"][0]["models"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let latest = &management(root.path())["providers"][0];
    assert_eq!(
        edit(
            root.path(),
            "delete-profile",
            &serde_json::json!({"client":"claude","id":"demo","revision":latest["revision"]})
        )["ok"],
        true
    );
    assert!(
        management(root.path())["providers"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn gui_rejects_stale_tui_edits_and_retains_unrelated_changes() {
    let root = tempfile::tempdir().unwrap();
    assert_eq!(
        edit(root.path(), "save-profile", &draft("codex"))["ok"],
        true
    );
    let mut p = management(root.path())["providers"][0].clone();
    let config = root.path().join("config.toml");
    let raw = fs::read_to_string(&config)
        .unwrap()
        .replace("GUI provider", "Changed in TUI");
    fs::write(&config, &raw).unwrap();
    p["name"] = "Stale GUI".into();
    assert_eq!(
        edit(root.path(), "save-profile", &p)["error"],
        "edit_conflict"
    );
    assert_eq!(fs::read_to_string(&config).unwrap(), raw);
    assert_eq!(
        edit(
            root.path(),
            "delete-profile",
            &serde_json::json!({"client":"codex","id":"demo","revision":p["revision"]})
        )["error"],
        "edit_conflict"
    );
    // Changes to another client's providers do not block or disappear during save.
    let mut fresh = management(root.path())["providers"][0].clone();
    assert_eq!(
        edit(root.path(), "save-profile", &draft("claude"))["ok"],
        true
    );
    fresh["name"] = "Fresh GUI".into();
    assert_eq!(edit(root.path(), "save-profile", &fresh)["ok"], true);
    assert_eq!(
        management(root.path())["providers"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}
#[test]
fn gui_redacts_endpoint_secrets_and_preserves_them_on_unrelated_edits() {
    let root = tempfile::tempdir().unwrap();
    let mut p = draft("claude");
    p["base_url"] = "https://user:password@example.invalid/v1?key=hidden".into();
    assert_eq!(edit(root.path(), "save-profile", &p)["ok"], true);
    let view = management(root.path());
    for secret in ["password", "hidden", "new-secret-not-in-argv"] {
        assert!(!view.to_string().contains(secret));
    }
    let mut p = view["providers"][0].clone();
    assert_eq!(p["endpoint_redacted"], true);
    p["name"] = "Keep endpoint".into();
    assert_eq!(edit(root.path(), "save-profile", &p)["ok"], true);
    assert!(
        fs::read_to_string(root.path().join("config.toml"))
            .unwrap()
            .contains("?key=hidden")
    );
}
#[test]
fn gui_pi_edits_preserve_native_extras_and_default_references() {
    let root = tempfile::tempdir().unwrap();
    assert_eq!(edit(root.path(), "save-profile", &draft("pi"))["ok"], true);
    let path = root.path().join("pi/models.json");
    let mut raw: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    raw["providers"]["demo"]["customSetting"] = "preserve".into();
    raw["providers"]["demo"]["models"][0]["cost"] = serde_json::json!({"input":1});
    fs::write(&path, raw.to_string()).unwrap();
    assert_eq!(
        action(
            root.path(),
            &[
                "apply",
                "--client",
                "pi",
                "--profile",
                "demo",
                "--model",
                "one"
            ]
        )["ok"],
        true
    );
    let mut p = management(root.path())["providers"][0].clone();
    p["models"][0]["label"] = "GUI label".into();
    assert_eq!(edit(root.path(), "save-profile", &p)["ok"], true);
    let raw: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(raw["providers"]["demo"]["customSetting"], "preserve");
    assert_eq!(raw["providers"]["demo"]["models"][0]["cost"]["input"], 1);
    assert_eq!(raw["providers"]["demo"]["models"][0]["name"], "GUI label");
    assert!(!root.path().join("config.toml").exists());
}
#[test]
fn invalid_gui_model_or_credentials_do_not_write_configuration() {
    let root = tempfile::tempdir().unwrap();
    let mut p = draft("claude");
    p["models"][0]["max_output_tokens"] = 0.into();
    assert_eq!(
        edit(root.path(), "save-profile", &p)["error"],
        "invalid_input"
    );
    assert!(!root.path().join("config.toml").exists());
    p["models"][0]["max_output_tokens"] = 100.into();
    p["secret"] = "".into();
    assert_eq!(
        edit(root.path(), "save-profile", &p)["error"],
        "invalid_input"
    );
    assert!(!root.path().join("config.toml").exists());
}

#[test]
fn gui_preserves_context_variants_in_default_and_role_models() {
    let root = tempfile::tempdir().unwrap();
    let mut p = draft("claude");
    p["default_model"] = "one[1m]".into();
    p["aliases"]["sonnet"] = "one[1m]".into();
    let saved = edit(root.path(), "save-profile", &p);
    assert_eq!(saved["ok"], true, "{saved}");
    let mut p = management(root.path())["providers"][0].clone();
    p["name"] = "Context retained".into();
    assert_eq!(edit(root.path(), "save-profile", &p)["ok"], true);
    assert_eq!(
        management(root.path())["providers"][0]["default_model"],
        "one[1m]"
    );
    assert_eq!(
        management(root.path())["providers"][0]["aliases"]["sonnet"],
        "one[1m]"
    );
}

fn query(root: &std::path::Path, command: &str) -> Value {
    let out = support::command(root)
        .args(["dashboard", command, "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
fn manage(root: &std::path::Path, operation: &str, payload: &Value) -> Value {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = support::command(root)
        .args(["dashboard", "action", "manage", operation, "--json"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    serde_json::from_slice(&child.wait_with_output().unwrap().stdout).unwrap()
}
#[test]
fn gui_distinguishes_saved_paused_from_failed_and_explicitly_reconnects() {
    let root = tempfile::tempdir().unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    support::command(root.path())
        .args(["proxy", "port", &port.to_string()])
        .output()
        .unwrap();
    struct Stop<'a>(&'a std::path::Path);
    impl Drop for Stop<'_> {
        fn drop(&mut self) {
            let _ = support::command(self.0).args(["proxy", "stop"]).output();
        }
    }
    let _stop = Stop(root.path());
    assert_eq!(
        edit(root.path(), "save-profile", &draft("claude"))["ok"],
        true
    );
    assert_eq!(
        manage(root.path(), "sync", &serde_json::json!({}))["ok"],
        true
    );
    let path = root.path().join("claude/settings.json");
    let mut settings: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    settings["env"]["ANTHROPIC_BASE_URL"] = "https://external.invalid/private".into();
    settings["env"]["ANTHROPIC_AUTH_TOKEN"] = "external-secret-do-not-log".into();
    fs::write(&path, settings.to_string()).unwrap();
    let mut profile = management(root.path())["providers"][0].clone();
    profile["name"] = "Edited while paused".into();
    let saved = edit(root.path(), "save-profile", &profile);
    assert_eq!(saved["saved"], true);
    assert_eq!(saved["error"], "configuration_saved_sync_paused");
    assert!(
        saved["sync_conflicts"]
            .as_array()
            .unwrap()
            .contains(&"env.ANTHROPIC_BASE_URL".into())
    );
    assert!(!saved.to_string().contains("external-secret"));
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(&path).unwrap()).unwrap(),
        settings
    );
    let state = query(root.path(), "workspace");
    assert!(state["sync_conflicts"].as_array().unwrap().len() >= 2);
    assert!(!state.to_string().contains("external-secret"));
    assert_eq!(
        management(root.path())["providers"][0]["name"],
        "Edited while paused"
    );
    assert_eq!(
        manage(root.path(), "sync", &serde_json::json!({}))["ok"],
        true
    );
    assert!(
        query(root.path(), "workspace")["sync_conflicts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn gui_preferences_validate_conflicts_and_share_with_tui() {
    let root = tempfile::tempdir().unwrap();
    let state = query(root.path(), "workspace");
    let payload = serde_json::json!({"revision":state["preferences_revision"],"claude":{"hide_attribution":true,"env":{"ENABLE_TOOL_SEARCH":"auto:50","MY_LITERAL":"$(not-a-command)"}},"reasoning":"high"});
    assert_eq!(manage(root.path(), "preferences", &payload)["ok"], true);
    let fresh = query(root.path(), "workspace");
    assert_eq!(
        fresh["preferences"]["claude"]["env"]["MY_LITERAL"],
        "$(not-a-command)"
    );
    assert_eq!(
        manage(root.path(), "preferences", &payload)["error"],
        "edit_conflict"
    );
    let invalid = serde_json::json!({"revision":fresh["preferences_revision"],"claude":{"env":{"ANTHROPIC_AUTH_TOKEN":"invalid"}}});
    assert_eq!(
        manage(root.path(), "preferences", &invalid)["error"],
        "invalid_input"
    );
    assert_eq!(
        query(root.path(), "workspace")["preferences_revision"],
        fresh["preferences_revision"]
    );
}
#[test]
fn unsaved_gui_draft_can_fetch_models_test_connection_and_inference() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        process::Stdio,
    };
    let root = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        for (status, body) in [
            ("200 OK", r#"{"data":[{"id":"discovered-model"}]}"#),
            ("401 Unauthorized", "{}"),
            ("200 OK", r#"{"choices":[{"message":{"content":"OK"}}]}"#),
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            let mut raw = vec![];
            let mut buf = [0; 4096];
            loop {
                let n = stream.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                raw.extend_from_slice(&buf[..n]);
                if let Some(i) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&raw[..i]).to_lowercase();
                    let count = header
                        .lines()
                        .find_map(|l| {
                            l.strip_prefix("content-length:")
                                .and_then(|v| v.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if raw.len() >= i + 4 + count {
                        break;
                    }
                }
            }
            let request = String::from_utf8_lossy(&raw);
            assert!(request.contains("draft-secret"));
            if request.starts_with("POST") {
                assert!(request.contains("discovered-model"));
                assert!(request.contains("Reply OK."));
            }
            write!(stream,"HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        }
    });
    let mut payload = draft("claude");
    payload["id"] = "".into();
    payload["name"] = "".into();
    payload["models"] = serde_json::json!([]);
    payload["default_model"] = "".into();
    payload["base_url"] = format!("http://{address}/v1").into();
    payload["secret"] = "draft-secret".into();
    let probe = |kind: &str| {
        let mut c = support::command(root.path());
        c.args(["dashboard", "probe", kind, "--json"]);
        if kind == "model" {
            c.args(["--model", "discovered-model"]);
        }
        let mut child = c
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.to_string().as_bytes())
            .unwrap();
        serde_json::from_slice::<Value>(&child.wait_with_output().unwrap().stdout).unwrap()
    };
    assert_eq!(
        probe("models")["data"]["models"][0]["id"],
        "discovered-model"
    );
    assert_eq!(probe("connection")["data"]["http_status"], 401);
    assert_eq!(probe("model")["ok"], true);
    server.join().unwrap();
    assert!(!root.path().join("config.toml").exists());
    assert!(!root.path().join("state").exists());
}
#[cfg(unix)]
#[test]
fn dashboard_login_streams_device_progress_and_saves_without_switching() {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use std::{
        io::{BufRead, BufReader},
        os::unix::fs::PermissionsExt,
        process::Stdio,
    };
    let root = tempfile::tempdir().unwrap();
    let claims = serde_json::json!({"sub":"gui-test","email":"test@example.invalid","https://api.openai.com/auth":{"chatgpt_account_id":"workspace","chatgpt_plan_type":"plus"}});
    let auth = serde_json::json!({"tokens":{"id_token":format!("e30.{}.sig",URL_SAFE_NO_PAD.encode(claims.to_string())),"access_token":"access-secret","refresh_token":"refresh-secret","account_id":"workspace"}});
    let fixture = root.path().join("auth-fixture.json");
    fs::write(&fixture, auth.to_string()).unwrap();
    let script = root.path().join("codex-mock");
    fs::write(&script, include_str!("fixtures/codex_rpc.py")).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    let mut child = support::command(root.path())
        .args([
            "dashboard",
            "login",
            "--name",
            "GUI account",
            "--device",
            "--json",
        ])
        .env("CCSW_CODEX_BIN", script)
        .env("CCSW_MOCK_AUTH", fixture)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let input = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let progress: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(progress["event"], "progress");
    assert!(progress["message"].as_str().unwrap().contains("Code:"));
    line.clear();
    reader.read_line(&mut line).unwrap();
    let done: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(done["ok"], true);
    drop(input);
    assert!(child.wait().unwrap().success());
    assert!(!root.path().join("codex/auth.json").exists());
    let id = done["id"].as_str().unwrap();
    assert_eq!(
        manage(
            root.path(),
            "account-rename",
            &serde_json::json!({"id":id,"name":"Renamed"})
        )["ok"],
        true
    );
    assert_eq!(snapshot(root.path()).0["accounts"][0]["name"], "Renamed");
    assert_eq!(
        manage(root.path(), "account-remove", &serde_json::json!({"id":id}))["ok"],
        true
    );
    assert!(
        snapshot(root.path()).0["accounts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[cfg(unix)]
#[test]
fn dashboard_login_cancels_when_gui_closes_stdin() {
    use std::{
        io::{BufRead, BufReader},
        os::unix::fs::PermissionsExt,
        process::Stdio,
        time::{Duration, Instant},
    };
    let root = tempfile::tempdir().unwrap();
    let script = root.path().join("codex-waiting");
    fs::write(&script, r#"#!/usr/bin/env python3
import json,sys
for line in sys.stdin:
    r=json.loads(line)
    if 'id' not in r: continue
    result={}
    if r.get('method')=='account/login/start':
        result={'type':'chatgptDeviceCode','loginId':'pending','verificationUrl':'https://example.invalid/device','userCode':'TEST'}
    print(json.dumps({'id':r['id'],'result':result}),flush=True)
"#).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    let mut child = support::command(root.path())
        .args([
            "dashboard",
            "login",
            "--name",
            "Cancelled",
            "--device",
            "--json",
        ])
        .env("CCSW_CODEX_BIN", script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap()["event"],
        "progress"
    );
    drop(child.stdin.take());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("GUI login did not cancel promptly");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    line.clear();
    reader.read_line(&mut line).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap()["error"],
        "cancelled"
    );
    assert!(
        snapshot(root.path()).0["accounts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
