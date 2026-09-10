//! Responses ingress. Native Responses are forwarded; other protocols use Messages as an intermediate.
use super::*;
use std::collections::BTreeSet;

fn error(status: StatusCode, message: impl std::fmt::Display) -> Response {
    (
        status,
        Json(json!({"error":{"type":"api_error","message":message.to_string()}})),
    )
        .into_response()
}
pub(super) async fn handle(
    State(state): State<ServerState>,
    AxumPath(route): AxumPath<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    serve(state, route, headers, body, false).await
}
pub(super) async fn compact(
    State(state): State<ServerState>,
    AxumPath(route): AxumPath<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    serve(state, route, headers, body, true).await
}
async fn serve(
    state: ServerState,
    route: String,
    headers: HeaderMap,
    mut body: Value,
    compact: bool,
) -> Response {
    let (target, _) = match authenticated_target(&state, &route, &headers) {
        Ok(v) => v,
        Err(_) => {
            return error(
                StatusCode::UNAUTHORIZED,
                "Invalid local proxy authentication",
            );
        }
    };
    if !target.codex {
        return error(StatusCode::BAD_REQUEST, "This is not a Codex route");
    }
    if body
        .get("model")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        return error(StatusCode::BAD_REQUEST, "A configured model is required");
    }
    let (profile, model) = match resolve_profile(&target, body["model"].as_str()) {
        Ok(v) => v,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    if !body.is_object() {
        return error(StatusCode::BAD_REQUEST, "Request must be an object");
    }
    body["model"] = json!(strip_1m(&model));
    let native = profile.api_format == ApiFormat::OpenaiResponses;
    if compact && !native {
        return error(
            StatusCode::BAD_REQUEST,
            "This provider requires client-side compaction; remote Responses compaction is unavailable",
        );
    }
    let streaming = !compact && body["stream"].as_bool().unwrap_or(false);
    let namespaces = if native {
        HashMap::new()
    } else {
        match flatten_namespaces(&mut body) {
            Ok(v) => v,
            Err(e) => return error(StatusCode::BAD_REQUEST, e),
        }
    };
    let custom = custom_tools(&body);
    let upstream_body = if native {
        let mut limit = json!({});
        if let Some(tokens) = body.get("max_output_tokens") {
            limit["max_tokens"] = tokens.clone();
        }
        if let Err(e) = apply_model_limits(&profile, &model, &mut limit) {
            return error(StatusCode::BAD_REQUEST, e);
        }
        if let Some(tokens) = limit.get("max_tokens") {
            body["max_output_tokens"] = tokens.clone();
        }
        body.clone()
    } else {
        let mut messages = match to_messages(&body) {
            Ok(v) => v,
            Err(e) => return error(StatusCode::BAD_REQUEST, e),
        };
        if body.get("max_output_tokens").is_none()
            && profile
                .models
                .iter()
                .any(|m| strip_1m(&m.id) == strip_1m(&model) && m.max_output_tokens.is_some())
        {
            messages.as_object_mut().unwrap().remove("max_tokens");
        }
        if let Err(e) = apply_model_limits(&profile, &model, &mut messages) {
            return error(StatusCode::BAD_REQUEST, e);
        }
        if profile.api_format == ApiFormat::Anthropic
            && body["parallel_tool_calls"] == false
            && messages.get("tools").is_some()
        {
            if messages.get("tool_choice").is_none() {
                messages["tool_choice"] = json!({"type":"auto"});
            }
            messages["tool_choice"]["disable_parallel_tool_use"] = json!(true);
        }
        match translate_request(&messages, profile.api_format) {
            Ok(mut v) => {
                if profile.api_format == ApiFormat::OpenaiChat {
                    if let Some(parallel) = body.get("parallel_tool_calls") {
                        v["parallel_tool_calls"] = parallel.clone();
                    }
                    if let Some(effort) = body["reasoning"].get("effort") {
                        v["reasoning_effort"] = effort.clone();
                    }
                }
                v
            }
            Err(e) => return error(StatusCode::BAD_REQUEST, e),
        }
    };
    let endpoint = match api_endpoint(
        &profile.base_url,
        if compact {
            "responses/compact"
        } else {
            match profile.api_format {
                ApiFormat::Anthropic => "messages",
                ApiFormat::OpenaiChat => "chat/completions",
                ApiFormat::OpenaiResponses => "responses",
            }
        },
    ) {
        Ok(v) => v,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    let mut request = state.client.post(endpoint).json(&upstream_body);
    for name in [
        "user-agent",
        "openai-beta",
        "session_id",
        "conversation_id",
        "x-codex-turn-state",
        "x-client-request-id",
    ] {
        if let Some(value) = headers.get(name) {
            request = request.header(name, value);
        }
    }
    if profile.api_format == ApiFormat::Anthropic {
        request = request.header("anthropic-version", "2023-06-01");
    }
    request = match &profile.credential {
        Credential::Bearer { value } => request.bearer_auth(value),
        Credential::XApiKey { value } => request.header("x-api-key", value),
        Credential::ApiKey { value } => request.header("api-key", value),
        Credential::None => request,
    };
    let response = match tokio::time::timeout(HEADER_TIMEOUT, request.send()).await {
        Ok(Ok(v)) => v,
        _ => {
            return error(
                StatusCode::BAD_GATEWAY,
                "Upstream connection failed or timed out",
            );
        }
    };
    if !response.status().is_success() {
        return error(
            response.status(),
            format!(
                "Provider returned HTTP {}; check its credentials, model and request compatibility",
                response.status()
            ),
        );
    }
    if streaming {
        if native {
            return native_stream(response);
        }
        // Reuse the existing Chat -> Messages stream translator, then emit Responses events.
        let intermediate = if profile.api_format == ApiFormat::Anthropic {
            passthrough_response(response)
        } else {
            stream_response(response, profile.api_format)
        };
        return converted_stream(intermediate, strip_1m(&model), custom, namespaces);
    }
    let bytes =
        match tokio::time::timeout(TOTAL_TIMEOUT, read_body(response, BODY_LIMIT, false)).await {
            Ok(Ok(v)) => v,
            _ => {
                return error(
                    StatusCode::BAD_GATEWAY,
                    "Upstream body failed or exceeded limits",
                );
            }
        };
    let value: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return error(StatusCode::BAD_GATEWAY, "Invalid upstream JSON"),
    };
    if native {
        return Json(value).into_response();
    }
    let message = if profile.api_format == ApiFormat::Anthropic {
        value
    } else {
        match translate_response(&value, profile.api_format) {
            Ok(v) => v,
            Err(e) => return error(StatusCode::BAD_GATEWAY, e),
        }
    };
    match from_message(&message, &custom) {
        Ok(mut v) => {
            restore_namespaces(&mut v, &namespaces);
            Json(v).into_response()
        }
        Err(e) => error(StatusCode::BAD_GATEWAY, e),
    }
}
type Namespaces = HashMap<String, (String, String)>;
fn namespace_alias(namespace: &str, name: &str) -> String {
    use sha2::{Digest, Sha256};
    format!(
        "ccsw_{}",
        &format!("{:x}", Sha256::digest(format!("{namespace}\0{name}")))[..32]
    )
}
fn flatten_namespaces(body: &mut Value) -> Result<Namespaces> {
    let mut names = Namespaces::new();
    let mut tools = vec![];
    for tool in body["tools"].as_array().into_iter().flatten() {
        if tool["type"] != "namespace" {
            tools.push(tool.clone());
            continue;
        }
        let namespace = tool["name"]
            .as_str()
            .context("Namespace name is required")?;
        for child in tool["tools"]
            .as_array()
            .context("Namespace tools must be an array")?
        {
            if !matches!(child["type"].as_str(), Some("function" | "custom")) {
                bail!("Only function/custom namespace tools are supported");
            }
            let name = child["name"].as_str().context("Tool name is required")?;
            let alias = namespace_alias(namespace, name);
            names.insert(alias.clone(), (namespace.into(), name.into()));
            let mut flattened = child.clone();
            flattened["name"] = json!(alias);
            tools.push(flattened);
        }
    }
    let mut unique = BTreeSet::new();
    for tool in &tools {
        if let Some(name) = tool["name"].as_str()
            && !unique.insert(name)
        {
            bail!("Duplicate or conflicting tool names");
        }
    }
    if body.get("tools").is_some() {
        body["tools"] = json!(tools);
    }
    if let Some(items) = body["input"].as_array_mut() {
        for item in items {
            if matches!(
                item["type"].as_str(),
                Some("function_call" | "custom_tool_call")
            ) && let (Some(namespace), Some(name)) =
                (item["namespace"].as_str(), item["name"].as_str())
            {
                let alias = namespace_alias(namespace, name);
                names.insert(alias.clone(), (namespace.into(), name.into()));
                item["name"] = json!(alias);
                item.as_object_mut().unwrap().remove("namespace");
            }
        }
    }
    if let (Some(namespace), Some(name)) = (
        body["tool_choice"]["namespace"].as_str(),
        body["tool_choice"]["name"].as_str(),
    ) {
        let alias = namespace_alias(namespace, name);
        body["tool_choice"]["name"] = json!(alias);
        body["tool_choice"]
            .as_object_mut()
            .unwrap()
            .remove("namespace");
    }
    Ok(names)
}
fn restore_namespaces(value: &mut Value, names: &Namespaces) {
    match value {
        Value::Array(items) => {
            for item in items {
                restore_namespaces(item, names);
            }
        }
        Value::Object(object) => {
            if matches!(
                object.get("type").and_then(Value::as_str),
                Some("function_call" | "custom_tool_call")
            ) && let Some((namespace, name)) = object
                .get("name")
                .and_then(Value::as_str)
                .and_then(|name| names.get(name))
            {
                object.insert("name".into(), json!(name));
                object.insert("namespace".into(), json!(namespace));
            }
            for item in object.values_mut() {
                restore_namespaces(item, names);
            }
        }
        _ => {}
    }
}
fn custom_tools(body: &Value) -> BTreeSet<String> {
    body["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|t| t["type"] == "custom")
        .filter_map(|t| t["name"].as_str().map(str::to_owned))
        .collect()
}
fn text_content(content: &Value) -> Result<Vec<Value>> {
    if let Some(text) = content.as_str() {
        return Ok(vec![json!({"type":"text","text":text})]);
    }
    let mut blocks = vec![];
    for block in content
        .as_array()
        .context("Message content must be text or an array")?
    {
        match block["type"].as_str() {
            Some("input_text" | "output_text" | "text") => {
                blocks.push(json!({"type":"text","text":block["text"]}))
            }
            Some("input_image") => {
                let url = block["image_url"]
                    .as_str()
                    .context("Image requires image_url")?;
                if let Some(data) = url.strip_prefix("data:") {
                    let (kind, data) = data
                        .split_once(";base64,")
                        .context("Only base64 data images are supported")?;
                    blocks.push(json!({"type":"image","source":{"type":"base64","media_type":kind,"data":data}}));
                } else {
                    blocks.push(json!({"type":"image","source":{"type":"url","url":url}}));
                }
            }
            _ => bail!("Unsupported Responses content type for this provider"),
        }
    }
    Ok(blocks)
}
fn append_message(messages: &mut Vec<Value>, role: &str, blocks: Vec<Value>) {
    if let Some(last) = messages.last_mut()
        && last["role"] == role
    {
        last["content"].as_array_mut().unwrap().extend(blocks);
        return;
    }
    messages.push(json!({"role":role,"content":blocks}));
}
pub(super) fn to_messages(body: &Value) -> Result<Value> {
    if body
        .get("previous_response_id")
        .is_some_and(|v| !v.is_null())
    {
        bail!(
            "This provider needs the complete conversation; previous_response_id is not supported"
        );
    }
    let mut system = body["instructions"].as_str().unwrap_or("").to_owned();
    let mut messages = vec![];
    let items = if let Some(text) = body["input"].as_str() {
        vec![json!({"role":"user","content":text})]
    } else {
        body["input"]
            .as_array()
            .context("Responses input must be text or an array")?
            .clone()
    };
    for item in items {
        match item["type"].as_str().unwrap_or("message") {
            "message" => {
                let role = item["role"].as_str().context("Message role is required")?;
                let blocks = text_content(&item["content"])?;
                if ["system", "developer"].contains(&role) {
                    for block in blocks {
                        if block["type"] != "text" {
                            bail!("Non-text system messages cannot be converted");
                        }
                        system.push('\n');
                        system.push_str(block["text"].as_str().unwrap_or(""));
                    }
                } else if ["user", "assistant"].contains(&role) {
                    append_message(&mut messages, role, blocks);
                } else {
                    bail!("Unsupported message role");
                }
            }
            "function_call" | "custom_tool_call" => {
                let arguments = if item["type"] == "custom_tool_call" {
                    json!({"input":item["input"]})
                } else {
                    serde_json::from_str(
                        item["arguments"]
                            .as_str()
                            .context("Missing function arguments")?,
                    )
                    .context("Invalid function arguments JSON")?
                };
                append_message(
                    &mut messages,
                    "assistant",
                    vec![
                        json!({"type":"tool_use","id":item["call_id"],"name":item["name"],"input":arguments}),
                    ],
                );
            }
            "function_call_output" | "custom_tool_call_output" => {
                let output = if item["output"].is_string() {
                    item["output"].clone()
                } else {
                    json!(text_content(&item["output"])?)
                };
                append_message(
                    &mut messages,
                    "user",
                    vec![
                        json!({"type":"tool_result","tool_use_id":item["call_id"],"content":output}),
                    ],
                );
            }
            "reasoning" => {
                // Only summaries have a portable representation; opaque reasoning requires its originating provider.
                if item.get("encrypted_content").is_some_and(|v| !v.is_null()) {
                    bail!(
                        "Encrypted reasoning cannot be moved to a different protocol provider; start a new chat"
                    );
                }
                let summary = item["summary"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                if !summary.is_empty() {
                    append_message(
                        &mut messages,
                        "assistant",
                        vec![json!({"type":"text","text":summary})],
                    );
                }
            }
            other => bail!("Responses item '{other}' is not supported by this provider"),
        }
    }
    let mut tools = vec![];
    for tool in body["tools"].as_array().into_iter().flatten() {
        match tool["type"].as_str() {
            Some("function") => tools.push(json!({"name":tool["name"],"description":tool["description"].as_str().unwrap_or(""),"input_schema":tool.get("parameters").cloned().unwrap_or(json!({"type":"object","properties":{}}))})),
            Some("custom") => tools.push(json!({"name":tool["name"],"description":format!("{}\nReturn the complete tool input as the string property input. {}", tool["description"].as_str().unwrap_or(""), tool.get("format").map(Value::to_string).unwrap_or_default()),"input_schema":{"type":"object","properties":{"input":{"type":"string"}},"required":["input"],"additionalProperties":false}})),
            _ => bail!("This provider only supports function and custom tools; disable hosted tools such as web search"),
        }
    }
    let mut result = json!({"model":body["model"],"messages":messages,"system":system,"stream":body["stream"].as_bool().unwrap_or(false),"max_tokens":body.get("max_output_tokens").cloned().unwrap_or(json!(8192))});
    if !tools.is_empty() {
        result["tools"] = json!(tools);
    }
    if let Some(choice) = body.get("tool_choice") {
        result["tool_choice"] = match choice.as_str() {
            Some("auto") => json!({"type":"auto"}),
            Some("required") => json!({"type":"any"}),
            Some("none") => {
                result.as_object_mut().unwrap().remove("tools");
                json!({"type":"auto"})
            }
            _ if choice["type"] == "function" || choice["type"] == "custom" => {
                json!({"type":"tool","name":choice["name"]})
            }
            _ => bail!("Unsupported tool_choice for this provider"),
        };
        if result.get("tools").is_none() {
            result.as_object_mut().unwrap().remove("tool_choice");
        }
    }
    for key in ["temperature", "top_p"] {
        if let Some(v) = body.get(key) {
            result[key] = v.clone();
        }
    }
    Ok(result)
}
fn output_item(block: &Value, index: usize, custom: &BTreeSet<String>) -> Result<Value> {
    Ok(match block["type"].as_str() {
        Some("text") => {
            json!({"id":format!("msg_{index}"),"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":block["text"],"annotations":[]}]})
        }
        Some("tool_use") => {
            let name = block["name"].as_str().context("Missing tool name")?;
            if custom.contains(name) {
                json!({"id":format!("ct_{index}"),"type":"custom_tool_call","status":"completed","call_id":block["id"],"name":name,"input":block["input"]["input"].as_str().context("Custom tool must return string input")?})
            } else {
                json!({"id":format!("fc_{index}"),"type":"function_call","status":"completed","call_id":block["id"],"name":name,"arguments":serde_json::to_string(&block["input"])?})
            }
        }
        Some("thinking") => {
            json!({"id":format!("rs_{index}"),"type":"reasoning","summary":[{"type":"summary_text","text":block["thinking"]}]})
        }
        _ => bail!("Unsupported upstream output block"),
    })
}
fn from_message(message: &Value, custom: &BTreeSet<String>) -> Result<Value> {
    let output = message["content"]
        .as_array()
        .context("Missing message content")?
        .iter()
        .enumerate()
        .map(|(i, b)| output_item(b, i, custom))
        .collect::<Result<Vec<_>>>()?;
    let incomplete = message["stop_reason"] == "max_tokens";
    Ok(
        json!({"id":format!("resp_{}",Uuid::new_v4().simple()),"object":"response","created_at":0,"model":message["model"],"status":if incomplete {"incomplete"} else {"completed"},"output":output,"error":null,"incomplete_details":if incomplete {json!({"reason":"max_output_tokens"})} else {Value::Null},"usage":{"input_tokens":message["usage"]["input_tokens"].as_u64().unwrap_or(0),"output_tokens":message["usage"]["output_tokens"].as_u64().unwrap_or(0)}}),
    )
}
fn sse(value: &Value) -> Bytes {
    Bytes::from(format!(
        "event: {}\ndata: {}\n\n",
        value["type"].as_str().unwrap_or("error"),
        value
    ))
}
fn stream_error(message: &str) -> Bytes {
    sse(&json!({"type":"error","code":"upstream_error","message":message,"param":null}))
}
fn native_stream(response: reqwest::Response) -> Response {
    let mut upstream = response.bytes_stream();
    let output = async_stream::stream! {
        let mut decoder = Decoder::default();
        loop {
            let bytes = match tokio::time::timeout(IDLE_TIMEOUT, upstream.next()).await { Ok(Some(Ok(v))) => v, _ => { yield Ok::<_,std::convert::Infallible>(stream_error("Upstream ended without completion or timed out")); return; } };
            let frames = match decoder.push(&bytes) { Ok(v) => v, Err(_) => { yield Ok(stream_error("Invalid upstream stream")); return; } };
            let done = frames.iter().any(|s| serde_json::from_str::<Value>(s).is_ok_and(|v| matches!(v["type"].as_str(), Some("response.completed" | "response.failed" | "response.incomplete" | "error"))));
            yield Ok(bytes);
            if done { return; }
        }
    };
    Response::builder()
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(output))
        .unwrap()
}

