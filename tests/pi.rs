use serde_json::{Value, json};
use std::{
    fs,
    process::{Command, Output},
};
struct Sandbox {
    root: tempfile::TempDir,
}
impl Sandbox {
    fn new() -> Self {
        let s = Self {
            root: tempfile::tempdir().unwrap(),
        };
        fs::create_dir(s.root.path().join("pi")).unwrap();
        fs::write(s.root.path().join("config.toml"), "version=3\n[profiles.local]\nname='Local'\nbase_url='http://localhost:12345/v1'\napi_format='openai-chat'\ndefault_model='test[1m]'\n[profiles.local.credential]\nkind='bearer'\nvalue='!secret$KEY'\n[[profiles.local.models]]\nid='test[1m]'\nmax_output_tokens=512\n").unwrap();
        s.put(
            "models.json",
            json!({"providers":{"existing":{"apiKey":"keep","models":[]}},"custom":true}),
        );
        s.put("settings.json", json!({"defaultProvider":"original","defaultModel":"before","theme":"dark","packages":["keep"]}));
        s.put(
            "auth.json",
            json!({"openai-codex":{"type":"oauth","access":"never-change"}}),
        );
        s
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(assert_cmd::cargo::cargo_bin("ccsw"))
            .args(args)
            .env("HOME", self.root.path())
            .env("PI_CODING_AGENT_DIR", self.root.path().join("pi"))
            .env("CCSW_CONFIG", self.root.path().join("config.toml"))
            .env("XDG_STATE_HOME", self.root.path().join("state"))
            .env("XDG_CACHE_HOME", self.root.path().join("cache"))
            .env("CODEX_HOME", self.root.path().join("codex"))
            .env("CLAUDE_CONFIG_DIR", self.root.path().join("claude"))
            .output()
            .unwrap()
    }
    fn ok(&self, args: &[&str]) -> String {
        let o = self.run(args);
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8(o.stdout).unwrap()
    }
    fn get(&self, name: &str) -> Value {
        serde_json::from_slice(&fs::read(self.root.path().join("pi").join(name)).unwrap()).unwrap()
    }
    fn put(&self, name: &str, v: Value) {
        fs::write(self.root.path().join("pi").join(name), v.to_string()).unwrap();
    }
}
#[test]
fn native_apply_restore_and_secret_literal() {
    let s = Sandbox::new();
    let before = [
        s.get("models.json"),
        s.get("settings.json"),
        s.get("auth.json"),
    ];
    s.ok(&["pi", "apply", "--profile", "local"]);
    let models = s.get("models.json");
    let p = &models["providers"]["ccsw-local"];
    assert_eq!(p["api"], "openai-completions");
    assert_eq!(p["apiKey"], "$!secret$$KEY");
    assert_eq!(p["models"][0]["id"], "test");
    assert_eq!(p["models"][0]["contextWindow"], 1_000_000);
    assert_eq!(p["models"][0]["maxTokens"], 512);
    assert_eq!(s.get("settings.json")["defaultProvider"], "ccsw-local");
    assert!(!s.ok(&["pi", "status"]).contains("secret"));
    s.ok(&["pi", "apply", "--profile", "local"]);
    s.ok(&["pi", "disconnect"]);
    assert_eq!(
        [
            s.get("models.json"),
            s.get("settings.json"),
            s.get("auth.json")
        ],
        before
    );
}
#[test]
fn external_edits_conflict_and_survive_disconnect() {
    let s = Sandbox::new();
    s.ok(&["pi", "apply", "--profile", "local"]);
    let mut models = s.get("models.json");
    models["providers"]["ccsw-local"]["apiKey"] = json!("external");
    s.put("models.json", models.clone());
    assert!(
        !s.run(&["pi", "apply", "--profile", "local"])
            .status
            .success()
    );
    assert!(s.ok(&["pi", "status"]).contains("conflict"));
    s.ok(&["pi", "disconnect"]);
    assert_eq!(s.get("models.json"), models);
}
#[test]
fn import_preview_deduplicates_and_preserves_compatibility() {
    let s = Sandbox::new();
    s.put("models.json",json!({"providers":{
        "custom":{"baseUrl":"https://example.invalid/v1","api":"openai-completions","apiKey":"secret","compat":{"supportsDeveloperRole":false},"models":[{"id":"a","reasoning":true,"input":["text","image"],"contextWindow":1000,"maxTokens":100}]},
        "dynamic":{"baseUrl":"https://example.invalid","api":"anthropic-messages","apiKey":"!touch /tmp/do-not-execute","models":[{"id":"a"}]},
        "builtin":{"models":[{"id":"a"}]}
    }}));
    let before = fs::read(s.root.path().join("config.toml")).unwrap();
    let preview = s.ok(&["pi", "import", "--dry-run"]);
    assert!(preview.contains("Skipped dynamic"));
    assert!(preview.contains("Skipped builtin"));
    assert!(!preview.contains("secret"));
    assert_eq!(before, fs::read(s.root.path().join("config.toml")).unwrap());
    s.ok(&["pi", "import"]);
    s.ok(&["pi", "import"]);
    let cfg: toml::Value =
        toml::from_str(&fs::read_to_string(s.root.path().join("config.toml")).unwrap()).unwrap();
    assert_eq!(cfg["profiles"].as_table().unwrap().len(), 2);
    s.ok(&["pi", "apply", "--profile", "pi-custom"]);
    let models = s.get("models.json");
    let p = &models["providers"]["ccsw-pi-custom"];
    assert_eq!(p["compat"]["supportsDeveloperRole"], false);
    assert_eq!(p["models"][0]["reasoning"], true);
}
#[test]
fn collisions_and_uninstall_are_conservative() {
    let s = Sandbox::new();
    let mut models = s.get("models.json");
    models["providers"]["ccsw-local"] = json!({"keep":true});
    s.put("models.json", models.clone());
    assert!(
        !s.run(&["pi", "apply", "--profile", "local"])
            .status
            .success()
    );
    assert_eq!(s.get("models.json"), models);
    models["providers"]
        .as_object_mut()
        .unwrap()
        .remove("ccsw-local");
    s.put("models.json", models.clone());
    let auth = s.get("auth.json");
    s.ok(&["pi", "apply", "--profile", "local"]);
    s.ok(&["uninstall", "--yes"]);
    assert_eq!(s.get("models.json"), models);
    assert_eq!(s.get("auth.json"), auth);
}

#[test]
fn import_resync_removes_disabled_models_and_reconnects_catalog() {
    let s = Sandbox::new();
    s.ok(&["pi", "apply", "--profile", "local"]);
    let path = s.root.path().join("config.toml");
    let mut cfg: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    cfg["profiles"]["local"].as_table_mut().unwrap().insert(
        "disabled_models".into(),
        toml::Value::Array(vec![toml::Value::String("test[1m]".into())]),
    );
    fs::write(&path, toml::to_string(&cfg).unwrap()).unwrap();
    assert!(s.ok(&["pi", "status"]).contains("pending"));
    s.ok(&["pi", "import"]);
    assert!(
        s.get("models.json")["providers"]
            .get("ccsw-local")
            .is_none()
    );
    assert_eq!(s.get("settings.json")["defaultProvider"], "original");
    cfg["profiles"]["local"]["disabled_models"] = toml::Value::Array(vec![]);
    fs::write(&path, toml::to_string(&cfg).unwrap()).unwrap();
    s.ok(&["pi", "import"]);
    assert!(
        s.get("models.json")["providers"]
            .get("ccsw-local")
            .is_some()
    );
}
