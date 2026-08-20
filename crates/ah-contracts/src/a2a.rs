//! a2a seam:A2A(Agent2Agent)协议转换与适配(对齐
//! `openjiuwen/extensions/a2a/a2a_transformer.py` + `a2a_agentcard_adapter.py` +
//! `a2a_client.py` + `a2a_server.py` 的确定性部分)。
//!
//! - `A2ATransformer` 纯函数:openjiuwen payload ↔ A2A payload(request/
//!   response/part/artifact 双向转换,状态映射,A2A TaskState → openjiuwen
//!   TaskStatus);
//! - `A2AAgentCardAdapter`:openjiuwen AgentCard ↔ A2A AgentCard(描述拼接
//!   [input_params]/[output_params]、接口列表构建);
//! - `A2AClient` 合并逻辑:session id 解析、AgentResult 跨事件聚合
//!   (artifacts 拼接/metadata 合并/状态优先级);
//! - server 归一化:`normalize_jsonrpc_route_path`(尾斜杠)/
//!   `normalize_jsonrpc_interface_url`(jsonrpc 路径尾斜杠)、
//!   `resolve_transport_protocols`(JSONRPC/HTTP+JSON,gRPC 显式拒绝)。
//!
//! 纯函数、无 IO、无 LLM;契约零实现(HTTP/SSE 传输走 transport seam)。

use crate::controller::TaskStatus;
use serde_json::{Map, Value};

/// A2A TaskState 状态名 → openjiuwen TaskStatus(对齐 `_A2A_STATUS_TO_OJW_STATUS`)。
pub fn a2a_status_to_ojw(status: Option<&str>) -> TaskStatus {
    match status {
        None => TaskStatus::Unknown,
        Some("TASK_STATE_UNSPECIFIED") => TaskStatus::Unknown,
        Some("TASK_STATE_SUBMITTED") => TaskStatus::Submitted,
        Some("TASK_STATE_WORKING") => TaskStatus::Working,
        Some("TASK_STATE_COMPLETED") => TaskStatus::Completed,
        Some("TASK_STATE_FAILED") => TaskStatus::Failed,
        Some("TASK_STATE_CANCELED") => TaskStatus::Canceled,
        Some("TASK_STATE_INPUT_REQUIRED") => TaskStatus::InputRequired,
        Some("TASK_STATE_REJECTED") => TaskStatus::Failed,
        Some("TASK_STATE_AUTH_REQUIRED") => TaskStatus::InputRequired,
        Some(_) => TaskStatus::Unknown,
    }
}

