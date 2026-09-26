use std::{fs, path::Path, process::Command};

pub fn isolate(command: &mut Command, root: &Path) {
    fs::create_dir_all(root.join("tmp")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let bin = root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let herdr = bin.join("herdr");
        if !herdr.exists() {
            fs::write(
                &herdr,
                "#!/bin/sh\nif [ \"$1 $2\" = 'plugin list' ]; then\n  printf '{\"result\":{\"plugins\":[]}}\\n'\n  exit 0\nfi\nprintf 'Unexpected Herdr command in isolated test: %s\\n' \"$*\" >&2\nexit 1\n",
            )
            .unwrap();
            fs::set_permissions(&herdr, fs::Permissions::from_mode(0o755)).unwrap();
        }
        command.env(
            "PATH",
            format!(
                "{}:{}",
                bin.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        );
    }
    for key in [
        "HOME",
        "USERPROFILE",
        "HOMEDRIVE",
        "HOMEPATH",
        "APPDATA",
        "LOCALAPPDATA",
        "CCSW_CONFIG",
        "CLAUDE_CONFIG_DIR",
        "CODEX_HOME",
        "GROK_HOME",
        "PI_CODING_AGENT_DIR",
        "XDG_CONFIG_HOME",
        "XDG_STATE_HOME",
        "XDG_CACHE_HOME",
        "CCSW_CODEX_BIN",
        "CCSW_CLAUDE_BIN",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_BASE_URL",
        "OPENAI_API_KEY",
        "CODEX_ACCESS_TOKEN",
        "CODEX_AUTH",
        "HERDR_ENV",
        "HERDR_SOCKET_PATH",
        "HERDR_CONFIG_PATH",
        "HERDR_BIN_PATH",
        "HERDR_WORKSPACE_ID",
        "HERDR_TAB_ID",
        "HERDR_PANE_ID",
    ] {
        command.env_remove(key);
    }
    command
        .env("HOME", root)
        .env("USERPROFILE", root)
        .env("APPDATA", root.join("roaming"))
        .env("LOCALAPPDATA", root.join("local"))
        .env("CCSW_CONFIG", root.join("config.toml"))
        .env("XDG_CONFIG_HOME", root.join("config-root"))
        .env("XDG_STATE_HOME", root.join("state"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("CLAUDE_CONFIG_DIR", root.join("claude"))
        .env("CODEX_HOME", root.join("codex"))
        .env("GROK_HOME", root.join("grok"))
        .env("PI_CODING_AGENT_DIR", root.join("pi"))
        .env("TMP", root.join("tmp"))
        .env("TEMP", root.join("tmp"))
        .env("TMPDIR", root.join("tmp"));
}

pub fn command(root: &Path) -> Command {
    let mut command = Command::new(assert_cmd::cargo::cargo_bin("ccsw"));
    isolate(&mut command, root);
    command
}
