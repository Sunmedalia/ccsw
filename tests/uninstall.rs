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
            "{args:?}\n{}\n{}",
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

#[cfg(unix)]
#[test]
fn uninstall_herdr_removes_only_the_ccsw_link_and_shortcut() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = Sandbox::new();
    let root = sandbox.root.path();
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap();
    let fake = bin.join("herdr");
    fs::write(&fake, "#!/bin/sh\nif [ \"$1 $2\" = 'plugin list' ]; then cat \"$HERDR_LIST_FIXTURE\"; fi\nexit 0\n").unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    let list = root.join("plugin-list.json");
    fs::write(&list, serde_json::json!({"result":{"plugins":[{"plugin_id":"ccsw","plugin_root":env!("CARGO_MANIFEST_DIR"),"source":{"kind":"local"}}]}}).to_string()).unwrap();
    let herdr_config = root.join("herdr.toml");
    fs::write(&herdr_config, "[[keys.command]]\nkey='prefix+u'\ntype='plugin_action'\ncommand='ccsw.open'\n[[keys.command]]\nkey='prefix+x'\ntype='plugin_action'\ncommand='files.open'\n").unwrap();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let run = |args: &[&str]| {
        support::command(root)
            .args(args)
            .env("HOME", root)
            .env("CCSW_CONFIG", root.join("config.toml"))
            .env("XDG_STATE_HOME", root.join("state"))
            .env("XDG_CACHE_HOME", root.join("cache"))
            .env("CLAUDE_CONFIG_DIR", root.join("claude"))
            .env("HERDR_ENV", "1")
            .env("HERDR_CONFIG_PATH", &herdr_config)
            .env("HERDR_LIST_FIXTURE", &list)
            .env("PATH", &path)
            .output()
            .unwrap()
    };
    let preview = run(&["uninstall", "--dry-run", "--herdr"]);
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    assert!(String::from_utf8_lossy(&preview.stdout).contains("Unlink local CCSW Herdr plugin"));
    assert!(
        fs::read_to_string(&herdr_config)
            .unwrap()
            .contains("ccsw.open")
    );
    let result = run(&["uninstall", "--yes", "--herdr"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let after = fs::read_to_string(&herdr_config).unwrap();
    assert!(!after.contains("ccsw.open"));
    assert!(after.contains("files.open"));
}
