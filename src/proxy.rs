use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, OpenOptions},
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path as AxumPath, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use bytes::Bytes;
use fs2::FileExt;
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tempfile::NamedTempFile;
use url::Url;
use uuid::Uuid;

use crate::config::{self, ApiFormat, AppPaths, Credential, Profile, set_private};

const DEFAULT_LISTEN: &str = "127.0.0.1:17321";
const MAX_ERROR_BODY: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RouteTarget {
    config_path: PathBuf,
    profile_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Registry {
    listen: String,
    local_token: String,
    #[serde(default)]
    routes: BTreeMap<String, RouteTarget>,
}

#[derive(Debug, Clone)]
pub struct LocalRoute {
    pub base_url: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProxyStatus {
    pub running: bool,
    pub listen: String,
    pub routes: usize,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone)]
struct ProxyPaths {
    registry: PathBuf,
    registry_lock: PathBuf,
    daemon_lock: PathBuf,
    pid: PathBuf,
    log: PathBuf,
}

impl ProxyPaths {
    fn from_app(paths: &AppPaths) -> Result<Self> {
        let directory = paths.state.parent().context("state path has no parent")?;
        Ok(Self {
            registry: directory.join("proxy.json"),
            registry_lock: directory.join("proxy.json.lock"),
            daemon_lock: directory.join("proxy.daemon.lock"),
            pid: directory.join("proxy.pid"),
            log: directory.join("proxy.log"),
        })
    }
}

pub fn ensure_route(paths: &AppPaths, profile_id: &str, profile: &Profile) -> Result<LocalRoute> {
    if !profile.api_format.is_openai() {
        bail!("ensure_route only accepts OpenAI-compatible profiles");
    }
    let proxy_paths = ProxyPaths::from_app(paths)?;
    let route_id = update_registry(&proxy_paths, None, |registry| {
        if let Some((id, _)) = registry.routes.iter().find(|(_, target)| {
            target.config_path == paths.config && target.profile_id == profile_id
        }) {
            return id.clone();
        }
        let id = Uuid::new_v4().simple().to_string();
        registry.routes.insert(
            id.clone(),
            RouteTarget {
                config_path: paths.config.clone(),
                profile_id: profile_id.to_owned(),
            },
        );
        id
    })?;
    start(paths, None)?;
    let registry = load_registry(&proxy_paths)?;
    Ok(LocalRoute {
        base_url: format!("http://{}/r/{route_id}", registry.listen),
        token: registry.local_token,
    })
}

pub fn routed_profile(paths: &AppPaths, profile_id: &str, profile: &Profile) -> Result<Profile> {
    if !profile.api_format.is_openai() {
        return Ok(profile.clone());
    }
    let local = ensure_route(paths, profile_id, profile)?;
    let mut routed = profile.clone();
    routed.api_format = ApiFormat::Anthropic;
    routed.base_url = local.base_url;
    routed.credential = Credential::Bearer { value: local.token };
    Ok(routed)
}

pub fn start(paths: &AppPaths, listen: Option<&str>) -> Result<ProxyStatus> {
    let proxy_paths = ProxyPaths::from_app(paths)?;
    if let Ok(status) = status(paths)
        && status.running
    {
        if let Some(wanted) = listen
            && wanted != status.listen
        {
            bail!(
                "CCSW proxy already runs at {}; stop it before changing the address",
                status.listen
            );
        }
        return Ok(status);
    }
    let address = listen.unwrap_or(DEFAULT_LISTEN);
    let socket: SocketAddr = address
        .parse()
        .with_context(|| format!("invalid proxy listen address {address}"))?;
    if !socket.ip().is_loopback() {
        bail!("CCSW proxy only accepts loopback listen addresses");
    }
    update_registry(&proxy_paths, Some(address), |_| ())?;
    let parent = proxy_paths
        .registry
        .parent()
        .context("proxy registry has no parent")?;
    fs::create_dir_all(parent)?;
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&proxy_paths.log)?;
    set_private(&proxy_paths.log)?;
    let stderr = log.try_clone()?;
    let executable = std::env::current_exe().context("cannot resolve ccsw executable")?;
    let mut command = Command::new(executable);
    command
        .args([
            "internal",
            "proxy-serve",
            "--registry",
            proxy_paths.registry.to_string_lossy().as_ref(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr));
    for name in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_BASE_URL",
        "OPENAI_API_KEY",
    ] {
        command.env_remove(name);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: setsid has no memory-safety preconditions and is called in the child.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    command.spawn().context("failed to start CCSW proxy")?;
    for _ in 0..50 {
        std::thread::sleep(Duration::from_millis(50));
        let current = status(paths)?;
        if current.running {
            return Ok(current);
        }
    }
    bail!(
        "CCSW proxy did not become ready; inspect {}",
        proxy_paths.log.display()
    )
}

pub fn status(paths: &AppPaths) -> Result<ProxyStatus> {
    let proxy_paths = ProxyPaths::from_app(paths)?;
    let registry = load_or_default_registry(&proxy_paths, None)?;
    let url = format!("http://{}/health", registry.listen);
    let running = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(350))
        .build()?
        .get(url)
        .send()
        .and_then(|response| response.json::<Value>())
        .is_ok_and(|value| value.get("name").and_then(Value::as_str) == Some("ccsw-proxy"));
    let pid = fs::read_to_string(&proxy_paths.pid)
        .ok()
        .and_then(|value| value.trim().parse().ok());
    Ok(ProxyStatus {
        running,
        listen: registry.listen,
        routes: registry.routes.len(),
        pid,
    })
}

pub fn stop(paths: &AppPaths) -> Result<()> {
    let proxy_paths = ProxyPaths::from_app(paths)?;
    if !status(paths)?.running {
        fs::remove_file(&proxy_paths.pid).ok();
        bail!("CCSW proxy is not running");
    }
    let pid: i32 = fs::read_to_string(&proxy_paths.pid)
        .context("CCSW proxy is not running")?
        .trim()
        .parse()
        .context("proxy PID file is invalid")?;
    #[cfg(unix)]
    {
        // SAFETY: kill is called with a PID read from CCSW's private state file.
        if unsafe { libc::kill(pid, libc::SIGTERM) } == -1 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error).context("failed to stop CCSW proxy");
            }
        }
    }
    for _ in 0..30 {
        std::thread::sleep(Duration::from_millis(50));
        if !status(paths)?.running {
            fs::remove_file(&proxy_paths.pid).ok();
            return Ok(());
        }
    }
    bail!("CCSW proxy did not stop")
}

