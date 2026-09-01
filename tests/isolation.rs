#![cfg(unix)]

use std::{fs, os::unix::fs::PermissionsExt, process::Command, thread};

fn write_fixture(root: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let config = root.join("config.toml");
    fs::write(
        &config,
        r#"
version = 2

[profiles.a]
name = "Gateway A"
base_url = "https://a.example"
api_format = "anthropic"
default_model = "model-a"

[profiles.a.credential]
kind = "bearer"
value = "token-a"

[profiles.b]
name = "Gateway B"
base_url = "https://b.example"
api_format = "anthropic"
default_model = "model-b"

[profiles.b.credential]
kind = "bearer"
value = "token-b"
"#,
    )
    .unwrap();
    let fake = root.join("fake-claude");
    fs::write(
        &fake,
        r#"#!/bin/sh
printf '%s|%s|%s|%s\n' "$ANTHROPIC_BASE_URL" "$ANTHROPIC_AUTH_TOKEN" "$CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST" "$*" > "$FAKE_OUTPUT"
previous=''
for arg in "$@"; do
  if [ "$previous" = '--settings' ]; then
    cp "$arg" "$FAKE_SETTINGS_OUTPUT"
    break
  fi
  previous="$arg"
done
"#,
    )
    .unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    (config, fake)
}

#[test]
fn concurrent_profiles_do_not_share_provider_environment() {
    let temp = tempfile::tempdir().unwrap();
    let (config, fake) = write_fixture(temp.path());
    let binary = assert_cmd::cargo::cargo_bin("ccsw");
    let run = |profile: &'static str, output_name: &'static str| {
        let config = config.clone();
        let fake = fake.clone();
        let binary = binary.clone();
        let root = temp.path().to_path_buf();
        thread::spawn(move || {
            let output = root.join(output_name);
            let settings_output = root.join(format!("{output_name}.settings"));
            let status = Command::new(binary)
                .args(["run", "--profile", profile])
                .env("CCSW_CONFIG", config)
                .env("CCSW_CLAUDE_BIN", fake)
                .env("FAKE_OUTPUT", &output)
                .env("FAKE_SETTINGS_OUTPUT", &settings_output)
                .env("HOME", &root)
                .env("XDG_STATE_HOME", root.join("state"))
                .env("XDG_CACHE_HOME", root.join("cache"))
                .status()
                .unwrap();
            assert!(status.success());
            (
                fs::read_to_string(output).unwrap(),
                fs::read_to_string(settings_output).unwrap(),
            )
        })
    };
    let a = run("a", "a.out");
    let b = run("b", "b.out");
    let (a, a_settings) = a.join().unwrap();
    let (b, b_settings) = b.join().unwrap();
    assert!(a.contains("https://a.example|token-a|1|"));
    assert!(a.contains("--model model-a"));
    assert!(!a.contains("token-b"));
    assert!(b.contains("https://b.example|token-b|1|"));
    assert!(b.contains("--model model-b"));
    assert!(!b.contains("token-a"));
    assert!(a_settings.contains("modelPicker"));
    assert!(a_settings.contains("SessionStart"));
    assert!(!a_settings.contains("token-a"));
    assert!(!b_settings.contains("token-b"));

    let original = fs::read_to_string(config).unwrap();
    assert!(original.contains("token-a"));
    assert!(original.contains("token-b"));
}

#[test]
fn openai_profile_routes_claude_through_private_local_proxy() {
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
"#,
    )
    .unwrap();
    let fake = temp.path().join("fake-claude-openai");
    fs::write(
        &fake,
        r#"#!/bin/sh
printf '%s|%s\n' "$ANTHROPIC_BASE_URL" "$ANTHROPIC_AUTH_TOKEN" > "$FAKE_OUTPUT"
curl -sS "$ANTHROPIC_BASE_URL/v1/messages/count_tokens" \
  -H "Authorization: Bearer $ANTHROPIC_AUTH_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"model":"gpt-test[1m]","messages":[{"role":"user","content":"hello"}]}' >> "$FAKE_OUTPUT"
previous=''
for arg in "$@"; do
  if [ "$previous" = '--settings' ]; then
    cp "$arg" "$FAKE_SETTINGS_OUTPUT"
    break
  fi
  previous="$arg"
done
"#,
    )
    .unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
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

    let output = temp.path().join("openai.out");
    let settings = temp.path().join("openai.settings");
    let mut run = Command::new(&binary);
    run.args(["run", "--profile", "openai"])
        .env("CCSW_CLAUDE_BIN", &fake)
        .env("FAKE_OUTPUT", &output)
        .env("FAKE_SETTINGS_OUTPUT", &settings);
    common(&mut run);
    assert!(run.status().unwrap().success());

    let captured = fs::read_to_string(output).unwrap();
    assert!(captured.starts_with(&format!("http://{listen}/r/")));
    assert!(!captured.contains("upstream-secret"));
    assert!(captured.contains("input_tokens"));
    let runtime_settings = fs::read_to_string(settings).unwrap();
    assert!(!runtime_settings.contains("upstream-secret"));

    let claude_config = temp.path().join("claude");
    let mut apply = Command::new(&binary);
    apply
        .args(["apply", "--profile", "openai"])
        .env("CLAUDE_CONFIG_DIR", &claude_config);
    common(&mut apply);
    assert!(apply.status().unwrap().success());
    let applied = fs::read_to_string(claude_config.join("settings.json")).unwrap();
    assert!(applied.contains(&format!("http://{listen}/r/")));
    assert!(applied.contains("gpt-test[1m]"));
    assert!(!applied.contains("upstream-secret"));

    let mut stop = Command::new(binary);
    stop.args(["proxy", "stop"]);
    common(&mut stop);
    assert!(stop.status().unwrap().success());
}
