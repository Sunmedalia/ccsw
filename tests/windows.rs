#![cfg(windows)]
#[allow(dead_code)]
#[path = "../src/platform.rs"]
mod platform;
mod support;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[test]
fn windows_directory_precedence_empty_overrides_and_literal_paths() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("用户 space & %literal% !^()");
    fs::create_dir_all(&root).unwrap();
    let run = |extra: &[(&str, &str)]| {
        let mut command = support::command(&root);
        command
            .args(["config", "path"])
            .env("CCSW_CONFIG", "")
            .env("XDG_CONFIG_HOME", "");
        for (key, value) in extra {
            command.env(key, value);
        }
        command.output().unwrap()
    };
    let output = run(&[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        Path::new(String::from_utf8(output.stdout).unwrap().trim()),
        root.join("roaming/ccsw/config.toml")
    );
    let output = run(&[("APPDATA", ""), ("HOME", "Z:\\wrong-home")]);
    assert!(output.status.success());
    assert_eq!(
        Path::new(String::from_utf8(output.stdout).unwrap().trim()),
        root.join("AppData/Roaming/ccsw/config.toml")
    );
    let output = support::command(&root)
        .args(["config", "path"])
        .env_remove("HOME")
        .env_remove("USERPROFILE")
        .env_remove("HOMEDRIVE")
        .env_remove("HOMEPATH")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "complete overrides do not need HOME: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = support::command(&root)
        .args(["config", "path"])
        .env("ccsw_config", root.join("小写 config.toml"))
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        Path::new(String::from_utf8(output.stdout).unwrap().trim()),
        root.join("小写 config.toml")
    );
}

#[test]
fn pi_expands_windows_tilde_without_home() {
    let temp = tempfile::tempdir().unwrap();
    let output = support::command(temp.path())
        .args(["pi", "files"])
        .env_remove("HOME")
        .env("PI_CODING_AGENT_DIR", "~\\自定义 pi")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains(&temp.path().join("自定义 pi").display().to_string())
    );
}

#[test]
fn path_identity_handles_existing_extended_paths_and_missing_children() {
    let temp = tempfile::tempdir().unwrap();
    let canonical = fs::canonicalize(temp.path()).unwrap();
    assert!(platform::same_path(temp.path(), &canonical).unwrap());
    assert!(
        platform::same_path(
            &temp.path().join("not-created/config.toml"),
            &canonical.join("not-created/config.toml")
        )
        .unwrap()
    );
}

#[cfg(feature = "test-support")]
fn install_helper(directory: &Path) -> PathBuf {
    fs::create_dir_all(directory).unwrap();
    let binary = directory.join("client helper.exe");
    fs::copy(env!("CARGO_BIN_EXE_ccsw-test-helper"), &binary).unwrap();
    binary
}

#[cfg(feature = "test-support")]
#[test]
fn doctor_finds_native_and_batch_clients_and_preserves_environment() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("用户 app & %literal% !^()");
    let binary = install_helper(&root);
    let record = root.join("record.json");
    let literal = "secret %PATH% !VALUE! & ^ ( ) \\\" $value 🦀";
    let run = |program: &Path| {
        support::command(&root)
            .arg("doctor")
            .env("CCSW_CLAUDE_BIN", program)
            .env("CCSW_HELPER_RECORD", &record)
            .env("CCSW_HELPER_LITERAL", literal)
            .output()
            .unwrap()
    };
    let output = run(&binary);
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("Claude Code 2.1.242"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let value: serde_json::Value = serde_json::from_slice(&fs::read(&record).unwrap()).unwrap();
    assert_eq!(value["args"], serde_json::json!(["--version"]));
    assert_eq!(value["literal"], literal);
    // Normal npm-style shim, including a spaced/Unicode directory and basename.
    let shim_dir = temp.path().join("用户 shim folder");
    install_helper(&shim_dir);
    let shim = shim_dir.join("claude.cmd");
    fs::write(&shim, "@echo off\r\n\"%~dp0client helper.exe\" %*\r\n").unwrap();
    let output = run(&shim);
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("Claude Code 2.1.242"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let output = support::command(&root)
        .arg("doctor")
        .env("CCSW_CLAUDE_BIN", "claude")
        .env("PATH", &shim_dir)
        .env("PATHEXT", ".CMD;.EXE")
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&output.stdout).contains("Claude Code 2.1.242"));
    let output = run(Path::new("\"wrong quoted.exe\""));
    assert!(String::from_utf8_lossy(&output.stdout).contains("without enclosing quotes"));
}