struct Bridge {
    response: Value,
    blocks: Vec<Value>,
    arguments: HashMap<usize, String>,
    custom: BTreeSet<String>,
    sequence: u64,
    completed: bool,
    namespaces: Namespaces,
}
impl Bridge {
    fn new(model: String, custom: BTreeSet<String>) -> Self {
        Self {
            response: json!({"id":format!("resp_{}",Uuid::new_v4().simple()),"object":"response","created_at":0,"model":model,"status":"in_progress","output":[],"error":null,"incomplete_details":null,"usage":{"input_tokens":0,"output_tokens":0}}),
            blocks: vec![],
            arguments: HashMap::new(),
            custom,
            sequence: 0,
            completed: false,
            namespaces: Namespaces::new(),
        }
    }
    fn emit(&mut self, mut event: Value) -> Bytes {
        restore_namespaces(&mut event, &self.namespaces);
        event["sequence_number"] = json!(self.sequence);
        self.sequence += 1;
        sse(&event)
    }
    fn item_id(&self, index: usize) -> String {
        format!("{}_item_{index}", self.response["id"].as_str().unwrap())
    }
    fn consume(&mut self, event: &Value) -> Result<Vec<Bytes>> {
        let mut result = vec![];
        let index = event["index"].as_u64().unwrap_or(0) as usize;
        if index > 4096 {
            bail!("Too many output blocks");
        }
        match event["type"].as_str() {
            Some("message_start") => {
                self.response["usage"]["input_tokens"] =
                    event["message"]["usage"]["input_tokens"].clone();
                result.push(self.emit(json!({"type":"response.created","response":self.response})));
                result.push(
                    self.emit(json!({"type":"response.in_progress","response":self.response})),
                );
            }
            Some("content_block_start") => {
                if self.blocks.len() != index {
                    bail!("Unexpected output block order");
                }
                let block = event["content_block"].clone();
                let mut item = if block["type"] == "tool_use"
                    && self.custom.contains(block["name"].as_str().unwrap_or(""))
                {
                    json!({"type":"custom_tool_call","call_id":block["id"],"name":block["name"],"input":"","status":"in_progress"})
                } else {
                    output_item(&block, index, &self.custom)?
                };
                item["id"] = json!(self.item_id(index));
                if item.get("status").is_some() {
                    item["status"] = json!("in_progress");
                }
                if block["type"] == "tool_use" {
                    self.arguments.insert(index, String::new());
                    if item["type"] == "function_call" {
                        item["arguments"] = json!("");
                    }
                }
                self.blocks.push(block.clone());
                result.push(self.emit(
                    json!({"type":"response.output_item.added","output_index":index,"item":item}),
                ));
                if block["type"] == "text" {
                    result.push(self.emit(json!({"type":"response.content_part.added","output_index":index,"item_id":self.item_id(index),"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}})));
                }
            }
            Some("content_block_delta") => {
                let block = self
                    .blocks
                    .get_mut(index)
                    .context("Delta before output block")?;
                match event["delta"]["type"].as_str() {
                    Some("text_delta") => {
                        let delta = event["delta"]["text"].as_str().unwrap_or("");
                        let mut text = block["text"].as_str().unwrap_or("").to_owned();
                        text.push_str(delta);
                        block["text"] = json!(text);
                        result.push(self.emit(json!({"type":"response.output_text.delta","output_index":index,"item_id":self.item_id(index),"content_index":0,"delta":delta,"logprobs":[]})));
                    }
                    Some("input_json_delta") => {
                        let delta = event["delta"]["partial_json"].as_str().unwrap_or("");
                        let buffer = self.arguments.entry(index).or_default();
                        if buffer.len() + delta.len() > BODY_LIMIT {
                            bail!("Tool arguments exceed size limit");
                        }
                        buffer.push_str(delta);
                        if !self.custom.contains(block["name"].as_str().unwrap_or("")) {
                            result.push(self.emit(json!({"type":"response.function_call_arguments.delta","output_index":index,"item_id":self.item_id(index),"delta":delta})));
                        }
                    }
                    Some("thinking_delta") => {
                        let delta = event["delta"]["thinking"].as_str().unwrap_or("");
                        let mut text = block["thinking"].as_str().unwrap_or("").to_owned();
                        text.push_str(delta);
                        block["thinking"] = json!(text);
                    }
                    Some("signature_delta") => {}
                    _ => bail!("Unsupported upstream content delta"),
                }
            }
            Some("content_block_stop") => {
                let block = self
                    .blocks
                    .get_mut(index)
                    .context("Output stop before start")?;
                if let Some(arguments) = self.arguments.remove(&index)
                    && !arguments.is_empty()
                {
                    block["input"] = serde_json::from_str(&arguments)
                        .context("Invalid streamed tool arguments")?;
                }
                let mut item = output_item(block, index, &self.custom)?;
                item["id"] = json!(self.item_id(index));
                match item["type"].as_str() {
                    Some("message") => {
                        result.push(self.emit(json!({"type":"response.output_text.done","output_index":index,"item_id":self.item_id(index),"content_index":0,"text":item["content"][0]["text"],"logprobs":[]})));
                        result.push(self.emit(json!({"type":"response.content_part.done","output_index":index,"item_id":self.item_id(index),"content_index":0,"part":item["content"][0]})));
                    }
                    Some("function_call") => result.push(self.emit(json!({"type":"response.function_call_arguments.done","output_index":index,"item_id":self.item_id(index),"arguments":item["arguments"]}))),
                    Some("custom_tool_call") => {
                        result.push(self.emit(json!({"type":"response.custom_tool_call_input.delta","output_index":index,"item_id":self.item_id(index),"delta":item["input"]})));
                        result.push(self.emit(json!({"type":"response.custom_tool_call_input.done","output_index":index,"item_id":self.item_id(index),"input":item["input"]})));
                    }
                    _ => {},
                }
                self.response["output"]
                    .as_array_mut()
                    .unwrap()
                    .push(item.clone());
                result.push(self.emit(
                    json!({"type":"response.output_item.done","output_index":index,"item":item}),
                ));
            }
            Some("message_delta") => {
                if let Some(n) = event["usage"].get("output_tokens") {
                    self.response["usage"]["output_tokens"] = n.clone();
                }
                if let Some(n) = event["usage"].get("input_tokens") {
                    self.response["usage"]["input_tokens"] = n.clone();
                }
                if event["delta"]["stop_reason"] == "max_tokens" {
                    self.response["incomplete_details"] = json!({"reason":"max_output_tokens"});
                }
            }
            Some("message_stop") => {
                self.completed = true;
                let status = if self.response["incomplete_details"].is_null() {
                    "completed"
                } else {
                    "incomplete"
                };
                self.response["status"] = json!(status);
                self.response["usage"]["total_tokens"] = json!(
                    self.response["usage"]["input_tokens"].as_u64().unwrap_or(0)
                        + self.response["usage"]["output_tokens"]
                            .as_u64()
                            .unwrap_or(0)
                );
                result.push(
                    self.emit(
                        json!({"type":format!("response.{status}"),"response":self.response}),
                    ),
                );
            }
            Some("ping") => {}
            Some("error") => bail!("Upstream stream reported an error"),
            _ => bail!("Unsupported upstream event"),
        }
        Ok(result)
    }
}
fn converted_stream(
    response: Response,
    model: String,
    custom: BTreeSet<String>,
    namespaces: Namespaces,
) -> Response {
    let mut stream = response.into_body().into_data_stream();
    let output = async_stream::stream! {
        let mut decoder = Decoder::default();
        let mut bridge = Bridge::new(model, custom);
        bridge.namespaces = namespaces;
        loop {
            let bytes = match tokio::time::timeout(IDLE_TIMEOUT, stream.next()).await {
                Ok(Some(Ok(bytes))) => bytes,
                _ => { yield Ok::<_,std::convert::Infallible>(stream_error("Upstream ended without completion or timed out")); return; }
            };
            let frames = match decoder.push(&bytes) { Ok(v) => v, Err(_) => { yield Ok(stream_error("Invalid upstream stream")); return; } };
            for frame in frames {
                let event = match serde_json::from_str::<Value>(&frame) { Ok(v) => v, Err(_) => { yield Ok(stream_error("Invalid upstream JSON")); return; } };
                match bridge.consume(&event) { Ok(events) => for event in events { yield Ok(event); }, Err(e) => { yield Ok(stream_error(&e.to_string())); return; } }
                if bridge.completed { return; }
            }
        }
    };
    Response::builder()
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(output))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn custom_tools_and_multi_turn_results_round_trip() {
        let request = json!({"model":"m","instructions":"Be helpful","input":[{"role":"developer","content":[{"type":"input_text","text":"Use tools"}]},{"role":"user","content":"Edit a file"},{"type":"custom_tool_call","call_id":"call_1","name":"apply_patch","input":"*** Begin Patch\n*** End Patch"},{"type":"custom_tool_call_output","call_id":"call_1","output":"Done"}],"tools":[{"type":"custom","name":"apply_patch","description":"Apply a patch","format":{"type":"text"}}]});
        let converted = to_messages(&request).unwrap();
        assert_eq!(
            converted["messages"][1]["content"][0]["input"]["input"],
            "*** Begin Patch\n*** End Patch"
        );
        assert_eq!(
            converted["messages"][2]["content"][0]["tool_use_id"],
            "call_1"
        );
        let response = from_message(&json!({"model":"m","stop_reason":"tool_use","content":[{"type":"tool_use","id":"call_1","name":"apply_patch","input":{"input":"*** Begin Patch\n*** End Patch"}}]}), &custom_tools(&request)).unwrap();
        assert_eq!(response["output"][0]["type"], "custom_tool_call");
        assert_eq!(
            response["output"][0]["input"],
            "*** Begin Patch\n*** End Patch"
        );
        assert_eq!(response["output"][0]["call_id"], "call_1");
    }
    #[test]
    fn unsupported_hosted_tools_and_opaque_history_fail_explicitly() {
        for extra in [
            json!({"tools":[{"type":"web_search"}]}),
            json!({"previous_response_id":"resp_old"}),
            json!({"input":[{"type":"reasoning","encrypted_content":"opaque"}]}),
        ] {
            let mut request = json!({"model":"m","input":"hello"});
            request
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            assert!(to_messages(&request).is_err());
        }
    }
    #[test]
    fn streaming_parallel_tools_preserves_ids_arguments_and_completion() {
        let mut bridge = Bridge::new("m".into(), BTreeSet::from(["apply_patch".into()]));
        let events = [
            json!({"type":"message_start","message":{"usage":{"input_tokens":3}}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"你好"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"call_one","name":"shell","input":{}}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"cmd\":"}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"ls\"}"}}),
            json!({"type":"content_block_stop","index":1}),
            json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"call_two","name":"apply_patch","input":{}}}),
            json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"input\":\"patch\\ntext\"}"}}),
            json!({"type":"content_block_stop","index":2}),
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":7}}),
            json!({"type":"message_stop"}),
        ];
        let mut decoder = Decoder::default();
        let mut emitted = vec![];
        for event in events {
            for bytes in bridge.consume(&event).unwrap() {
                for frame in decoder.push(&bytes).unwrap() {
                    emitted.push(serde_json::from_str::<Value>(&frame).unwrap());
                }
            }
        }
        assert!(bridge.completed);
        let done = &emitted.last().unwrap()["response"];
        assert_eq!(done["output"][0]["content"][0]["text"], "你好");
        assert_eq!(done["output"][1]["call_id"], "call_one");
        assert_eq!(done["output"][1]["arguments"], r#"{"cmd":"ls"}"#);
        assert_eq!(done["output"][2]["input"], "patch\ntext");
        assert_eq!(done["usage"]["total_tokens"], 10);
        assert_eq!(
            emitted
                .iter()
                .filter(|e| e["type"] == "response.completed")
                .count(),
            1
        );
        for (i, event) in emitted.iter().enumerate() {
            assert_eq!(event["sequence_number"], i);
        }
    }
    #[test]
    fn images_and_function_results_convert_to_both_upstreams() {
        let body = json!({"model":"m","input":[{"role":"user","content":[{"type":"input_image","image_url":"data:image/png;base64,YWJj"}]},{"type":"function_call","call_id":"call","name":"shell","arguments":"{}"},{"type":"function_call_output","call_id":"call","output":"done"}],"tools":[{"type":"function","name":"shell","parameters":{"type":"object","properties":{}}}]});
        let messages = to_messages(&body).unwrap();
        assert_eq!(
            messages["messages"][0]["content"][0]["source"]["media_type"],
            "image/png"
        );
        let chat = translate_request(&messages, ApiFormat::OpenaiChat).unwrap();
        assert_eq!(chat["messages"][2]["tool_call_id"], "call");
    }
    #[tokio::test]
    async fn responses_http_ingress_routes_all_protocols_and_enforces_auth_and_limits() {
        for format in [
            ApiFormat::Anthropic,
            ApiFormat::OpenaiChat,
            ApiFormat::OpenaiResponses,
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let (sender, mut receiver) = tokio::sync::mpsc::channel(4);
            let upstream = Router::new().fallback(post(move |headers: HeaderMap, Json(body): Json<Value>| {
                let sender = sender.clone();
                async move {
                    sender.send((headers,body)).await.unwrap();
                    Json(match format {
                        ApiFormat::Anthropic => json!({"id":"msg_test","model":"m","content":[{"type":"tool_use","id":"call_1","name":"shell","input":{"cmd":"pwd"}}],"stop_reason":"tool_use","usage":{"input_tokens":2,"output_tokens":3}}),
                        ApiFormat::OpenaiChat => json!({"id":"chat_test","model":"m","choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"shell","arguments":"{\"cmd\":\"pwd\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":2,"completion_tokens":3}}),
                        ApiFormat::OpenaiResponses => json!({"id":"resp_test","status":"completed","output":[{"type":"function_call","call_id":"call_1","name":"shell","arguments":"{\"cmd\":\"pwd\"}"}]}),
                    })
                }
            }));
            let task = tokio::spawn(async move {
                axum::serve(listener, upstream).await.unwrap();
            });
            let temp = tempfile::tempdir().unwrap();
            let config_path = temp.path().join("config.toml");
            let mut config = Config::default();
            let profile: Profile = toml::from_str(&format!("name='Test'\nbase_url='http://{address}/v1'\ndefault_model='m'\n[[models]]\nid='m'\nmax_output_tokens=100\n")).unwrap();
            config.codex.profiles.insert(
                "test".into(),
                Profile {
                    api_format: format,
                    credential: Credential::Bearer {
                        value: "upstream-only".into(),
                    },
                    ..profile
                },
            );
            crate::config::update(&config_path, |c| {
                *c = config;
                Ok(())
            })
            .unwrap();
            let registry = temp.path().join("registry.json");
            fs::write(
                &registry,
                serde_json::to_vec(&Registry {
                    listen: "127.0.0.1:1".into(),
                    local_token: "local-only".into(),
                    routes: BTreeMap::from([(
                        "route".into(),
                        RouteTarget {
                            codex: true,
                            config_path,
                            profile_id: Some("test".into()),
                            models: BTreeMap::new(),
                        },
                    )]),
                })
                .unwrap(),
            )
            .unwrap();
            let state = ServerState {
                shutdown: Default::default(),
                registry,
                client: Client::new(),
            };
            let body = json!({"model":"m","input":"Read a file","max_output_tokens":1000,"tools":[{"type":"function","name":"shell","parameters":{"type":"object","properties":{"cmd":{"type":"string"}}}}]});
            let denied = serve(
                state.clone(),
                "route".into(),
                HeaderMap::new(),
                body.clone(),
                false,
            )
            .await;
            assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
            let headers = HeaderMap::from_iter([(
                header::AUTHORIZATION,
                header::HeaderValue::from_static("Bearer local-only"),
            )]);
            let response = serve(state, "route".into(), headers, body, false).await;
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = axum::body::to_bytes(response.into_body(), BODY_LIMIT)
                .await
                .unwrap();
            let value: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(value["output"][0]["call_id"], "call_1");
            let (headers, request) = receiver.recv().await.unwrap();
            assert_eq!(headers[header::AUTHORIZATION], "Bearer upstream-only");
            assert_eq!(
                request[if format == ApiFormat::OpenaiResponses {
                    "max_output_tokens"
                } else {
                    "max_tokens"
                }],
                100
            );
            task.abort();
        }
    }
    #[test]
    fn namespace_tools_round_trip_without_colliding_with_plain_names() {
        let mut body = json!({"model":"m","input":[{"type":"function_call","namespace":"agent","name":"wait","call_id":"call_1","arguments":"{}"}],"tools":[{"type":"function","name":"wait","parameters":{"type":"object"}},{"type":"namespace","name":"agent","tools":[{"type":"function","name":"wait","parameters":{"type":"object"}}]}]});
        let names = flatten_namespaces(&mut body).unwrap();
        let alias = body["tools"][1]["name"].as_str().unwrap();
        assert_ne!(alias, "wait");
        assert_eq!(body["input"][0]["name"], alias);
        let message = json!({"model":"m","content":[{"type":"tool_use","id":"call_1","name":alias,"input":{}}]});
        let mut response = from_message(&message, &BTreeSet::new()).unwrap();
        restore_namespaces(&mut response, &names);
        assert_eq!(response["output"][0]["name"], "wait");
        assert_eq!(response["output"][0]["namespace"], "agent");
        assert_eq!(response["output"][0]["call_id"], "call_1");
        assert_eq!(body["tools"][0]["name"], "wait");
    }
}