/// 把 openjiuwen request dict 转成 A2A SendMessageRequest 形状
/// (对齐 `to_a2a_request`;返回 JSON object)。
///
/// - message_id:随机 hex(调用方注入 uuid;此处校验存在);
/// - role: "user";
/// - context_id: `conversation_id` 或 `sessionId`;
/// - parts[0].text = `query`(存在时);
/// - metadata: 其余非空键(query/sessionId/conversation_id 排除)。
pub fn to_a2a_request(request: &Value, message_id: &str) -> Result<Value, A2aError> {
    let obj = request.as_object().ok_or_else(|| {
        A2aError(format!(
            "request must be a dict, got {}",
            request_type_name(request)
        ))
    })?;
    let session_id = obj
        .get("conversation_id")
        .or_else(|| obj.get("sessionId"))
        .and_then(Value::as_str);
    let mut message = Map::new();
    message.insert(
        "message_id".to_string(),
        Value::String(message_id.to_string()),
    );
    message.insert("role".to_string(), Value::String("user".to_string()));
    if let Some(session_id) = session_id {
        message.insert(
            "context_id".to_string(),
            Value::String(session_id.to_string()),
        );
    }
    if let Some(text) = obj.get("query").and_then(Value::as_str) {
        let mut parts = Vec::new();
        let mut part = Map::new();
        part.insert("text".to_string(), Value::String(text.to_string()));
        parts.push(Value::Object(part));
        message.insert("parts".to_string(), Value::Array(parts));
    }
    let metadata: Map<String, Value> = obj
        .iter()
        .filter(|(key, value)| {
            !matches!(key.as_str(), "query" | "sessionId" | "conversation_id") && !value.is_null()
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if !metadata.is_empty() {
        message.insert("metadata".to_string(), to_struct(Value::Object(metadata)));
    }
    let mut send = Map::new();
    send.insert("message".to_string(), Value::Object(message));
    Ok(Value::Object(send))
}

fn request_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(_) => "int",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

/// 从 A2A Message 提取 openjiuwen payload(对齐 `_message_to_payload`)。
pub fn message_to_payload(message: &Value) -> Value {
    let mut payload = Map::new();
    if let Some(parts) = message.get("parts").and_then(Value::as_array) {
        for part in parts {
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                payload.insert("query".to_string(), Value::String(text.to_string()));
                break;
            }
        }
    }
    if let Some(task_id) = message.get("task_id").and_then(Value::as_str) {
        payload.insert("task_id".to_string(), Value::String(task_id.to_string()));
    }
    if let Some(context_id) = message.get("context_id").and_then(Value::as_str) {
        payload.insert(
            "sessionId".to_string(),
            Value::String(context_id.to_string()),
        );
    }
    merge_metadata(
        Value::Object(payload),
        from_struct(message.get("metadata")).as_ref(),
    )
}

/// 合并 metadata 到 payload(未占用键且非空;对齐 `_merge_metadata`)。
pub fn merge_metadata(payload: Value, metadata: Option<&Value>) -> Value {
    let mut payload = payload;
    let Some(metadata) = metadata.and_then(Value::as_object) else {
        return payload;
    };
    let obj = payload.as_object_mut().expect("payload is object");
    for (key, value) in metadata {
        if !value.is_null() && !obj.contains_key(key) {
            obj.insert(key.clone(), value.clone());
        }
    }
    payload
}

/// 转 dict 为 A2A Struct 形状(JSON object;对齐 `_to_struct`)。
pub fn to_struct(data: Value) -> Value {
    data
}

/// 从 A2A Struct 形状提取 dict(对齐 `_from_struct`)。
pub fn from_struct(struct_value: Option<&Value>) -> Option<Value> {
    struct_value.filter(|v| v.is_object()).cloned()
}

/// 解析 A2A Message 的 session id(对齐 `_resolve_session_id`)。
pub fn resolve_session_id(inputs: &Value) -> Option<String> {
    let obj = inputs.as_object()?;
    let session_id = obj
        .get("conversation_id")
        .or_else(|| obj.get("sessionId"))?;
    Some(session_id.as_str().unwrap_or_default().to_string())
}

/// A2A part → openjiuwen part 投影(对齐 `_a2a_part_to_part`)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct A2aPartView {
    pub text: Option<String>,
    pub raw: Option<Value>,
    pub url: Option<String>,
    pub data: Option<Value>,
    pub filename: Option<String>,
    pub media_type: Option<String>,
    #[serde(default)]
    pub metadata: Map<String, Value>,
}

/// openjiuwen part → A2A part 形状(对齐 `to_a2a_part`)。
pub fn to_a2a_part(part: &A2aPartView) -> Value {
    let mut out = Map::new();
    if let Some(text) = &part.text {
        out.insert("text".to_string(), Value::String(text.clone()));
    }
    if let Some(raw) = &part.raw {
        out.insert("raw".to_string(), raw.clone());
    }
    if let Some(url) = &part.url {
        out.insert("url".to_string(), Value::String(url.clone()));
    }
    if let Some(data) = &part.data {
        // dict → data.struct_value;其余 → data.string_value。
        let mut data_obj = Map::new();
        if let Some(struct_value) = data.as_object() {
            data_obj.insert(
                "struct_value".to_string(),
                Value::Object(struct_value.clone()),
            );
        } else if let Some(s) = data.as_str() {
            data_obj.insert("string_value".to_string(), Value::String(s.to_string()));
        } else {
            data_obj.insert("string_value".to_string(), Value::String(data.to_string()));
        }
        out.insert("data".to_string(), Value::Object(data_obj));
    }
    if let Some(filename) = &part.filename {
        out.insert("filename".to_string(), Value::String(filename.clone()));
    }
    if let Some(media_type) = &part.media_type {
        out.insert("media_type".to_string(), Value::String(media_type.clone()));
    }
    if !part.metadata.is_empty() {
        out.insert(
            "metadata".to_string(),
            to_struct(Value::Object(part.metadata.clone())),
        );
    }
    Value::Object(out)
}

