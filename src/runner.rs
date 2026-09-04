use std::{
    ffi::{OsStr, OsString},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tempfile::NamedTempFile;
use uuid::Uuid;

use crate::config::{AppPaths, Credential, ModelEntry, Profile, set_private};

const CLEARED_PROVIDER_VARS: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_MODEL",
    "ANTHROPIC_DEFAULT_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
    "ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
    "ANTHROPIC_DEFAULT_SONNET_MODEL_DESCRIPTION",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL_DESCRIPTION",
    "ANTHROPIC_DEFAULT_FABLE_MODEL",
    "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME",
    "ANTHROPIC_DEFAULT_FABLE_MODEL_DESCRIPTION",
    "CLAUDE_CODE_SUBAGENT_MODEL",
    "CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_USE_ANTHROPIC_AWS",
];

#[derive(Debug, Clone)]
pub enum SessionMode {
    New,
    Resume(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCapture {
    pub session_id: String,
    pub model_id: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub event: Option<String>,
}

pub struct LaunchResult {
    pub status: ExitStatus,
    pub capture: SessionCapture,
}

pub struct LaunchRequest<'a> {
    pub profile: &'a Profile,
    pub model_id: &'a str,
    pub models: &'a [ModelEntry],
    pub mode: SessionMode,
    pub forwarded_args: Vec<OsString>,
}

pub fn launch(paths: &AppPaths, request: LaunchRequest<'_>) -> Result<LaunchResult> {
    if !request.profile.enabled {
        bail!("provider is disabled");
    }
    fs::create_dir_all(&paths.runtime_dir)?;
    let initial_session = match &request.mode {
        SessionMode::New => Uuid::new_v4().to_string(),
        SessionMode::Resume(id) => id.clone(),
    };
    let capture_path = paths
        .runtime_dir
        .join(format!("capture-{}.json", Uuid::new_v4()));
    write_capture(
        &capture_path,
        &SessionCapture {
            session_id: initial_session.clone(),
            model_id: request.model_id.to_owned(),
            cwd: None,
            event: Some("ccsw-launch".into()),
        },
    )?;

    let (forwarded, extra_settings) = extract_forwarded_settings(request.forwarded_args)?;
    validate_forwarded_args(&forwarded)?;
    let executable = std::env::current_exe().context("cannot resolve ccsw executable")?;
    let runtime_settings =
        build_runtime_settings(extra_settings, request.models, &executable, &capture_path)?;
    let mut settings_file = NamedTempFile::new_in(&paths.runtime_dir)?;
    settings_file.write_all(serde_json::to_string_pretty(&runtime_settings)?.as_bytes())?;
    settings_file.as_file().sync_all()?;
    set_private(settings_file.path())?;

    let claude_bin = std::env::var_os("CCSW_CLAUDE_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("claude"));
    let mut command = build_command(
        &claude_bin,
        request.profile,
        request.model_id,
        &request.mode,
        &initial_session,
        settings_file.path(),
        &forwarded,
    );
    let status = command
        .status()
        .with_context(|| format!("failed to start {}", claude_bin.display()))?;
    let capture = read_capture(&capture_path).unwrap_or(SessionCapture {
        session_id: initial_session,
        model_id: request.model_id.to_owned(),
        cwd: None,
        event: Some("capture-unavailable".into()),
    });
    fs::remove_file(capture_path).ok();
    Ok(LaunchResult { status, capture })
}

pub fn build_command(
    claude_bin: &Path,
    profile: &Profile,
    model_id: &str,
    mode: &SessionMode,
    initial_session: &str,
    runtime_settings: &Path,
    forwarded: &[OsString],
) -> Command {
    let mut command = Command::new(claude_bin);
    for name in CLEARED_PROVIDER_VARS {
        command.env_remove(name);
    }
    command
        .env("CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST", "1")
        .env("ANTHROPIC_BASE_URL", &profile.base_url);
    match &profile.credential {
        Credential::Bearer { value } => {
            command.env("ANTHROPIC_AUTH_TOKEN", value);
        }
        Credential::XApiKey { value } | Credential::ApiKey { value } => {
            command.env("ANTHROPIC_API_KEY", value);
        }
        Credential::None => {}
    }
    for (role, model) in profile.aliases.iter() {
        let prefix = format!("ANTHROPIC_DEFAULT_{}_MODEL", role.to_ascii_uppercase());
        command.env(&prefix, model);
        if let Some(entry) = profile.manual_model(model) {
            if let Some(label) = &entry.label {
                command.env(format!("{prefix}_NAME"), label);
            }
            if let Some(description) = &entry.description {
                command.env(format!("{prefix}_DESCRIPTION"), description);
            }
        }
    }
    if let Some(model) = &profile.subagent_model {
        command.env("CLAUDE_CODE_SUBAGENT_MODEL", model);
    }

    match mode {
        SessionMode::New => {
            command.arg("--session-id").arg(initial_session);
        }
        SessionMode::Resume(id) => {
            command.arg("--resume").arg(id);
        }
    }
    command.arg("--model").arg(model_id);
    if !profile.fallback_models.is_empty() {
        command
            .arg("--fallback-model")
            .arg(profile.fallback_models.join(","));
    }
    command
        .arg("--settings")
        .arg(runtime_settings)
        .args(forwarded);
    command
}

fn build_runtime_settings(
    base: Option<Value>,
    models: &[ModelEntry],
    executable: &Path,
    capture_path: &Path,
) -> Result<Value> {
    let mut root = match base.unwrap_or_else(|| json!({})) {
        Value::Object(root) => root,
        _ => bail!("forwarded --settings must contain a JSON object"),
    };
    root.insert(
        "modelPicker".into(),
        json!({
            "replaceBuiltInOptions": true,
            "options": models.iter().map(model_picker_row).collect::<Vec<_>>()
        }),
    );
    let command = format!(
        "{} internal capture-session --path {}",
        shell_quote(executable.as_os_str()),
        shell_quote(capture_path.as_os_str())
    );
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .context("hooks in forwarded --settings must be an object")?;
    for event in ["SessionStart", "PostModelSwitch", "SessionEnd"] {
        let entry = hooks
            .entry(event)
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .with_context(|| format!("hooks.{event} must be an array"))?;
        entry.push(json!({
            "hooks": [{
                "type": "command",
                "command": command,
                "timeout": 5
            }]
        }));
    }
    Ok(Value::Object(root))
}

fn model_picker_row(model: &ModelEntry) -> Value {
    let mut row = Map::new();
    row.insert("model".into(), Value::String(model.id.clone()));
    if let Some(label) = &model.label {
        row.insert("label".into(), Value::String(label.clone()));
    }
    if let Some(description) = &model.description {
        row.insert("description".into(), Value::String(description.clone()));
    }
    Value::Object(row)
}

fn extract_forwarded_settings(args: Vec<OsString>) -> Result<(Vec<OsString>, Option<Value>)> {
    let mut result = Vec::new();
    let mut settings = None;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if arg == "--settings" {
            let value = iter.next().context("--settings requires a value")?;
            if settings.is_some() {
                bail!("only one forwarded --settings value is supported");
            }
            settings = Some(read_settings_value(&value)?);
        } else if let Some(value) = arg.to_str().and_then(|arg| arg.strip_prefix("--settings=")) {
            if settings.is_some() {
                bail!("only one forwarded --settings value is supported");
            }
            settings = Some(read_settings_value(OsStr::new(value))?);
        } else {
            result.push(arg);
        }
    }
    Ok((result, settings))
}

fn read_settings_value(value: &OsStr) -> Result<Value> {
    let text = value.to_string_lossy();
    let content = if text.trim_start().starts_with('{') {
        text.into_owned()
    } else {
        fs::read_to_string(Path::new(value))
            .with_context(|| format!("failed to read forwarded --settings {}", text))?
    };
    serde_json::from_str(&content).context("forwarded --settings is not valid JSON")
}

fn validate_forwarded_args(args: &[OsString]) -> Result<()> {
    let reserved = [
        "--model",
        "--resume",
        "-r",
        "--continue",
        "-c",
        "--session-id",
        "--fallback-model",
    ];
    for arg in args {
        let text = arg.to_string_lossy();
        if reserved
            .iter()
            .any(|reserved| text == *reserved || text.starts_with(&format!("{reserved}=")))
        {
            bail!("{text} is managed by ccsw and cannot be forwarded");
        }
    }
    Ok(())
}

fn shell_quote(value: &OsStr) -> String {
    let value = value.to_string_lossy();
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub fn capture_session(path: &Path) -> Result<()> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let value: Value = serde_json::from_str(&input).context("invalid hook input")?;
    capture_value(path, &value)
}

fn capture_value(path: &Path, value: &Value) -> Result<()> {
    let previous = read_capture(path).ok();
    let session_id = value
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| previous.as_ref().map(|capture| capture.session_id.clone()))
        .context("hook input has no session_id")?;
    let model_id = value
        .get("model")
        .or_else(|| value.get("new_model"))
        .or_else(|| value.get("requested_model"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| previous.as_ref().map(|capture| capture.model_id.clone()))
        .unwrap_or_default();
    let capture = SessionCapture {
        session_id,
        model_id,
        cwd: value
            .get("cwd")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| previous.as_ref().and_then(|capture| capture.cwd.clone())),
        event: value
            .get("hook_event_name")
            .and_then(Value::as_str)
            .map(str::to_owned),
    };
    write_capture(path, &capture)
}