#[cfg(feature = "test-support")]
#[test]
fn doctor_timeout_reaps_launcher_and_descendant() {
    use ::windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use ::windows::Win32::System::Threading::{
        OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };
    let temp = tempfile::tempdir().unwrap();
    let binary = install_helper(temp.path());
    let pid_path = temp.path().join("descendant.pid");
    let output = support::command(temp.path())
        .arg("doctor")
        .env("CCSW_CLAUDE_BIN", binary)
        .env("CCSW_HELPER_TREE", "1")
        .env("CCSW_HELPER_PID", &pid_path)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&output.stdout).contains("timed out"));
    let pid: u32 = fs::read_to_string(pid_path).unwrap().parse().unwrap();
    // SAFETY: read-only synchronization handle to the PID created by this fixture.
    unsafe {
        if let Ok(handle) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            let state = WaitForSingleObject(handle, 3000);
            CloseHandle(handle).unwrap();
            assert_eq!(state, WAIT_OBJECT_0, "descendant is still running");
        }
    }
}

#[test]
fn uninstall_rejects_junction_without_deleting_target() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir(root.join("victim")).unwrap();
    fs::write(root.join("victim/config.toml"), "version = 3\n").unwrap();
    let junction = root.join("junction");
    // Only fixture-generated paths enter this shell operation.
    let result = std::process::Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&junction)
        .arg(root.join("victim"))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output = support::command(root)
        .args(["uninstall", "--yes"])
        .env("CCSW_CONFIG", junction.join("config.toml"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(
        fs::read_to_string(root.join("victim/config.toml")).unwrap(),
        "version = 3\n"
    );
    fs::remove_dir(junction).unwrap();
}

#[test]
fn startup_uses_registry_directory_before_taking_session_lock() {
    let root = tempfile::tempdir().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port().to_string();
    drop(listener);
    assert!(
        support::command(root.path())
            .args(["proxy", "port", &port])
            .output()
            .unwrap()
            .status
            .success()
    );
    let registry = root.path().join("state/ccsw/proxy.json");
    let output = support::command(root.path())
        .args(["internal", "proxy-start", "--registry"])
        .arg(&registry)
        .env("XDG_STATE_HOME", root.path().join("unrelated-state"))
        .output()
        .unwrap();
    let stopped = support::command(root.path())
        .args(["proxy", "stop"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stopped.status.success());
    assert!(!root.path().join("unrelated-state").exists());
}

#[test]
fn extended_long_state_path_survives_proxy_restart() {
    let root = tempfile::tempdir().unwrap();
    // An explicit extended path also works on hosts without LongPathsEnabled.
    let state = fs::canonicalize(root.path())
        .unwrap()
        .join("long-directory-component-".repeat(5))
        .join("another-long-directory-component-".repeat(5));
    assert!(state.as_os_str().len() > 260);
    let run = |args: &[&str]| {
        support::command(root.path())
            .args(args)
            .env("XDG_STATE_HOME", &state)
            .output()
            .unwrap()
    };
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port().to_string();
    drop(listener);
    let configured = run(&["proxy", "port", &port]);
    assert!(
        configured.status.success(),
        "{}",
        String::from_utf8_lossy(&configured.stderr)
    );
    for _ in 0..2 {
        let started = run(&["proxy", "start"]);
        let stopped = run(&["proxy", "stop"]);
        assert!(
            started.status.success(),
            "{}",
            String::from_utf8_lossy(&started.stderr)
        );
        assert!(
            stopped.status.success(),
            "{}",
            String::from_utf8_lossy(&stopped.stderr)
        );
    }
    assert!(state.join("ccsw/proxy.json").is_file());
}