/// A2A artifact → openjiuwen Artifact 视图(对齐 `_a2a_artifact_to_artifact`)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ArtifactView {
    pub artifact_id: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub parts: Vec<A2aPartView>,
    #[serde(default)]
    pub metadata: Map<String, Value>,
}

/// A2A artifact JSON → ArtifactView(对齐 `_a2a_artifact_to_artifact`)。
pub fn a2a_artifact_to_artifact(artifact: Option<&Value>) -> ArtifactView {
    let obj = artifact.and_then(Value::as_object);
    ArtifactView {
        artifact_id: obj
            .and_then(|o| o.get("artifact_id"))
            .and_then(Value::as_str)
            .map(str::to_string),
        name: obj
            .and_then(|o| o.get("name"))
            .and_then(Value::as_str)
            .map(str::to_string),
        description: obj
            .and_then(|o| o.get("description"))
            .and_then(Value::as_str)
            .map(str::to_string),
        parts: obj
            .and_then(|o| o.get("parts"))
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .map(|p| {
                        let po = p.as_object();
                        A2aPartView {
                            text: po
                                .and_then(|o| o.get("text"))
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            raw: po.and_then(|o| o.get("raw")).cloned(),
                            url: po
                                .and_then(|o| o.get("url"))
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            data: po.and_then(|o| o.get("data")).map(|d| {
                                // data.struct_value / data.string_value → data。
                                if let Some(sv) = d.as_object().and_then(|o| o.get("struct_value"))
                                {
                                    sv.clone()
                                } else if let Some(s) = d
                                    .as_object()
                                    .and_then(|o| o.get("string_value"))
                                    .and_then(Value::as_str)
                                {
                                    Value::String(s.to_string())
                                } else if let Some(s) = d.as_str() {
                                    Value::String(s.to_string())
                                } else {
                                    Value::String(d.to_string())
                                }
                            }),
                            filename: po
                                .and_then(|o| o.get("filename"))
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            media_type: po
                                .and_then(|o| o.get("media_type"))
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            metadata: po
                                .and_then(|o| o.get("metadata"))
                                .and_then(Value::as_object)
                                .cloned()
                                .unwrap_or_default(),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default(),
        metadata: obj
            .and_then(|o| o.get("metadata"))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default(),
    }
}

/// openjiuwen AgentResult 视图(A2A 客户端聚合目标)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AgentResultView {
    pub task_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub status: Option<TaskStatus>,
    #[serde(default)]
    pub artifacts: Vec<ArtifactView>,
    #[serde(default)]
    pub metadata: Map<String, Value>,
}

/// 构建 AgentResult(对齐 `_build_agent_result`)。
pub fn build_agent_result(
    task_id: Option<String>,
    session_id: Option<String>,
    status: Option<TaskStatus>,
    artifacts: Vec<ArtifactView>,
    metadata: Map<String, Value>,
) -> AgentResultView {
    AgentResultView {
        task_id,
        session_id,
        status,
        artifacts,
        metadata,
    }
}

/// 从 A2A Task JSON 构建 AgentResult(对齐 `_a2a_task_to_result`)。
pub fn a2a_task_to_result(task: Option<&Value>) -> AgentResultView {
    let obj = task.and_then(Value::as_object);
    let status_state = obj
        .and_then(|o| o.get("status"))
        .and_then(|s| s.get("state"))
        .and_then(Value::as_str);
    let artifacts = obj
        .and_then(|o| o.get("artifacts"))
        .and_then(Value::as_array)
        .map(|arts| {
            arts.iter()
                .map(|a| a2a_artifact_to_artifact(Some(a)))
                .collect()
        })
        .unwrap_or_default();
    build_agent_result(
        obj.and_then(|o| o.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string),
        obj.and_then(|o| o.get("context_id"))
            .and_then(Value::as_str)
            .map(str::to_string),
        Some(a2a_status_to_ojw(status_state)),
        artifacts,
        obj.and_then(|o| o.get("metadata"))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default(),
    )
}

/// 从 A2A Message JSON 构建 AgentResult(对齐 `_a2a_message_to_result`)。
pub fn a2a_message_to_result(message: &Value) -> AgentResultView {
    let obj = message.as_object();
    let parts = obj
        .and_then(|o| o.get("parts"))
        .and_then(Value::as_array)
        .map(|ps| {
            ps.iter()
                .map(|p| {
                    let po = p.as_object();
                    A2aPartView {
                        text: po
                            .and_then(|o| o.get("text"))
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        raw: po.and_then(|o| o.get("raw")).cloned(),
                        url: po
                            .and_then(|o| o.get("url"))
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        data: po.and_then(|o| o.get("data")).map(|d| {
                            if let Some(sv) = d.as_object().and_then(|o| o.get("struct_value")) {
                                sv.clone()
                            } else if let Some(s) = d
                                .as_object()
                                .and_then(|o| o.get("string_value"))
                                .and_then(Value::as_str)
                            {
                                Value::String(s.to_string())
                            } else if let Some(s) = d.as_str() {
                                Value::String(s.to_string())
                            } else {
                                Value::String(d.to_string())
                            }
                        }),
                        filename: po
                            .and_then(|o| o.get("filename"))
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        media_type: po
                            .and_then(|o| o.get("media_type"))
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        metadata: po
                            .and_then(|o| o.get("metadata"))
                            .and_then(Value::as_object)
                            .cloned()
                            .unwrap_or_default(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    build_agent_result(
        obj.and_then(|o| o.get("task_id"))
            .and_then(Value::as_str)
            .map(str::to_string),
        obj.and_then(|o| o.get("context_id"))
            .and_then(Value::as_str)
            .map(str::to_string),
        Some(TaskStatus::Completed),
        vec![ArtifactView {
            artifact_id: Some("message".to_string()),
            name: None,
            description: None,
            parts,
            metadata: Map::new(),
        }],
        obj.and_then(|o| o.get("metadata"))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default(),
    )
}

/// 跨事件聚合 AgentResult(对齐 `_merge_agent_results`)。
pub fn merge_agent_results(base: &AgentResultView, update: &AgentResultView) -> AgentResultView {
    let mut artifacts = base.artifacts.clone();
    artifacts.extend(update.artifacts.clone());
    let mut metadata = base.metadata.clone();
    for (k, v) in &update.metadata {
        metadata.insert(k.clone(), v.clone());
    }
    let session_id = update
        .session_id
        .clone()
        .or_else(|| base.session_id.clone());
    let task_id = update.task_id.clone().or_else(|| base.task_id.clone());
    let status = match update.status {
        Some(status) if status != TaskStatus::Unknown => Some(status),
        _ => base.status,
    };
    AgentResultView {
        task_id,
        session_id,
        status,
        artifacts,
        metadata,
    }
}

/// 聚合后回填 session id(对齐 `_with_session_id`)。
pub fn with_session_id(mut result: AgentResultView, session_id: Option<String>) -> AgentResultView {
    if session_id.is_some() {
        result.session_id = session_id;
    }
    result
}

// --- server 归一化 ---

/// 归一化 JSON-RPC 路由路径:以 "/" 开头 + 尾斜杠(对齐 `_normalize_jsonrpc_route_path`)。
pub fn normalize_jsonrpc_route_path(rpc_url: &str) -> String {
    let with_slash = if rpc_url.starts_with('/') {
        rpc_url.to_string()
    } else {
        format!("/{rpc_url}")
    };
    let trimmed = with_slash.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        format!("{trimmed}/")
    }
}

/// 归一化发布的 JSON-RPC interface_url 与挂载路径对齐
/// (对齐 `_normalize_jsonrpc_interface_url`;jsonrpc 路径补尾斜杠)。
pub fn normalize_jsonrpc_interface_url(interface_url: Option<&str>) -> Option<String> {
    let url = interface_url?;
    if url.is_empty() {
        return None;
    }
    // 解析 scheme://authority/path(仅路径段处理)。
    let (prefix, path) = match url.find("://") {
        Some(idx) => {
            let rest = &url[idx + 3..];
            match rest.find('/') {
                Some(slash) => (&url[..idx + 3 + slash], &rest[slash..]),
                None => (url, ""),
            }
        }
        None => {
            let trimmed = url.trim_start_matches('/');
            ("", &url[url.len() - trimmed.len()..])
        }
    };
    if !path.to_lowercase().contains("jsonrpc") {
        return Some(url.to_string());
    }
    let norm_path = format!("{}/", path.trim_end_matches('/'));
    Some(format!("{prefix}{norm_path}"))
}

/// 解析传输协议集合(对齐 `_resolve_transport_protocols`;gRPC 显式拒绝)。
///
/// 从 A2A card 的 supported_interfaces 提取 protocol_binding 集合;
/// 空集默认 JSONRPC。
pub fn resolve_transport_protocols(
    supported_interfaces: &[Value],
) -> Result<Vec<String>, A2aError> {
    let mut transports = std::collections::BTreeSet::new();
    for interface in supported_interfaces {
        let Some(binding) = interface
            .as_object()
            .and_then(|o| o.get("protocol_binding"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let upper = binding.to_uppercase();
        if upper == "GRPC" {
            return Err(A2aError("gRPC transport is not supported.".to_string()));
        }
        if upper == "HTTP+JSON" || upper == "HTTP_JSON" {
            transports.insert("HTTP+JSON".to_string());
        } else if !upper.is_empty() {
            transports.insert(binding.to_string());
        }
    }
    if transports.is_empty() {
        transports.insert("JSONRPC".to_string());
    }
    Ok(transports.into_iter().collect())
}

// --- AgentCard 适配 ---

/// A2A AgentCard 接口项。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct A2aInterface {
    pub url: String,
    pub protocol_binding: String,
    pub protocol_version: String,
    #[serde(default)]
    pub tenant: Option<String>,
}

/// openjiuwen AgentCard → A2A AgentCard 视图(对齐 `to_a2a_agent_card` 确定性部分)。
///
/// 描述拼接 `[input_params]` / `[output_params]`(JSON 序列化),capabilities =
/// streaming=true + push_notifications=false,接口列表按 supported_interfaces
/// 或 interface_url 构建。参数镜像 Python 签名(1:1),故允许超 7 参。
#[allow(clippy::too_many_arguments)]
pub fn to_a2a_agent_card(
    name: &str,
    description: &str,
    input_params: Option<&Value>,
    output_params: Option<&Value>,
    interface_url: Option<&str>,
    protocol_binding: &str,
    protocol_version: &str,
    tenant: Option<&str>,
    supported_interfaces: &[Value],
) -> Value {
    let description = build_description(description, input_params, output_params);
    let mut card = Map::new();
    card.insert("name".to_string(), Value::String(name.to_string()));
    card.insert("description".to_string(), Value::String(description));
    let capabilities = serde_json::json!({
        "streaming": true,
        "push_notifications": false,
    });
    card.insert("capabilities".to_string(), capabilities);
    card.insert(
        "default_input_modes".to_string(),
        Value::Array(vec![
            Value::String("text/plain".to_string()),
            Value::String("application/json".to_string()),
        ]),
    );
    card.insert(
        "default_output_modes".to_string(),
        Value::Array(vec![
            Value::String("text/plain".to_string()),
            Value::String("application/json".to_string()),
        ]),
    );
    let interfaces = build_interfaces(
        interface_url,
        protocol_binding,
        protocol_version,
        tenant,
        supported_interfaces,
    );
    if !interfaces.is_empty() {
        card.insert("supported_interfaces".to_string(), Value::Array(interfaces));
    }
    Value::Object(card)
}

/// 序列化参数 payload(对齐 `_serialize_param_payload`)。
pub fn serialize_param_payload(value: Option<&Value>) -> String {
    match value {
        None => String::new(),
        Some(v) => {
            let payload = if v.is_object() {
                v.clone()
            } else {
                serde_json::json!({ "value": v.to_string() })
            };
            // sort_keys=True 语义:规范化 JSON。
            sort_json_keys(&payload)
        }
    }
}

fn sort_json_keys(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let sorted: Map<String, Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), sort_json_value(v)))
                .collect();
            serde_json::to_string(&Value::Object(sorted)).unwrap_or_default()
        }
        _ => value.to_string(),
    }
}

fn sort_json_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted: Map<String, Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), sort_json_value(v)))
                .collect();
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.iter().map(sort_json_value).collect()),
        other => other.clone(),
    }
}