fn read_capture(path: &Path) -> Result<SessionCapture> {
    serde_json::from_slice(&fs::read(path)?).context("invalid session capture")
}

fn write_capture(path: &Path, capture: &SessionCapture) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut temp = NamedTempFile::new_in(path.parent().context("capture path has no parent")?)?;
    temp.write_all(&serde_json::to_vec(capture)?)?;
    set_private(temp.path())?;
    temp.persist(path).map_err(|error| error.error)?;
    set_private(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RoleModels;

    fn profile(token: &str) -> Profile {
        Profile {
            name: "Test".into(),
            enabled: true,
            base_url: "https://gateway.example".into(),
            api_format: crate::config::ApiFormat::Anthropic,
            credential: Credential::Bearer {
                value: token.into(),
            },
            default_model: "model-a".into(),
            aliases: RoleModels {
                opus: Some("model-a".into()),
                ..Default::default()
            },
            subagent_model: Some("model-b".into()),
            fallback_models: vec!["model-b".into()],
            enabled_models: vec![],
            disabled_models: vec![],
            models: vec![],
        }
    }

    #[test]
    fn commands_keep_profile_environments_isolated() {
        let settings = Path::new("/tmp/settings.json");
        let a = build_command(
            Path::new("claude"),
            &profile("token-a"),
            "model-a",
            &SessionMode::Resume("session-a".into()),
            "session-a",
            settings,
            &[],
        );
        let b = build_command(
            Path::new("claude"),
            &profile("token-b"),
            "model-b",
            &SessionMode::Resume("session-b".into()),
            "session-b",
            settings,
            &[],
        );
        let env = |command: &Command, key: &str| {
            command
                .get_envs()
                .find(|(name, _)| *name == OsStr::new(key))
                .and_then(|(_, value)| value)
                .unwrap()
                .to_string_lossy()
                .into_owned()
        };
        assert_eq!(env(&a, "ANTHROPIC_AUTH_TOKEN"), "token-a");
        assert_eq!(env(&b, "ANTHROPIC_AUTH_TOKEN"), "token-b");
        assert_eq!(env(&a, "CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST"), "1");
        assert!(
            a.get_envs()
                .find(|(name, _)| *name == OsStr::new("CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY"))
                .is_some_and(|(_, value)| value.is_none())
        );
    }

    #[test]
    fn rejects_reserved_passthrough_flags() {
        assert!(validate_forwarded_args(&["--model=opus".into()]).is_err());
        assert!(validate_forwarded_args(&["--verbose".into()]).is_ok());
    }

    #[test]
    fn runtime_settings_never_contain_credentials() {
        let models = vec![ModelEntry {
            id: "model-a".into(),
            label: None,
            description: None,
        }];
        let value = build_runtime_settings(
            Some(json!({"permissions": {"allow": ["Bash(git status)"]}})),
            &models,
            Path::new("/usr/bin/ccsw"),
            Path::new("/tmp/capture.json"),
        )
        .unwrap();
        let encoded = serde_json::to_string(&value).unwrap();
        assert!(encoded.contains("modelPicker"));
        assert!(encoded.contains("Bash(git status)"));
        assert!(!encoded.contains("credential"));
        assert!(value["modelPicker"]["options"][0].get("label").is_none());
    }

    #[test]
    fn hooks_track_clear_and_model_switch() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("capture.json");
        capture_value(
            &path,
            &json!({
                "session_id": "session-after-clear",
                "model": "model-a",
                "cwd": "/project",
                "hook_event_name": "SessionStart"
            }),
        )
        .unwrap();
        capture_value(
            &path,
            &json!({
                "session_id": "session-after-clear",
                "model": "model-b",
                "hook_event_name": "PostModelSwitch"
            }),
        )
        .unwrap();
        capture_value(
            &path,
            &json!({
                "session_id": "session-after-clear",
                "hook_event_name": "SessionEnd",
                "reason": "prompt_input_exit"
            }),
        )
        .unwrap();
        let capture = read_capture(&path).unwrap();
        assert_eq!(capture.session_id, "session-after-clear");
        assert_eq!(capture.model_id, "model-b");
        assert_eq!(capture.event.as_deref(), Some("SessionEnd"));
    }
}