pub fn install(paths: &AppPaths) -> Result<PathBuf> {
    start(paths, None)?;
    let proxy_paths = ProxyPaths::from_app(paths)?;
    stop(paths)?;
    let executable = std::env::current_exe()?;
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set")?;
    #[cfg(target_os = "macos")]
    {
        let directory = home.join("Library/LaunchAgents");
        fs::create_dir_all(&directory)?;
        let path = directory.join("com.ccsw.proxy.plist");
        let label = "com.ccsw.proxy";
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>{label}</string>
<key>ProgramArguments</key><array><string>{}</string><string>internal</string><string>proxy-serve</string><string>--registry</string><string>{}</string></array>
<key>RunAtLoad</key><true/>
</dict></plist>
"#,
            xml_escape(&executable.to_string_lossy()),
            xml_escape(&proxy_paths.registry.to_string_lossy())
        );
        fs::write(&path, plist)?;
        let domain = format!("gui/{}", unsafe { libc::getuid() });
        let _ = Command::new("launchctl")
            .args(["bootout", &domain, path.to_string_lossy().as_ref()])
            .status();
        let status = Command::new("launchctl")
            .args(["bootstrap", &domain, path.to_string_lossy().as_ref()])
            .status()?;
        if !status.success() {
            bail!("launchctl could not install {}", path.display());
        }
        Ok(path)
    }
    #[cfg(target_os = "linux")]
    {
        let directory = home.join(".config/systemd/user");
        fs::create_dir_all(&directory)?;
        let path = directory.join("ccsw-proxy.service");
        fs::write(
            &path,
            format!(
                "[Unit]\nDescription=CCSW protocol proxy\n\n[Service]\nExecStart={} internal proxy-serve --registry {}\nRestart=on-failure\n\n[Install]\nWantedBy=default.target\n",
                executable.display(),
                proxy_paths.registry.display()
            ),
        )?;
        let status = Command::new("systemctl")
            .args(["--user", "enable", "--now", "ccsw-proxy.service"])
            .status()?;
        if !status.success() {
            bail!("systemctl could not install {}", path.display());
        }
        return Ok(path);
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    bail!("proxy service installation is supported on macOS and Linux");
}

#[allow(clippy::needless_return)]
pub fn uninstall() -> Result<Option<PathBuf>> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set")?;
    #[cfg(target_os = "macos")]
    {
        let path = home.join("Library/LaunchAgents/com.ccsw.proxy.plist");
        if !path.exists() {
            return Ok(None);
        }
        let domain = format!("gui/{}", unsafe { libc::getuid() });
        let _ = Command::new("launchctl")
            .args(["bootout", &domain, path.to_string_lossy().as_ref()])
            .status();
        fs::remove_file(&path)?;
        return Ok(Some(path));
    }
    #[cfg(target_os = "linux")]
    {
        let path = home.join(".config/systemd/user/ccsw-proxy.service");
        if !path.exists() {
            return Ok(None);
        }
        let _ = Command::new("systemctl")
            .args(["--user", "disable", "--now", "ccsw-proxy.service"])
            .status();
        fs::remove_file(&path)?;
        return Ok(Some(path));
    }
    #[allow(unreachable_code)]
    Ok(None)
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn load_or_default_registry(paths: &ProxyPaths, listen: Option<&str>) -> Result<Registry> {
    if paths.registry.exists() {
        return load_registry(paths);
    }
    Ok(Registry {
        listen: listen.unwrap_or(DEFAULT_LISTEN).to_owned(),
        local_token: format!(
            "ccsw-{}{}",
            Uuid::new_v4().simple(),
            Uuid::new_v4().simple()
        ),
        routes: BTreeMap::new(),
    })
}

fn load_registry(paths: &ProxyPaths) -> Result<Registry> {
    serde_json::from_slice(&fs::read(&paths.registry)?)
        .with_context(|| format!("failed to read {}", paths.registry.display()))
}

fn update_registry<T>(
    paths: &ProxyPaths,
    listen: Option<&str>,
    edit: impl FnOnce(&mut Registry) -> T,
) -> Result<T> {
    let parent = paths.registry.parent().context("registry has no parent")?;
    fs::create_dir_all(parent)?;
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&paths.registry_lock)?;
    lock.lock_exclusive()?;
    let mut registry = load_or_default_registry(paths, listen)?;
    if let Some(listen) = listen {
        registry.listen = listen.to_owned();
    }
    let result = edit(&mut registry);
    let mut temp = NamedTempFile::new_in(parent)?;
    temp.write_all(&serde_json::to_vec_pretty(&registry)?)?;
    temp.as_file().sync_all()?;
    set_private(temp.path())?;
    temp.persist(&paths.registry).map_err(|error| error.error)?;
    set_private(&paths.registry)?;
    FileExt::unlock(&lock).ok();
    Ok(result)
}

#[derive(Clone)]
struct ServerState {
    registry: PathBuf,
    client: Client,
}

pub async fn serve(registry_path: PathBuf) -> Result<()> {
    let proxy_paths = ProxyPaths {
        registry: registry_path.clone(),
        registry_lock: registry_path.with_extension("json.lock"),
        daemon_lock: registry_path.with_file_name("proxy.daemon.lock"),
        pid: registry_path.with_file_name("proxy.pid"),
        log: registry_path.with_file_name("proxy.log"),
    };
    let singleton = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&proxy_paths.daemon_lock)?;
    singleton
        .try_lock_exclusive()
        .context("another CCSW proxy is already running")?;
    let registry = load_registry(&proxy_paths)?;
    let address: SocketAddr = registry.listen.parse()?;
    if !address.ip().is_loopback() {
        bail!("CCSW proxy refuses to bind a non-loopback address");
    }
    fs::write(&proxy_paths.pid, std::process::id().to_string())?;
    set_private(&proxy_paths.pid)?;
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .with_context(|| format!("failed to bind {address}"))?;
    let state = ServerState {
        registry: registry_path,
        client: Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()?,
    };
    let app = Router::new()
        .route("/health", get(health))
        .route("/r/{route}/v1/messages", post(messages))
        .route("/r/{route}/v1/messages/count_tokens", post(count_tokens))
        .route("/r/{route}/v1/models", get(models))
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        .with_state(state);
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await;
    fs::remove_file(&proxy_paths.pid).ok();
    result.context("proxy server failed")
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut terminate = signal(SignalKind::terminate()).expect("SIGTERM handler");
        tokio::select! {
            _ = terminate.recv() => {},
            _ = tokio::signal::ctrl_c() => {},
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await.ok();
}

async fn health() -> Json<Value> {
    Json(json!({"name":"ccsw-proxy","status":"ok"}))
}

fn registry_from_path(path: &Path) -> Result<Registry> {
    serde_json::from_slice(&fs::read(path)?)
        .with_context(|| format!("failed to load proxy registry {}", path.display()))
}