/// 构建描述(对齐 `_build_description`)。
pub fn build_description(
    base_description: &str,
    input_params: Option<&Value>,
    output_params: Option<&Value>,
) -> String {
    let mut sections = Vec::new();
    let base = base_description.trim();
    if !base.is_empty() {
        sections.push(base.to_string());
    }
    let input_text = serialize_param_payload(input_params);
    let output_text = serialize_param_payload(output_params);
    if !input_text.is_empty() {
        sections.push(format!("[input_params] {input_text}"));
    }
    if !output_text.is_empty() {
        sections.push(format!("[output_params] {output_text}"));
    }
    sections.join("\n").trim().to_string()
}

/// 构建接口列表(对齐 `_build_interfaces`;supported_interfaces 优先,
/// 无效条目跳过;否则回退 interface_url)。
pub fn build_interfaces(
    interface_url: Option<&str>,
    protocol_binding: &str,
    protocol_version: &str,
    tenant: Option<&str>,
    supported_interfaces: &[Value],
) -> Vec<Value> {
    let mut result = Vec::new();
    for item in supported_interfaces {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let Some(url) = obj.get("url").and_then(Value::as_str) else {
            continue;
        };
        let Some(binding) = obj.get("protocol_binding").and_then(Value::as_str) else {
            continue;
        };
        let Some(version) = obj.get("protocol_version").and_then(Value::as_str) else {
            continue;
        };
        if url.is_empty() || binding.is_empty() || version.is_empty() {
            continue;
        }
        let mut interface = Map::new();
        interface.insert("url".to_string(), Value::String(url.to_string()));
        interface.insert(
            "protocol_binding".to_string(),
            Value::String(binding.to_string()),
        );
        interface.insert(
            "protocol_version".to_string(),
            Value::String(version.to_string()),
        );
        if let Some(t) = obj
            .get("tenant")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
        {
            interface.insert("tenant".to_string(), Value::String(t.to_string()));
        }
        result.push(Value::Object(interface));
    }
    if result.is_empty()
        && let Some(url) = interface_url.filter(|u| !u.is_empty())
    {
        let mut interface = Map::new();
        interface.insert("url".to_string(), Value::String(url.to_string()));
        interface.insert(
            "protocol_binding".to_string(),
            Value::String(protocol_binding.to_string()),
        );
        interface.insert(
            "protocol_version".to_string(),
            Value::String(protocol_version.to_string()),
        );
        if let Some(t) = tenant.filter(|t| !t.is_empty()) {
            interface.insert("tenant".to_string(), Value::String(t.to_string()));
        }
        result.push(Value::Object(interface));
    }
    result
}

