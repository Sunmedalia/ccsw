mod support;
use std::{fs, process::Output};

struct Sandbox {
    root: tempfile::TempDir,
}
impl Sandbox {
    fn new() -> Self {
        let sandbox = Self {
            root: tempfile::tempdir().unwrap(),
        };
        fs::write(sandbox.root.path().join("config.toml"), "version = 2\n[profiles.local]\nname='Local'\nbase_url='https://example.invalid'\ndefault_model='model'\n").unwrap();
        sandbox
    }
    fn command(&self, args: &[&str]) -> Output {
        support::command(self.root.path())
            .args(args)
            .env("HOME", self.root.path())
            .env("USERPROFILE", self.root.path())
            .env("APPDATA", self.root.path().join("roaming"))
            .env("LOCALAPPDATA", self.root.path().join("local"))
            .env("CCSW_CONFIG", self.root.path().join("config.toml"))
            .env("XDG_STATE_HOME", self.root.path().join("state"))
            .env("XDG_CACHE_HOME", self.root.path().join("cache"))
            .env("CLAUDE_CONFIG_DIR", self.root.path().join("claude"))
            .output()
            .unwrap()
    }
    fn ok(&self, args: &[&str]) {
        let result = self.command(args);
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
}
#[test]
fn uninstall_preview_and_idempotent_cleanup_preserve_unrelated_files() {
    let sandbox = Sandbox::new();
    let config = sandbox.root.path().join("config.toml");
    let original = fs::read(&config).unwrap();
    fs::write(sandbox.root.path().join("keep.txt"), "precious").unwrap();
    sandbox.ok(&["uninstall", "--dry-run"]);
    assert_eq!(fs::read(&config).unwrap(), original);
    assert!(!sandbox.root.path().join("state").exists());
    sandbox.ok(&["uninstall", "--yes"]);
    assert!(!config.exists());
    sandbox.ok(&["uninstall", "--yes"]);
    assert_eq!(
        fs::read_to_string(sandbox.root.path().join("keep.txt")).unwrap(),
        "precious"
    );
}
#[test]
fn uninstall_stops_authenticated_proxy_and_detaches_only_managed_settings() {
    let sandbox = Sandbox::new();
    fs::create_dir(sandbox.root.path().join("claude")).unwrap();
    fs::write(
        sandbox.root.path().join("claude/settings.json"),
        r#"{"theme":"dark","env":{"KEEP":"yes"}}"#,
    )
    .unwrap();
    let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = socket.local_addr().unwrap().to_string();
    drop(socket);
    sandbox.ok(&["proxy", "start", "--listen", &address]);
    sandbox.ok(&["apply", "--profile", "local"]);
    sandbox.ok(&["uninstall", "--yes"]);
    let value: serde_json::Value = serde_json::from_slice(
        &fs::read(sandbox.root.path().join("claude/settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        value,
        serde_json::json!({"theme":"dark","env":{"KEEP":"yes"}})
    );
    assert!(!sandbox.root.path().join("state/ccsw").exists());
    assert!(
        !sandbox
            .root
            .path()
            .join("claude/settings.json.ccsw-backup")
            .exists()
    );
    assert!(std::net::TcpListener::bind(address).is_ok());
}

#[test]
fn uninstall_verifies_nonempty_lock_files_while_holding_their_locks() {
    let sandbox = Sandbox::new();
    let state = sandbox.root.path().join("state/ccsw");
    fs::create_dir_all(&state).unwrap();
    for name in ["session.lock", "codex.lock", "pi.lock", "sync-state.lock"] {
        fs::write(state.join(name), b"lock fixture\n").unwrap();
    }
    fs::write(
        sandbox.root.path().join("config.toml.lock"),
        b"lock fixture\n",
    )
    .unwrap();
    sandbox.ok(&["uninstall", "--yes"]);
    assert!(!state.exists());
    assert!(!sandbox.root.path().join("config.toml.lock").exists());
}