fn resolve_profile(
    state: &ServerState,
    route: &str,
    headers: &HeaderMap,
) -> Result<(Profile, Registry)> {
    let registry = registry_from_path(&state.registry)?;
    let expected = format!("Bearer {}", registry.local_token);
    let actual = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    if actual != Some(expected.as_str()) {
        bail!("invalid local proxy credential");
    }
    let target = registry
        .routes
        .get(route)
        .with_context(|| format!("unknown CCSW route {route}"))?;
    let config = config::load(&target.config_path)?;
    let profile = config
        .profiles
        .get(&target.profile_id)
        .cloned()
        .with_context(|| format!("profile '{}' no longer exists", target.profile_id))?;
    if !profile.api_format.is_openai() {
        bail!("profile '{}' is not an OpenAI route", target.profile_id);
    }
    Ok((profile, registry))
}

async fn models(
    State(state): State<ServerState>,
    AxumPath(route): AxumPath<String>,
    headers: HeaderMap,
) -> Response {
    match resolve_profile(&state, &route, &headers) {
        Ok((profile, _)) => Json(json!({
            "data": profile.models.iter().map(|model| json!({"id": strip_1m(&model.id), "object":"model"})).collect::<Vec<_>>()
        }))
        .into_response(),
        Err(error) => anthropic_error(StatusCode::UNAUTHORIZED, error),
    }
}

async fn count_tokens(
    State(state): State<ServerState>,
    AxumPath(route): AxumPath<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(error) = resolve_profile(&state, &route, &headers) {
        return anthropic_error(StatusCode::UNAUTHORIZED, error);
    }
    match estimate_tokens(&body) {
        Ok(tokens) => Json(json!({"input_tokens": tokens})).into_response(),
        Err(error) => anthropic_error(StatusCode::BAD_REQUEST, error),
    }
}

async fn messages(
    State(state): State<ServerState>,
    AxumPath(route): AxumPath<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let (profile, _) = match resolve_profile(&state, &route, &headers) {
        Ok(value) => value,
        Err(error) => return anthropic_error(StatusCode::UNAUTHORIZED, error),
    };
    let stream = body.get("stream").and_then(Value::as_bool).unwrap_or(false);
    let upstream_body = match translate_request(&body, profile.api_format) {
        Ok(body) => body,
        Err(error) => return anthropic_error(StatusCode::BAD_REQUEST, error),
    };
    let endpoint = match completion_endpoint(&profile.base_url, profile.api_format) {
        Ok(endpoint) => endpoint,
        Err(error) => return anthropic_error(StatusCode::BAD_REQUEST, error),
    };
    let mut request = state.client.post(endpoint).json(&upstream_body);
    request = match &profile.credential {
        Credential::Bearer { value } => request.bearer_auth(value),
        Credential::XApiKey { value } => request.header("x-api-key", value),
        Credential::ApiKey { value } => request.header("api-key", value),
        Credential::None => request,
    };
    let response = match request.send().await {
        Ok(response) => response,
        Err(error) => {
            return anthropic_error(
                StatusCode::BAD_GATEWAY,
                anyhow::anyhow!("upstream request failed: {error}"),
            );
        }
    };
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        let text = truncate_utf8(&text, MAX_ERROR_BODY);
        return anthropic_error(
            StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
            anyhow::anyhow!("upstream returned HTTP {status}: {text}"),
        );
    }
    if stream {
        stream_response(response, profile.api_format)
    } else {
        let value = match response.json::<Value>().await {
            Ok(value) => value,
            Err(error) => {
                return anthropic_error(
                    StatusCode::BAD_GATEWAY,
                    anyhow::anyhow!("upstream returned invalid JSON: {error}"),
                );
            }
        };
        match translate_response(&value, profile.api_format) {
            Ok(value) => Json(value).into_response(),
            Err(error) => anthropic_error(StatusCode::BAD_GATEWAY, error),
        }
    }
}

fn anthropic_error(status: StatusCode, error: anyhow::Error) -> Response {
    let error_type = match status.as_u16() {
        400 => "invalid_request_error",
        401 | 403 => "authentication_error",
        429 => "rate_limit_error",
        500..=599 => "api_error",
        _ => "invalid_request_error",
    };
    (
        status,
        Json(json!({
            "type":"error",
            "error":{"type":error_type,"message":format!("{error:#}")}
        })),
    )
        .into_response()
}

fn completion_endpoint(base: &str, format: ApiFormat) -> Result<Url> {
    let wanted = match format {
        ApiFormat::OpenaiChat => "chat/completions",
        ApiFormat::OpenaiResponses => "responses",
        ApiFormat::Anthropic => bail!("Anthropic routes do not use the local proxy"),
    };
    api_endpoint(base, wanted)
}

pub fn models_endpoint(base: &str, format: ApiFormat) -> Result<Url> {
    if matches!(format, ApiFormat::Anthropic) {
        return api_endpoint(base, "models");
    }
    api_endpoint(base, "models")
}

fn api_endpoint(base: &str, wanted: &str) -> Result<Url> {
    let mut url = Url::parse(base).context("API URL is invalid")?;
    if !matches!(url.scheme(), "http" | "https") {
        bail!("API URL must use http or https");
    }
    let path = url.path().trim_end_matches('/');
    let known = ["/chat/completions", "/responses", "/messages", "/models"];
    let root = known
        .iter()
        .find_map(|suffix| path.strip_suffix(suffix))
        .unwrap_or(path);
    let next = if root.ends_with("/v1") {
        format!("{root}/{wanted}")
    } else {
        format!("{root}/v1/{wanted}")
    };
    url.set_path(&next);
    Ok(url)
}