/// a2a 转换错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct A2aError(pub String);

impl core::fmt::Display for A2aError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for A2aError {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn status_mapping_all_keys() {
        assert_eq!(
            a2a_status_to_ojw(Some("TASK_STATE_COMPLETED")),
            TaskStatus::Completed
        );
        assert_eq!(
            a2a_status_to_ojw(Some("TASK_STATE_WORKING")),
            TaskStatus::Working
        );
        assert_eq!(
            a2a_status_to_ojw(Some("TASK_STATE_FAILED")),
            TaskStatus::Failed
        );
        assert_eq!(
            a2a_status_to_ojw(Some("TASK_STATE_CANCELED")),
            TaskStatus::Canceled
        );
        assert_eq!(
            a2a_status_to_ojw(Some("TASK_STATE_INPUT_REQUIRED")),
            TaskStatus::InputRequired
        );
        assert_eq!(
            a2a_status_to_ojw(Some("TASK_STATE_REJECTED")),
            TaskStatus::Failed
        );
        assert_eq!(
            a2a_status_to_ojw(Some("TASK_STATE_AUTH_REQUIRED")),
            TaskStatus::InputRequired
        );
        assert_eq!(
            a2a_status_to_ojw(Some("TASK_STATE_SUBMITTED")),
            TaskStatus::Submitted
        );
        assert_eq!(a2a_status_to_ojw(None), TaskStatus::Unknown);
        assert_eq!(a2a_status_to_ojw(Some("BOGUS")), TaskStatus::Unknown);
    }

