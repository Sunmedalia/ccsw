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