fn translate_request(input: &Value, format: ApiFormat) -> Result<Value> {
    let object = input
        .as_object()
        .context("request body must be an object")?;
    let model = object
        .get("model")
        .and_then(Value::as_str)
        .context("model is required")?;
    let messages = object
        .get("messages")
        .and_then(Value::as_array)
        .context("messages must be an array")?;
    let system = system_text(object.get("system"))?;
    let mut result = Map::new();
    result.insert("model".into(), Value::String(strip_1m(model)));
    match format {
        ApiFormat::OpenaiChat => {
            let mut converted = Vec::new();
            if !system.is_empty() {
                converted.push(json!({"role":"system","content":system}));
            }
            for message in messages {
                converted.extend(chat_messages(message)?);
            }
            result.insert("messages".into(), Value::Array(converted));
            copy_number(object, &mut result, "max_tokens", "max_tokens");
        }
        ApiFormat::OpenaiResponses => {
            if !system.is_empty() {
                result.insert("instructions".into(), Value::String(system));
            }
            let mut converted = Vec::new();
            for message in messages {
                converted.extend(response_items(message)?);
            }
            result.insert("input".into(), Value::Array(converted));
            result.insert("store".into(), Value::Bool(false));
            copy_number(object, &mut result, "max_tokens", "max_output_tokens");
            if object.get("thinking").is_some() {
                result.insert(
                    "reasoning".into(),
                    json!({"effort":"high","summary":"auto"}),
                );
            }
        }
        ApiFormat::Anthropic => bail!("Anthropic request does not need translation"),
    }
    for key in ["temperature", "top_p"] {
        if let Some(value) = object.get(key) {
            result.insert(key.into(), value.clone());
        }
    }
    if let Some(stop) = object.get("stop_sequences") {
        result.insert("stop".into(), stop.clone());
    }
    if let Some(stream) = object.get("stream") {
        result.insert("stream".into(), stream.clone());
    }
    if let Some(tools) = object.get("tools").and_then(Value::as_array) {
        result.insert(
            "tools".into(),
            Value::Array(
                tools
                    .iter()
                    .map(|tool| convert_tool(tool, format))
                    .collect::<Result<Vec<_>>>()?,
            ),
        );
    }
    if let Some(choice) = object.get("tool_choice") {
        result.insert("tool_choice".into(), convert_tool_choice(choice, format)?);
    }
    Ok(Value::Object(result))
}

fn copy_number(source: &Map<String, Value>, target: &mut Map<String, Value>, from: &str, to: &str) {
    if let Some(value) = source.get(from).filter(|value| value.is_number()) {
        target.insert(to.into(), value.clone());
    }
}

fn system_text(value: Option<&Value>) -> Result<String> {
    match value {
        None => Ok(String::new()),
        Some(Value::String(text)) => Ok(text.clone()),
        Some(Value::Array(blocks)) => {
            let mut text = Vec::new();
            for block in blocks {
                if block.get("type").and_then(Value::as_str) != Some("text") {
                    bail!("unsupported system content block");
                }
                text.push(
                    block
                        .get("text")
                        .and_then(Value::as_str)
                        .context("system text block has no text")?,
                );
            }
            Ok(text.join("\n"))
        }
        Some(_) => bail!("system must be a string or text block array"),
    }
}

fn content_blocks(message: &Value) -> Result<Vec<Value>> {
    match message.get("content") {
        Some(Value::String(text)) => Ok(vec![json!({"type":"text","text":text})]),
        Some(Value::Array(blocks)) => Ok(blocks.clone()),
        _ => bail!("message content must be a string or array"),
    }
}

fn chat_messages(message: &Value) -> Result<Vec<Value>> {
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .context("message role is required")?;
    let blocks = content_blocks(message)?;
    if role == "system" || role == "developer" {
        let text = text_only_blocks(&blocks, role)?;
        return Ok(vec![json!({"role":role,"content":text})]);
    }
    if role == "assistant" {
        let mut text = String::new();
        let mut calls = Vec::new();
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => text.push_str(block.get("text").and_then(Value::as_str).unwrap_or("")),
                Some("tool_use") => calls.push(json!({
                    "id": block.get("id").and_then(Value::as_str).context("tool_use has no id")?,
                    "type":"function",
                    "function":{
                        "name":block.get("name").and_then(Value::as_str).context("tool_use has no name")?,
                        "arguments":serde_json::to_string(block.get("input").unwrap_or(&json!({})))?
                    }
                })),
                Some("thinking" | "redacted_thinking") => {}
                Some(other) => bail!("unsupported assistant content block: {other}"),
                None => bail!("content block has no type"),
            }
        }
        let mut row = Map::from_iter([
            ("role".into(), Value::String("assistant".into())),
            (
                "content".into(),
                if text.is_empty() {
                    Value::Null
                } else {
                    Value::String(text)
                },
            ),
        ]);
        if !calls.is_empty() {
            row.insert("tool_calls".into(), Value::Array(calls));
        }
        return Ok(vec![Value::Object(row)]);
    }
    if role != "user" {
        bail!("unsupported message role: {role}");
    }
    let mut result = Vec::new();
    let mut content = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => content.push(json!({"type":"text","text":block.get("text").and_then(Value::as_str).unwrap_or("")})),
            Some("image") => content.push(chat_image(&block)?),
            Some("tool_result") => result.push(json!({
                "role":"tool",
                "tool_call_id":block.get("tool_use_id").and_then(Value::as_str).context("tool_result has no tool_use_id")?,
                "content":flatten_tool_result(block.get("content"))?
            })),
            Some(other) => bail!("unsupported user content block: {other}"),
            None => bail!("content block has no type"),
        }
    }
    if !content.is_empty() {
        result.push(json!({"role":"user","content":content}));
    }
    Ok(result)
}

fn chat_image(block: &Value) -> Result<Value> {
    let source = block.get("source").context("image has no source")?;
    let url = match source.get("type").and_then(Value::as_str) {
        Some("base64") => format!(
            "data:{};base64,{}",
            source
                .get("media_type")
                .and_then(Value::as_str)
                .context("image has no media_type")?,
            source
                .get("data")
                .and_then(Value::as_str)
                .context("image has no data")?
        ),
        Some("url") => source
            .get("url")
            .and_then(Value::as_str)
            .context("image has no URL")?
            .to_owned(),
        Some(other) => bail!("unsupported image source: {other}"),
        None => bail!("image source has no type"),
    };
    Ok(json!({"type":"image_url","image_url":{"url":url}}))
}

fn response_items(message: &Value) -> Result<Vec<Value>> {
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .context("message role is required")?;
    let blocks = content_blocks(message)?;
    let mut result = Vec::new();
    let mut content = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => content.push(json!({
                "type":if role == "assistant" {"output_text"} else {"input_text"},
                "text":block.get("text").and_then(Value::as_str).unwrap_or("")
            })),
            Some("image") if role == "user" => {
                let image = chat_image(&block)?;
                content.push(json!({"type":"input_image","image_url":image["image_url"]["url"]}));
            }
            Some("tool_use") if role == "assistant" => result.push(json!({
                "type":"function_call",
                "call_id":block.get("id").and_then(Value::as_str).context("tool_use has no id")?,
                "name":block.get("name").and_then(Value::as_str).context("tool_use has no name")?,
                "arguments":serde_json::to_string(block.get("input").unwrap_or(&json!({})))?
            })),
            Some("tool_result") if role == "user" => result.push(json!({
                "type":"function_call_output",
                "call_id":block.get("tool_use_id").and_then(Value::as_str).context("tool_result has no tool_use_id")?,
                "output":flatten_tool_result(block.get("content"))?
            })),
            Some("thinking" | "redacted_thinking") if role == "assistant" => {}
            Some(other) => bail!("unsupported {role} content block: {other}"),
            None => bail!("content block has no type"),
        }
    }
    if !content.is_empty() {
        result.insert(0, json!({"role":role,"content":content}));
    }
    Ok(result)
}