    #[test]
    fn to_a2a_request_builds_message() {
        let req = json!({
            "query": "hello",
            "conversation_id": "s1",
            "extra": "meta",
            "skip": null,
        });
        let out = to_a2a_request(&req, "abc123").expect("ok");
        let message = &out["message"];
        assert_eq!(message["message_id"], "abc123");
        assert_eq!(message["role"], "user");
        assert_eq!(message["context_id"], "s1");
        assert_eq!(message["parts"][0]["text"], "hello");
        // metadata 排除 query/conversation_id,跳过 null。
        assert_eq!(message["metadata"]["extra"], "meta");
        assert!(message["metadata"].get("skip").is_none());
        assert!(message["metadata"].get("query").is_none());
        // 非 dict → Err。
        assert!(to_a2a_request(&json!("str"), "id").is_err());
    }

    #[test]
    fn to_a2a_request_uses_session_id_fallback() {
        let req = json!({"query": "hi", "sessionId": "s2"});
        let out = to_a2a_request(&req, "id").expect("ok");
        assert_eq!(out["message"]["context_id"], "s2");
        // 无 query → 无 parts。
        let bare = to_a2a_request(&json!({"sessionId": "s3"}), "id").expect("ok");
        assert!(bare["message"].get("parts").is_none());
    }

