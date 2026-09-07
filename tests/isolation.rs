#![cfg(unix)]

use std::{fs, process::Command};

#[test]
fn cli_exposes_configuration_commands_without_a_claude_launcher() {
    let binary = assert_cmd::cargo::cargo_bin("ccsw");
    let output = Command::new(&binary).arg("--help").output().unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("Manage Claude Code providers, models, and proxy settings"));
    assert!(
        !help
            .lines()
            .any(|line| line.trim_start().starts_with("run "))
    );

    let output = Command::new(binary).arg("run").output().unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("unrecognized subcommand 'run'")
    );
}

#[test]
fn openai_profile_syncs_through_the_private_local_proxy() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("openai.toml");
    fs::write(
        &config,
        r#"
version = 2

[profiles.openai]
name = "OpenAI compatible"
base_url = "https://upstream.invalid/v1/chat/completions"
api_format = "openai-chat"
default_model = "gpt-test[1m]"

[profiles.openai.credential]
kind = "bearer"
value = "upstream-secret"

[profiles.anthropic]
name = "Anthropic compatible"
base_url = "https://anthropic-upstream.invalid"
api_format = "anthropic"
default_model = "claude-test"

[profiles.anthropic.credential]
kind = "x-api-key"
value = "anthropic-upstream-secret"
"#,
    )
    .unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let listen = listener.local_addr().unwrap().to_string();
    drop(listener);
    let binary = assert_cmd::cargo::cargo_bin("ccsw");
    let common = |command: &mut Command| {
        command
            .env("CCSW_CONFIG", &config)
            .env("HOME", temp.path())
            .env("XDG_STATE_HOME", temp.path().join("state"))
            .env("XDG_CACHE_HOME", temp.path().join("cache"));
    };

    let mut start = Command::new(&binary);
    start.args(["proxy", "start", "--listen", &listen]);
    common(&mut start);
    assert!(start.status().unwrap().success());

    let claude_config = temp.path().join("claude");
    let mut apply = Command::new(&binary);
    apply
        .args(["apply", "--profile", "openai"])
        .env("CLAUDE_CONFIG_DIR", &claude_config);
    common(&mut apply);
    assert!(apply.status().unwrap().success());

    let applied = fs::read_to_string(claude_config.join("settings.json")).unwrap();
    assert!(applied.contains(&format!("http://{listen}/r/")));
    assert!(applied.contains("openai::gpt-test[1m]"));
    assert!(applied.contains("anthropic::claude-test"));
    assert!(!applied.contains("upstream-secret"));
    assert!(!applied.contains("anthropic-upstream-secret"));

    let mut stop = Command::new(&binary);
    stop.args(["proxy", "stop"]);
    common(&mut stop);
    assert!(stop.status().unwrap().success());
    let mut restart = Command::new(&binary);
    restart.args(["proxy", "start"]);
    common(&mut restart);
    let output = restart.output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains(&listen));
    let mut stop = Command::new(&binary);
    stop.args(["proxy", "stop"]);
    common(&mut stop);
    assert!(stop.status().unwrap().success());
}

#[test]
fn separate_user_state_can_use_distinct_ports_without_stopping_each_other() {
    struct Sandbox {
        root: tempfile::TempDir,
        binary: std::path::PathBuf,
    }
    impl Sandbox {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            fs::write(root.path().join("config.toml"), "version = 2\n[profiles.local]\nname = 'Local'\nbase_url = 'https://upstream.invalid'\ndefault_model = 'test-model'\n").unwrap();
            Self {
                root,
                binary: assert_cmd::cargo::cargo_bin("ccsw"),
            }
        }
        fn command(&self, args: &[&str]) -> std::process::Output {
            Command::new(&self.binary)
                .args(args)
                .env("HOME", self.root.path())
                .env("CCSW_CONFIG", self.root.path().join("config.toml"))
                .env("XDG_STATE_HOME", self.root.path().join("state"))
                .env("XDG_CACHE_HOME", self.root.path().join("cache"))
                .env("CLAUDE_CONFIG_DIR", self.root.path().join("claude"))
                .output()
                .unwrap()
        }
    }
    impl Drop for Sandbox {
        fn drop(&mut self) {
            self.command(&["proxy", "stop"]);
        }
    }
    let first = Sandbox::new();
    let second = Sandbox::new();
    let first_socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let second_socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let first_address = first_socket.local_addr().unwrap().to_string();
    let second_port = second_socket.local_addr().unwrap().port().to_string();
    drop(first_socket);
    assert!(
        first
            .command(&["proxy", "start", "--listen", &first_address])
            .status
            .success()
    );
    let conflict = second.command(&["proxy", "start", "--listen", &first_address]);
    assert!(!conflict.status.success());
    assert!(String::from_utf8_lossy(&conflict.stderr).contains("another user or process"));
    let rejected = first.command(&["proxy", "port", &second_port]);
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("stop this user's proxy"));
    drop(second_socket);
    assert!(
        second
            .command(&["proxy", "port", &second_port])
            .status
            .success()
    );
    let started = second.command(&["proxy", "start"]);
    assert!(started.status.success());
    assert!(String::from_utf8_lossy(&started.stdout).contains(&format!("127.0.0.1:{second_port}")));
    assert!(
        second
            .command(&["apply", "--profile", "local"])
            .status
            .success()
    );
    let settings = fs::read_to_string(second.root.path().join("claude/settings.json")).unwrap();
    assert!(settings.contains(&format!("127.0.0.1:{second_port}/r/")));
    let still_running = first.command(&["proxy", "status"]);
    assert!(String::from_utf8_lossy(&still_running.stdout).starts_with("running"));
    assert!(String::from_utf8_lossy(&still_running.stdout).contains(&first_address));
}