fn text_only_blocks(blocks: &[Value], role: &str) -> Result<String> {
    let mut text = Vec::new();
    for block in blocks {
        if block.get("type").and_then(Value::as_str) != Some("text") {
            bail!("unsupported {role} content block");
        }
        text.push(
            block
                .get("text")
                .and_then(Value::as_str)
                .context("text content block has no text")?,
        );
    }
    Ok(text.join("\n"))
}

fn flatten_tool_result(value: Option<&Value>) -> Result<String> {
    match value {
        None => Ok(String::new()),
        Some(Value::String(text)) => Ok(text.clone()),
        Some(Value::Array(blocks)) => {
            let mut result = Vec::new();
            for block in blocks {
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => result.push(
                        block
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned(),
                    ),
                    Some(other) => bail!("unsupported tool_result content block: {other}"),
                    None => bail!("tool_result content block has no type"),
                }
            }
            Ok(result.join("\n"))
        }
        Some(other) => Ok(other.to_string()),
    }
}

fn convert_tool(tool: &Value, format: ApiFormat) -> Result<Value> {
    if tool.get("type").is_some() && tool.get("name").is_none() {
        bail!("server-side Anthropic tools cannot be forwarded");
    }
    let name = tool
        .get("name")
        .and_then(Value::as_str)
        .context("tool has no name")?;
    let description = tool
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("");
    let schema = tool
        .get("input_schema")
        .cloned()
        .unwrap_or_else(|| json!({"type":"object"}));
    Ok(match format {
        ApiFormat::OpenaiChat => {
            json!({"type":"function","function":{"name":name,"description":description,"parameters":schema}})
        }
        ApiFormat::OpenaiResponses => {
            json!({"type":"function","name":name,"description":description,"parameters":schema})
        }
        ApiFormat::Anthropic => unreachable!(),
    })
}

fn convert_tool_choice(choice: &Value, format: ApiFormat) -> Result<Value> {
    let kind = choice.get("type").and_then(Value::as_str).unwrap_or("auto");
    Ok(match kind {
        "auto" => Value::String("auto".into()),
        "none" => Value::String("none".into()),
        "any" => Value::String("required".into()),
        "tool" => {
            let name = choice
                .get("name")
                .and_then(Value::as_str)
                .context("tool choice has no name")?;
            if matches!(format, ApiFormat::OpenaiChat) {
                json!({"type":"function","function":{"name":name}})
            } else {
                json!({"type":"function","name":name})
            }
        }
        other => bail!("unsupported tool choice: {other}"),
    })
}

fn translate_response(value: &Value, format: ApiFormat) -> Result<Value> {
    match format {
        ApiFormat::OpenaiChat => chat_response(value),
        ApiFormat::OpenaiResponses => responses_response(value),
        ApiFormat::Anthropic => bail!("Anthropic response does not need translation"),
    }
}

fn chat_response(value: &Value) -> Result<Value> {
    let choice = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|rows| rows.first())
        .context("chat response has no choice")?;
    let message = choice
        .get("message")
        .context("chat choice has no message")?;
    let mut content = Vec::new();
    if let Some(reasoning) = message
        .get("reasoning_content")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        content.push(json!({"type":"thinking","thinking":reasoning,"signature":"ccsw-openai"}));
    }
    if let Some(text) = message
        .get("content")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        content.push(json!({"type":"text","text":text}));
    }
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            let function = call.get("function").context("tool call has no function")?;
            let args = function
                .get("arguments")
                .and_then(Value::as_str)
                .unwrap_or("{}");
            content.push(json!({
                "type":"tool_use",
                "id":call.get("id").and_then(Value::as_str).unwrap_or("call_ccsw"),
                "name":function.get("name").and_then(Value::as_str).context("tool call has no name")?,
                "input":serde_json::from_str::<Value>(args).unwrap_or_else(|_| json!({"_raw":args}))
            }));
        }
    }
    Ok(anthropic_message(
        value.get("id").and_then(Value::as_str),
        value.get("model").and_then(Value::as_str),
        content,
        stop_reason(choice.get("finish_reason").and_then(Value::as_str)),
        value.get("usage"),
    ))
}