    #[test]
    fn message_to_payload_extracts_text_and_metadata() {
        let msg = json!({
            "parts": [{"text": "hi"}, {"text": "ignored"}],
            "task_id": "t1",
            "context_id": "s1",
            "metadata": {"k": "v"}
        });
        let payload = message_to_payload(&msg);
        assert_eq!(payload["query"], "hi");
        assert_eq!(payload["task_id"], "t1");
        assert_eq!(payload["sessionId"], "s1");
        assert_eq!(payload["k"], "v");
    }

    #[test]
    fn resolve_session_id_priority() {
        assert_eq!(
            resolve_session_id(&json!({"conversation_id": "a", "sessionId": "b"})),
            Some("a".to_string())
        );
        assert_eq!(
            resolve_session_id(&json!({"sessionId": "b"})),
            Some("b".to_string())
        );
        assert_eq!(resolve_session_id(&json!({})), None);
    }

    #[test]
    fn part_roundtrip() {
        let part = A2aPartView {
            text: Some("hi".to_string()),
            raw: None,
            url: None,
            data: Some(json!({"a": 1})),
            filename: Some("f.txt".to_string()),
            media_type: Some("text/plain".to_string()),
            metadata: Map::new(),
        };
        let a2a = to_a2a_part(&part);
        assert_eq!(a2a["text"], "hi");
        assert_eq!(a2a["data"]["struct_value"]["a"], 1);
        assert_eq!(a2a["filename"], "f.txt");
        // 标量 data → string_value。
        let part2 = A2aPartView {
            data: Some(json!("raw-string")),
            ..part.clone()
        };
        let a2a2 = to_a2a_part(&part2);
        assert_eq!(a2a2["data"]["string_value"], "raw-string");
    }

    #[test]
    fn task_to_result_maps_status_and_artifacts() {
        let task = json!({
            "id": "t1",
            "context_id": "s1",
            "status": {"state": "TASK_STATE_COMPLETED"},
            "artifacts": [{"artifact_id": "a1", "parts": [{"text": "out"}]}],
            "metadata": {"m": 1}
        });
        let result = a2a_task_to_result(Some(&task));
        assert_eq!(result.task_id.as_deref(), Some("t1"));
        assert_eq!(result.session_id.as_deref(), Some("s1"));
        assert_eq!(result.status, Some(TaskStatus::Completed));
        assert_eq!(result.artifacts.len(), 1);
        assert_eq!(result.artifacts[0].parts[0].text.as_deref(), Some("out"));
        assert_eq!(result.metadata.get("m"), Some(&json!(1)));
    }