fn responses_response(value: &Value) -> Result<Value> {
    let mut content = Vec::new();
    for item in value
        .get("output")
        .and_then(Value::as_array)
        .context("Responses output is missing")?
    {
        match item.get("type").and_then(Value::as_str) {
            Some("message") => {
                for part in item
                    .get("content")
                    .and_then(Value::as_array)
                    .unwrap_or(&Vec::new())
                {
                    match part.get("type").and_then(Value::as_str) {
                        Some("output_text") => content.push(json!({"type":"text","text":part.get("text").and_then(Value::as_str).unwrap_or("")})),
                        Some("refusal") => content.push(json!({"type":"text","text":part.get("refusal").and_then(Value::as_str).unwrap_or("")})),
                        Some(other) => bail!("unsupported Responses content: {other}"),
                        None => {}
                    }
                }
            }
            Some("function_call") => {
                let args = item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("{}");
                content.push(json!({
                    "type":"tool_use",
                    "id":item.get("call_id").or_else(|| item.get("id")).and_then(Value::as_str).unwrap_or("call_ccsw"),
                    "name":item.get("name").and_then(Value::as_str).context("function call has no name")?,
                    "input":serde_json::from_str::<Value>(args).unwrap_or_else(|_| json!({"_raw":args}))
                }));
            }
            Some("reasoning") => {
                if let Some(summary) = item.get("summary").and_then(Value::as_array) {
                    let text = summary
                        .iter()
                        .filter_map(|row| row.get("text").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join("\n");
                    if !text.is_empty() {
                        content.push(
                            json!({"type":"thinking","thinking":text,"signature":"ccsw-openai"}),
                        );
                    }
                }
            }
            Some(other) => bail!("unsupported Responses output item: {other}"),
            None => {}
        }
    }
    let has_tool = content
        .iter()
        .any(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"));
    let incomplete = value.get("status").and_then(Value::as_str) == Some("incomplete");
    Ok(anthropic_message(
        value.get("id").and_then(Value::as_str),
        value.get("model").and_then(Value::as_str),
        content,
        if has_tool {
            "tool_use"
        } else if incomplete {
            "max_tokens"
        } else {
            "end_turn"
        },
        value.get("usage"),
    ))
}

fn anthropic_message(
    id: Option<&str>,
    model: Option<&str>,
    content: Vec<Value>,
    stop: &str,
    usage: Option<&Value>,
) -> Value {
    let input_tokens = usage
        .and_then(|u| u.get("prompt_tokens").or_else(|| u.get("input_tokens")))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output_tokens = usage
        .and_then(|u| {
            u.get("completion_tokens")
                .or_else(|| u.get("output_tokens"))
        })
        .and_then(Value::as_u64)
        .unwrap_or(0);
    json!({
        "id":id.unwrap_or("msg_ccsw"),"type":"message","role":"assistant",
        "model":model.unwrap_or("openai-compatible"),"content":content,
        "stop_reason":stop,"stop_sequence":null,
        "usage":{"input_tokens":input_tokens,"output_tokens":output_tokens}
    })
}

fn stop_reason(reason: Option<&str>) -> &'static str {
    match reason {
        Some("length") => "max_tokens",
        Some("tool_calls" | "function_call") => "tool_use",
        Some("content_filter") => "refusal",
        _ => "end_turn",
    }
}

#[derive(Default)]
struct StreamState {
    started: bool,
    text_block: Option<usize>,
    thinking_block: Option<usize>,
    tools: HashMap<usize, usize>,
    next_block: usize,
    finish_reason: Option<String>,
    input_tokens: u64,
    output_tokens: u64,
    id: String,
    model: String,
}

fn stream_response(response: reqwest::Response, format: ApiFormat) -> Response {
    let mut upstream = response.bytes_stream();
    let output = async_stream::stream! {
        let mut pending = String::new();
        let mut state = StreamState::default();
        while let Some(chunk) = upstream.next().await {
            match chunk {
                Ok(bytes) => {
                    pending.push_str(&String::from_utf8_lossy(&bytes));
                    while let Some(index) = pending.find("\n\n").or_else(|| pending.find("\r\n\r\n")) {
                        let separator = if pending[index..].starts_with("\r\n") { 4 } else { 2 };
                        let frame = pending[..index].to_owned();
                        pending.drain(..index + separator);
                        let data = frame.lines().filter_map(|line| line.strip_prefix("data:").map(str::trim)).collect::<Vec<_>>().join("\n");
                        if data.is_empty() { continue; }
                        let events = if data == "[DONE]" {
                            finalize_stream(&mut state)
                        } else {
                            match serde_json::from_str::<Value>(&data) {
                                Ok(value) => translate_stream_event(&value, format, &mut state),
                                Err(error) => vec![error_sse(&format!("invalid upstream SSE JSON: {error}"))],
                            }
                        };
                        for event in events { yield Ok::<Bytes, std::convert::Infallible>(Bytes::from(event)); }
                    }
                }
                Err(error) => {
                    yield Ok(Bytes::from(error_sse(&format!("upstream stream failed: {error}"))));
                    return;
                }
            }
        }
        for event in finalize_stream(&mut state) { yield Ok(Bytes::from(event)); }
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(output))
        .expect("valid streaming response")
}

fn translate_stream_event(
    value: &Value,
    format: ApiFormat,
    state: &mut StreamState,
) -> Vec<String> {
    match format {
        ApiFormat::OpenaiChat => chat_stream_event(value, state),
        ApiFormat::OpenaiResponses => responses_stream_event(value, state),
        ApiFormat::Anthropic => vec![error_sse("invalid proxy stream format")],
    }
}

fn ensure_start(state: &mut StreamState, id: Option<&str>, model: Option<&str>) -> Vec<String> {
    if state.started {
        return Vec::new();
    }
    state.started = true;
    state.id = id.unwrap_or("msg_ccsw_stream").to_owned();
    state.model = model.unwrap_or("openai-compatible").to_owned();
    vec![sse(
        "message_start",
        json!({"type":"message_start","message":{
            "id":state.id,"type":"message","role":"assistant","model":state.model,
            "content":[],"stop_reason":null,"stop_sequence":null,
            "usage":{"input_tokens":0,"output_tokens":0}
        }}),
    )]
}

fn chat_stream_event(value: &Value, state: &mut StreamState) -> Vec<String> {
    let mut out = ensure_start(
        state,
        value.get("id").and_then(Value::as_str),
        value.get("model").and_then(Value::as_str),
    );
    if let Some(usage) = value.get("usage") {
        state.input_tokens = usage
            .get("prompt_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(state.input_tokens);
        state.output_tokens = usage
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(state.output_tokens);
    }
    let Some(choice) = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|rows| rows.first())
    else {
        return out;
    };
    let delta = choice.get("delta").unwrap_or(&Value::Null);
    if let Some(reasoning) = delta
        .get("reasoning_content")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        let index = ensure_block(state, &mut out, BlockKind::Thinking, None, None);
        out.push(sse("content_block_delta", json!({"type":"content_block_delta","index":index,"delta":{"type":"thinking_delta","thinking":reasoning}})));
    }
    if let Some(text) = delta
        .get("content")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        let index = ensure_block(state, &mut out, BlockKind::Text, None, None);
        out.push(sse("content_block_delta", json!({"type":"content_block_delta","index":index,"delta":{"type":"text_delta","text":text}})));
    }
    if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            let tool_index = call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            let function = call.get("function").unwrap_or(&Value::Null);
            let index = if let Some(index) = state.tools.get(&tool_index) {
                *index
            } else {
                let index = ensure_block(
                    state,
                    &mut out,
                    BlockKind::Tool(tool_index),
                    call.get("id").and_then(Value::as_str),
                    function.get("name").and_then(Value::as_str),
                );
                state.tools.insert(tool_index, index);
                index
            };
            if let Some(arguments) = function
                .get("arguments")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
            {
                out.push(sse("content_block_delta", json!({"type":"content_block_delta","index":index,"delta":{"type":"input_json_delta","partial_json":arguments}})));
            }
        }
    }
    if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
        state.finish_reason = Some(stop_reason(Some(reason)).into());
    }
    out
}

enum BlockKind {
    Text,
    Thinking,
    Tool(usize),
}

fn ensure_block(
    state: &mut StreamState,
    out: &mut Vec<String>,
    kind: BlockKind,
    id: Option<&str>,
    name: Option<&str>,
) -> usize {
    match kind {
        BlockKind::Text if state.text_block.is_some() => return state.text_block.unwrap_or(0),
        BlockKind::Thinking if state.thinking_block.is_some() => {
            return state.thinking_block.unwrap_or(0);
        }
        BlockKind::Tool(tool) if state.tools.contains_key(&tool) => return state.tools[&tool],
        _ => {}
    }
    let index = state.next_block;
    state.next_block += 1;
    let block = match kind {
        BlockKind::Text => {
            state.text_block = Some(index);
            json!({"type":"text","text":""})
        }
        BlockKind::Thinking => {
            state.thinking_block = Some(index);
            json!({"type":"thinking","thinking":"","signature":"ccsw-openai"})
        }
        BlockKind::Tool(tool) => {
            state.tools.insert(tool, index);
            json!({"type":"tool_use","id":id.unwrap_or("call_ccsw"),"name":name.unwrap_or("tool"),"input":{}})
        }
    };
    out.push(sse(
        "content_block_start",
        json!({"type":"content_block_start","index":index,"content_block":block}),
    ));
    index
}