    #[test]
    fn merge_and_with_session() {
        let base = build_agent_result(
            None,
            None,
            None,
            vec![ArtifactView {
                artifact_id: Some("a1".to_string()),
                name: None,
                description: None,
                parts: vec![],
                metadata: Map::new(),
            }],
            Map::new(),
        );
        let update = build_agent_result(
            Some("t1".to_string()),
            None,
            Some(TaskStatus::Completed),
            vec![ArtifactView {
                artifact_id: Some("a2".to_string()),
                name: None,
                description: None,
                parts: vec![],
                metadata: Map::new(),
            }],
            Map::from_iter([("k".to_string(), json!("v"))]),
        );
        let merged = merge_agent_results(&base, &update);
        assert_eq!(merged.artifacts.len(), 2);
        assert_eq!(merged.status, Some(TaskStatus::Completed));
        assert_eq!(merged.metadata.get("k"), Some(&json!("v")));
        let with_session = with_session_id(merged, Some("s9".to_string()));
        assert_eq!(with_session.session_id.as_deref(), Some("s9"));
    }

    #[test]
    fn merge_unknown_status_keeps_base() {
        let base = build_agent_result(None, None, Some(TaskStatus::Working), vec![], Map::new());
        let update = build_agent_result(None, None, Some(TaskStatus::Unknown), vec![], Map::new());
        let merged = merge_agent_results(&base, &update);
        assert_eq!(merged.status, Some(TaskStatus::Working));
    }

    #[test]
    fn normalize_routes_and_interfaces() {
        assert_eq!(
            normalize_jsonrpc_route_path("/a2a/jsonrpc"),
            "/a2a/jsonrpc/"
        );
        assert_eq!(normalize_jsonrpc_route_path("a2a/jsonrpc"), "/a2a/jsonrpc/");
        assert_eq!(
            normalize_jsonrpc_route_path("/a2a/jsonrpc/"),
            "/a2a/jsonrpc/"
        );
        assert_eq!(
            normalize_jsonrpc_interface_url(Some("http://host:8000/a2a/jsonrpc")),
            Some("http://host:8000/a2a/jsonrpc/".to_string())
        );
        // 非 jsonrpc 路径不改。
        assert_eq!(
            normalize_jsonrpc_interface_url(Some("http://host:8000/a2a/rest")),
            Some("http://host:8000/a2a/rest".to_string())
        );
        assert_eq!(normalize_jsonrpc_interface_url(None), None);
    }

    #[test]
    fn transport_protocols_resolution() {
        let ifaces = vec![
            json!({"url": "u", "protocol_binding": "JSONRPC", "protocol_version": "1.0"}),
            json!({"url": "u2", "protocol_binding": "HTTP+JSON", "protocol_version": "1.0"}),
        ];
        let protocols = resolve_transport_protocols(&ifaces).expect("ok");
        assert!(protocols.contains(&"JSONRPC".to_string()));
        assert!(protocols.contains(&"HTTP+JSON".to_string()));
        // 空 → 默认 JSONRPC。
        assert_eq!(
            resolve_transport_protocols(&[]).expect("default"),
            vec!["JSONRPC".to_string()]
        );
        // gRPC → Err。
        let grpc = vec![json!({"protocol_binding": "GRPC"})];
        assert!(resolve_transport_protocols(&grpc).is_err());
    }

    #[test]
    fn agent_card_description_and_interfaces() {
        let card = to_a2a_agent_card(
            "agent",
            "My agent",
            Some(&json!({"type": "object", "properties": {}})),
            None,
            Some("http://host/a2a/jsonrpc/"),
            "JSONRPC",
            "1.0",
            None,
            &[],
        );
        assert_eq!(card["name"], "agent");
        assert!(
            card["description"]
                .as_str()
                .unwrap()
                .contains("[input_params]")
        );
        assert_eq!(card["capabilities"]["streaming"], true);
        assert_eq!(
            card["supported_interfaces"][0]["protocol_binding"],
            "JSONRPC"
        );
        // supported_interfaces 优先。
        let card2 = to_a2a_agent_card(
            "agent",
            "",
            None,
            None,
            None,
            "JSONRPC",
            "1.0",
            None,
            &[
                json!({"url": "u", "protocol_binding": "HTTP+JSON", "protocol_version": "1.0", "tenant": "t1"}),
            ],
        );
        assert_eq!(
            card2["supported_interfaces"][0]["protocol_binding"],
            "HTTP+JSON"
        );
        assert_eq!(card2["supported_interfaces"][0]["tenant"], "t1");
    }
}