fn responses_stream_event(value: &Value, state: &mut StreamState) -> Vec<String> {
    let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
    let response = value.get("response").unwrap_or(value);
    let mut out = ensure_start(
        state,
        response.get("id").and_then(Value::as_str),
        response.get("model").and_then(Value::as_str),
    );
    match kind {
        "response.output_text.delta" => {
            let index = ensure_block(state, &mut out, BlockKind::Text, None, None);
            let text = value.get("delta").and_then(Value::as_str).unwrap_or("");
            out.push(sse("content_block_delta", json!({"type":"content_block_delta","index":index,"delta":{"type":"text_delta","text":text}})));
        }
        "response.reasoning_summary_text.delta" => {
            let index = ensure_block(state, &mut out, BlockKind::Thinking, None, None);
            let text = value.get("delta").and_then(Value::as_str).unwrap_or("");
            out.push(sse("content_block_delta", json!({"type":"content_block_delta","index":index,"delta":{"type":"thinking_delta","thinking":text}})));
        }
        "response.output_item.added" => {
            if let Some(item) = value.get("item")
                && item.get("type").and_then(Value::as_str) == Some("function_call")
            {
                let key = value
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                ensure_block(
                    state,
                    &mut out,
                    BlockKind::Tool(key),
                    item.get("call_id")
                        .or_else(|| item.get("id"))
                        .and_then(Value::as_str),
                    item.get("name").and_then(Value::as_str),
                );
            }
        }
        "response.function_call_arguments.delta" => {
            let key = value
                .get("output_index")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize;
            let index = if let Some(index) = state.tools.get(&key) {
                *index
            } else {
                ensure_block(
                    state,
                    &mut out,
                    BlockKind::Tool(key),
                    value.get("item_id").and_then(Value::as_str),
                    Some("tool"),
                )
            };
            let args = value.get("delta").and_then(Value::as_str).unwrap_or("");
            out.push(sse("content_block_delta", json!({"type":"content_block_delta","index":index,"delta":{"type":"input_json_delta","partial_json":args}})));
        }
        "response.completed" | "response.incomplete" => {
            let usage = response.get("usage").unwrap_or(&Value::Null);
            state.input_tokens = usage
                .get("input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            state.output_tokens = usage
                .get("output_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            state.finish_reason = Some(
                if !state.tools.is_empty() {
                    "tool_use"
                } else if kind == "response.incomplete" {
                    "max_tokens"
                } else {
                    "end_turn"
                }
                .into(),
            );
        }
        "response.failed" => out.push(error_sse(
            response
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("upstream response failed"),
        )),
        _ => {}
    }
    out
}

fn finalize_stream(state: &mut StreamState) -> Vec<String> {
    if !state.started {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut blocks = state
        .text_block
        .into_iter()
        .chain(state.thinking_block)
        .chain(state.tools.values().copied())
        .collect::<Vec<_>>();
    blocks.sort_unstable();
    blocks.dedup();
    for index in blocks {
        out.push(sse(
            "content_block_stop",
            json!({"type":"content_block_stop","index":index}),
        ));
    }
    out.push(sse("message_delta", json!({"type":"message_delta","delta":{"stop_reason":state.finish_reason.as_deref().unwrap_or("end_turn"),"stop_sequence":null},"usage":{"input_tokens":state.input_tokens,"output_tokens":state.output_tokens}})));
    out.push(sse("message_stop", json!({"type":"message_stop"})));
    state.started = false;
    out
}

fn sse(event: &str, value: Value) -> String {
    format!("event: {event}\ndata: {value}\n\n")
}

fn error_sse(message: &str) -> String {
    sse(
        "error",
        json!({"type":"error","error":{"type":"api_error","message":message}}),
    )
}

fn estimate_tokens(value: &Value) -> Result<usize> {
    let encoded = serde_json::to_string(value)?;
    let bpe = tiktoken_rs::cl100k_base().context("failed to initialize token estimator")?;
    Ok(bpe.encode_with_special_tokens(&encoded).len())
}

fn strip_1m(model: &str) -> String {
    model.strip_suffix("[1m]").unwrap_or(model).to_owned()
}

fn truncate_utf8(value: &str, limit: usize) -> &str {
    if value.len() <= limit {
        return value;
    }
    let mut end = limit;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, RoleModels};
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
    };

    #[test]
    fn normalizes_root_v1_and_complete_urls() {
        assert_eq!(
            completion_endpoint("https://api.example", ApiFormat::OpenaiChat)
                .unwrap()
                .as_str(),
            "https://api.example/v1/chat/completions"
        );
        assert_eq!(
            completion_endpoint("https://api.example/v1", ApiFormat::OpenaiResponses)
                .unwrap()
                .as_str(),
            "https://api.example/v1/responses"
        );
        assert_eq!(
            completion_endpoint(
                "https://api.example/v1/chat/completions?api-version=1",
                ApiFormat::OpenaiChat
            )
            .unwrap()
            .as_str(),
            "https://api.example/v1/chat/completions?api-version=1"
        );
    }

    #[test]
    fn translates_chat_tools_and_strips_1m() {
        let request = json!({
            "model":"gpt-test[1m]","max_tokens":100,
            "system":[{"type":"text","text":"code"}],
            "messages":[{"role":"assistant","content":[{"type":"tool_use","id":"call-1","name":"read","input":{"path":"a"}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"call-1","content":"ok"}]}],
            "tools":[{"name":"read","description":"read","input_schema":{"type":"object"}}]
        });
        let value = translate_request(&request, ApiFormat::OpenaiChat).unwrap();
        assert_eq!(value["model"], "gpt-test");
        assert_eq!(
            value["messages"][1]["tool_calls"][0]["function"]["name"],
            "read"
        );
        assert_eq!(value["messages"][2]["role"], "tool");
    }

    #[test]
    fn rejects_unknown_content_blocks() {
        let request =
            json!({"model":"x","messages":[{"role":"user","content":[{"type":"document"}]}]});
        assert!(translate_request(&request, ApiFormat::OpenaiChat).is_err());
    }

    #[test]
    fn translates_mid_conversation_system_messages() {
        let request = json!({
            "model":"x",
            "messages":[
                {"role":"system","content":[{"type":"text","text":"title the session"}]},
                {"role":"user","content":"hello"}
            ]
        });
        let chat = translate_request(&request, ApiFormat::OpenaiChat).unwrap();
        assert_eq!(chat["messages"][0]["role"], "system");
        assert_eq!(chat["messages"][0]["content"], "title the session");
        let responses = translate_request(&request, ApiFormat::OpenaiResponses).unwrap();
        assert_eq!(responses["input"][0]["role"], "system");
        assert_eq!(responses["input"][0]["content"][0]["type"], "input_text");
    }

    #[test]
    fn translates_non_streaming_chat_tool_response() {
        let value = chat_response(&json!({
            "id":"chat-1","model":"gpt","choices":[{"finish_reason":"tool_calls","message":{"content":null,"tool_calls":[{"id":"call-1","function":{"name":"read","arguments":"{\"path\":\"a\"}"}}]}}],
            "usage":{"prompt_tokens":10,"completion_tokens":3}
        })).unwrap();
        assert_eq!(value["stop_reason"], "tool_use");
        assert_eq!(value["content"][0]["input"]["path"], "a");
    }

    #[test]
    fn translates_responses_request_and_function_output() {
        let request = json!({
            "model":"gpt-response[1m]","max_tokens":50,"system":"code",
            "messages":[
                {"role":"assistant","content":[{"type":"tool_use","id":"call-7","name":"shell","input":{"cmd":"pwd"}}]},
                {"role":"user","content":[{"type":"tool_result","tool_use_id":"call-7","content":"/tmp"}]}
            ]
        });
        let value = translate_request(&request, ApiFormat::OpenaiResponses).unwrap();
        assert_eq!(value["model"], "gpt-response");
        assert_eq!(value["store"], false);
        assert_eq!(value["input"][0]["type"], "function_call");
        assert_eq!(value["input"][1]["type"], "function_call_output");
    }

    #[test]
    fn translates_non_streaming_responses_output() {
        let value = responses_response(&json!({
            "id":"resp-1","model":"gpt","status":"completed",
            "output":[
                {"type":"message","content":[{"type":"output_text","text":"done"}]},
                {"type":"function_call","call_id":"call-2","name":"read","arguments":"{\"path\":\"b\"}"}
            ],
            "usage":{"input_tokens":9,"output_tokens":4}
        })).unwrap();
        assert_eq!(value["stop_reason"], "tool_use");
        assert_eq!(value["content"][0]["text"], "done");
        assert_eq!(value["content"][1]["input"]["path"], "b");
    }

    #[test]
    fn chat_stream_accumulates_fragmented_tool_arguments() {
        let mut state = StreamState::default();
        let first = chat_stream_event(
            &json!({"id":"chat","model":"gpt","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call-1","function":{"name":"read","arguments":"{\"pa"}}]}}]}),
            &mut state,
        );
        let second = chat_stream_event(
            &json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"th\":\"a\"}"}}]},"finish_reason":"tool_calls"}]}),
            &mut state,
        );
        let completed = finalize_stream(&mut state);
        let all = first
            .into_iter()
            .chain(second)
            .chain(completed)
            .collect::<String>();
        assert!(all.contains("input_json_delta"));
        assert!(all.contains("{\\\"pa"));
        assert!(all.contains("th\\\":\\\"a\\\"}"));
        assert!(all.contains("tool_use"));
        assert!(all.contains("message_stop"));
    }

    #[tokio::test]
    async fn proxy_forwards_anthropic_messages_to_chat_completions() {
        let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
        let upstream_address = upstream.local_addr().unwrap();
        let (request_tx, request_rx) = mpsc::channel();
        let upstream_task = thread::spawn(move || {
            let (mut stream, _) = upstream.accept().unwrap();
            let mut request = vec![0_u8; 65536];
            let size = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..size]).into_owned();
            request_tx.send(request).unwrap();
            let body = r#"{"id":"chat-1","model":"gpt-test","choices":[{"finish_reason":"stop","message":{"content":"hello"}}],"usage":{"prompt_tokens":4,"completion_tokens":1}}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });

        let temp = tempfile::tempdir().unwrap();
        let app_paths = AppPaths {
            config: temp.path().join("config.toml"),
            state: temp.path().join("state/state.json"),
            cache: temp.path().join("cache/models.json"),
            runtime_dir: temp.path().join("cache/runtime"),
        };
        let profile = Profile {
            name: "OpenAI".into(),
            base_url: format!("http://{upstream_address}"),
            api_format: ApiFormat::OpenaiChat,
            credential: Credential::Bearer {
                value: "upstream-secret".into(),
            },
            default_model: "gpt-test[1m]".into(),
            aliases: RoleModels::default(),
            subagent_model: None,
            fallback_models: vec![],
            enabled_models: vec![],
            models: vec![],
        };
        config::update(&app_paths.config, |config: &mut Config| {
            config.profiles.insert("openai".into(), profile);
            Ok(())
        })
        .unwrap();

        let proxy_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy_address = proxy_listener.local_addr().unwrap();
        drop(proxy_listener);
        let proxy_paths = ProxyPaths::from_app(&app_paths).unwrap();
        update_registry(&proxy_paths, Some(&proxy_address.to_string()), |registry| {
            registry.routes.insert(
                "route-test".into(),
                RouteTarget {
                    config_path: app_paths.config.clone(),
                    profile_id: "openai".into(),
                },
            );
        })
        .unwrap();
        let registry = load_registry(&proxy_paths).unwrap();
        let registry_path = proxy_paths.registry.clone();
        let server = tokio::spawn(async move { serve(registry_path).await });
        let client = Client::new();
        let url = format!("http://{proxy_address}/r/route-test/v1/messages");
        let mut response = None;
        for _ in 0..30 {
            if let Ok(result) = client
                .post(&url)
                .bearer_auth(&registry.local_token)
                .json(&json!({
                    "model":"gpt-test[1m]",
                    "max_tokens":20,
                    "messages":[{"role":"user","content":"hello"}]
                }))
                .send()
                .await
            {
                response = Some(result);
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let value: Value = response.unwrap().json().await.unwrap();
        assert_eq!(value["type"], "message");
        assert_eq!(value["content"][0]["text"], "hello");
        let upstream_request = request_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(upstream_request.starts_with("POST /v1/chat/completions "));
        assert!(upstream_request.contains("authorization: Bearer upstream-secret"));
        assert!(upstream_request.contains("\"model\":\"gpt-test\""));
        assert!(!upstream_request.contains(&registry.local_token));
        server.abort();
        upstream_task.join().unwrap();
    }
}
