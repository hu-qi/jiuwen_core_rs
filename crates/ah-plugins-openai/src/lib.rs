//! # ah-plugins-openai
//!
//! 真实 OpenAI 兼容 HTTP 模型 provider(chat/completions 和 Responses API)。
//! 协议层移植自 agent-core_rs 的 OpenAiCompatibleClient;本 crate 是真实实现,
//! 无 mock 业务逻辑。集成测试用本地真实 HTTP 服务器验证真实协议路径。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use ah_contracts::credentials::CredentialProvider;
use ah_contracts::keys::{CREDENTIALS, LLM};
use ah_contracts::llm::{
    ChatMessage, ChatRole, ModelChunk, ModelError, ModelProvider, ModelRequest, ModelResponse,
    ToolCall, ToolCallDelta,
};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// provider 配置(配置字段,来自环境或调用方,不硬编码)。
#[derive(Clone, Debug)]
pub struct OpenAiConfig {
    /// 基础 URL,或完整 Responses endpoint,如 https://api.openai.com/v1 或 /responses。
    pub base_url: String,
    /// 可选 API key;缺失时按无认证请求。
    pub api_key: Option<String>,
    /// 默认模型名。
    pub model: String,
    /// 请求超时。
    pub timeout: Duration,
}

impl OpenAiConfig {
    /// 从环境变量构建:OPENAI_API_KEY(必须)、OPENAI_BASE_URL、OPENAI_MODEL;
    /// OPENAI_BASE_URL 以 `/responses` 结尾时使用 Responses API。
    /// key 缺失返回 None(调用方据此选择不挂载本插件)。
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("OPENAI_API_KEY").ok()?;
        Some(Self {
            base_url: std::env::var("OPENAI_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".to_string()),
            api_key: Some(api_key),
            model: std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string()),
            timeout: Duration::from_secs(60),
        })
    }

    /// 从 credentials seam + 环境变量解析(credentials 优先,环境变量兜底)。
    ///
    /// - openai.api_key:credentials 优先,fallback 环境变量 OPENAI_API_KEY
    ///   (两者都缺失时返回 None);
    /// - openai.base_url:credentials 优先,fallback OPENAI_BASE_URL,再 fallback 默认地址;
    ///   完整 `/responses` 地址会按 Responses API 处理;
    /// - openai.model:credentials 优先,fallback OPENAI_MODEL,再 fallback 默认模型。
    ///
    /// 环境变量读取逻辑与 [Self::from_env] 完全一致(向后兼容);
    /// credentials seam 是更优先的来源。
    pub fn from_env_with_credentials(credentials: Option<&dyn CredentialProvider>) -> Option<Self> {
        let api_key = credentials
            .and_then(|c| c.get("openai.api_key"))
            .map(|c| c.value)
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())?;
        Some(Self {
            base_url: credentials
                .and_then(|c| c.get("openai.base_url"))
                .map(|c| c.value)
                .or_else(|| std::env::var("OPENAI_BASE_URL").ok())
                .unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
            api_key: Some(api_key),
            model: credentials
                .and_then(|c| c.get("openai.model"))
                .map(|c| c.value)
                .or_else(|| std::env::var("OPENAI_MODEL").ok())
                .unwrap_or_else(|| "gpt-4o-mini".to_string()),
            timeout: Duration::from_secs(60),
        })
    }
}

// ------------------------------------------------------------------
// OpenAI wire 协议类型(仅本 crate 内部使用)
// ------------------------------------------------------------------

#[derive(Serialize)]
struct WireRequest {
    model: String,
    messages: Vec<WireMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<WireTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone)]
struct WireMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<WireToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_content: Option<String>,
}

#[derive(Serialize)]
struct WireTool {
    #[serde(rename = "type")]
    kind: String,
    function: WireFunction,
}

#[derive(Serialize)]
struct WireFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Serialize, Deserialize, Clone)]
struct WireToolCall {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    function: WireFunctionCall,
}

#[derive(Serialize, Deserialize, Clone)]
struct WireFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Deserialize)]
struct WireResponse {
    choices: Vec<WireChoice>,
}

#[derive(Deserialize)]
struct WireChoice {
    message: Option<WireMessage>,
}

#[derive(Serialize)]
struct ResponsesRequest {
    model: String,
    instructions: String,
    input: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
    include: Vec<String>,
    store: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

fn is_responses_endpoint(base_url: &str) -> bool {
    base_url.trim_end_matches('/').ends_with("/responses")
}

fn endpoint_url(base_url: &str) -> String {
    let base_url = base_url.trim_end_matches('/');
    if is_responses_endpoint(base_url) || base_url.ends_with("/chat/completions") {
        base_url.to_string()
    } else {
        format!("{base_url}/chat/completions")
    }
}

fn responses_instructions(request: &ModelRequest) -> String {
    request
        .messages
        .iter()
        .filter(|message| message.role == ChatRole::System && !message.content.is_empty())
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn responses_input(request: &ModelRequest) -> Vec<serde_json::Value> {
    request
        .messages
        .iter()
        .flat_map(|message| match message.role {
            ChatRole::System => Vec::new(),
            ChatRole::User => {
                let mut content = Vec::new();
                if !message.content.is_empty() || message.images.is_empty() {
                    content.push(json!({"type": "input_text", "text": message.content}));
                }
                content.extend(
                    message
                        .images
                        .iter()
                        .map(|image| json!({"type": "input_image", "image_url": image.data_url()})),
                );
                vec![json!({"role": "user", "content": content})]
            }
            ChatRole::Assistant => {
                let mut items = Vec::new();
                if !message.content.is_empty() {
                    items.push(json!({
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": message.content}]
                    }));
                }
                if let Some(calls) = &message.tool_calls {
                    items.extend(calls.iter().map(|call| {
                        json!({
                            "type": "function_call",
                            "id": call.id,
                            "call_id": call.id,
                            "name": call.name,
                            "arguments": call.arguments.to_string(),
                        })
                    }));
                }
                items
            }
            ChatRole::Tool => vec![json!({
                "type": "function_call_output",
                "call_id": message.tool_call_id,
                "output": message.content,
            })],
        })
        .collect()
}

fn responses_tools(request: &ModelRequest) -> Option<Vec<serde_json::Value>> {
    if request.tools.is_empty() {
        None
    } else {
        Some(
            request
                .tools
                .iter()
                .map(|schema| {
                    json!({
                        "type": "function",
                        "name": schema.name,
                        "description": schema.description,
                        "parameters": schema.parameters,
                    })
                })
                .collect(),
        )
    }
}

fn parse_responses_response(value: serde_json::Value) -> Result<ModelResponse, ModelError> {
    let mut content = value
        .get("output_text")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut tool_calls = Vec::new();
    for item in value
        .get("output")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        match item.get("type").and_then(serde_json::Value::as_str) {
            Some("message") => {
                if let Some(blocks) = item.get("content").and_then(serde_json::Value::as_array) {
                    for block in blocks {
                        if let Some(text) = block.get("text").and_then(serde_json::Value::as_str) {
                            content.push_str(text);
                        }
                    }
                }
            }
            Some("function_call") => {
                let id = item
                    .get("call_id")
                    .or_else(|| item.get("id"))
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| ModelError("responses function call has no call_id".into()))?;
                let name = item
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| ModelError("responses function call has no name".into()))?;
                let raw_arguments = item
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| serde_json::Value::String("{}".into()));
                let arguments = match raw_arguments {
                    serde_json::Value::String(raw) => {
                        serde_json::from_str(&raw).map_err(|error| {
                            ModelError(format!("invalid function arguments: {error}"))
                        })?
                    }
                    value => value,
                };
                tool_calls.push(ToolCall {
                    id: id.to_string(),
                    name: name.to_string(),
                    arguments,
                });
            }
            _ => {}
        }
    }
    Ok(ModelResponse {
        content,
        tool_calls,
        reasoning_content: None,
    })
}

fn wire_role(role: ChatRole) -> &'static str {
    match role {
        ChatRole::System => "system",
        ChatRole::User => "user",
        ChatRole::Assistant => "assistant",
        ChatRole::Tool => "tool",
    }
}

fn wire_content(message: &ChatMessage) -> Option<serde_json::Value> {
    if message.role == ChatRole::Assistant && message.tool_calls.is_some() {
        return None;
    }
    if message.images.is_empty() {
        return Some(json!(message.content));
    }
    let mut parts = Vec::new();
    if !message.content.is_empty() {
        parts.push(json!({"type": "text", "text": message.content}));
    }
    parts.extend(message.images.iter().map(|image| {
        json!({
            "type": "image_url",
            "image_url": {"url": image.data_url()}
        })
    }));
    Some(json!(parts))
}

/// 真实 OpenAI 兼容 provider:通过 HTTP 调用 chat/completions。
pub struct OpenAiModelProvider {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    model: String,
    timeout: Duration,
}

impl OpenAiModelProvider {
    /// 构建真实 provider。
    pub fn new(config: OpenAiConfig) -> Result<Self, ModelError> {
        let http = reqwest::Client::builder()
            .http1_only()
            .timeout(config.timeout)
            .build()
            .map_err(|e| ModelError(format!("http client build failed: {e}")))?;
        Ok(Self {
            http,
            base_url: config.base_url,
            api_key: config.api_key,
            model: config.model,
            timeout: config.timeout,
        })
    }

    fn request_model(&self, request: &ModelRequest) -> String {
        request.model.clone().unwrap_or_else(|| self.model.clone())
    }

    fn responses_request(&self, request: &ModelRequest, stream: bool) -> ResponsesRequest {
        let tools = responses_tools(request);
        ResponsesRequest {
            model: self.request_model(request),
            instructions: responses_instructions(request),
            input: responses_input(request),
            parallel_tool_calls: tools.as_ref().map(|_| true),
            tool_choice: tools.as_ref().map(|_| "auto".to_string()),
            tools,
            temperature: request.temperature,
            include: vec!["reasoning.encrypted_content".to_string()],
            store: false,
            stream: Some(stream),
        }
    }

    async fn chat_responses(&self, request: ModelRequest) -> Result<ModelResponse, ModelError> {
        let mut builder = self
            .http
            .post(endpoint_url(&self.base_url))
            .json(&self.responses_request(&request, false))
            .timeout(self.timeout);
        if let Some(key) = &self.api_key {
            builder = builder.bearer_auth(key);
        }
        let response = builder
            .send()
            .await
            .map_err(|e| ModelError(format!("http request failed: {e}")))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| ModelError(format!("read response body failed: {e}")))?;
        if !status.is_success() {
            return Err(ModelError(format!("provider returned {status}: {body}")));
        }
        let value: serde_json::Value = serde_json::from_str(&body)
            .map_err(|e| ModelError(format!("invalid responses provider response: {e}")))?;
        parse_responses_response(value)
    }

    async fn stream_responses(
        &self,
        request: ModelRequest,
        sink: tokio::sync::mpsc::Sender<ModelChunk>,
    ) -> Result<(), ModelError> {
        let mut builder = self
            .http
            .post(endpoint_url(&self.base_url))
            .json(&self.responses_request(&request, true))
            .timeout(self.timeout)
            .header("accept", "text/event-stream");
        if let Some(key) = &self.api_key {
            builder = builder.bearer_auth(key);
        }
        let response = builder
            .send()
            .await
            .map_err(|e| ModelError(format!("http request failed: {e}")))?;
        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .map_err(|e| ModelError(format!("read error body failed: {e}")))?;
            return Err(ModelError(format!("provider returned {status}: {body}")));
        }

        let mut response = response;
        let mut buffer = String::new();
        let mut event_name = None;
        let mut item_indexes = HashMap::<String, usize>::new();
        let mut call_metadata = HashMap::<usize, (Option<String>, Option<String>)>::new();
        let mut call_arguments_started = HashMap::<usize, bool>::new();
        let mut next_index = 0usize;
        while let Some(bytes) = response
            .chunk()
            .await
            .map_err(|e| ModelError(format!("stream read failed: {e}")))?
        {
            buffer.push_str(&String::from_utf8_lossy(&bytes));
            let mut lines = buffer.split('\n').map(str::to_string).collect::<Vec<_>>();
            buffer = lines.pop().unwrap_or_default();
            for line in lines {
                let line = line.trim();
                if let Some(name) = line.strip_prefix("event:") {
                    event_name = Some(name.trim().to_string());
                    continue;
                }
                let Some(payload) = line.strip_prefix("data:").map(str::trim) else {
                    continue;
                };
                if payload.is_empty() {
                    continue;
                }
                if payload == "[DONE]" {
                    sink.send(ModelChunk {
                        done: true,
                        ..Default::default()
                    })
                    .await
                    .map_err(|_| ModelError("stream sink closed".into()))?;
                    return Ok(());
                }
                let value: serde_json::Value = serde_json::from_str(payload)
                    .map_err(|e| ModelError(format!("invalid SSE chunk: {e}")))?;
                let kind = value
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
                    .or_else(|| event_name.take())
                    .unwrap_or_default();
                if kind == "response.completed" {
                    sink.send(ModelChunk {
                        done: true,
                        ..Default::default()
                    })
                    .await
                    .map_err(|_| ModelError("stream sink closed".into()))?;
                    return Ok(());
                }
                let mut chunk = ModelChunk::default();
                match kind.as_str() {
                    "response.output_text.delta" => {
                        chunk.content_delta = value
                            .get("delta")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                    }
                    "response.output_item.added" => {
                        let Some(item) = value.get("item") else {
                            continue;
                        };
                        if item.get("type").and_then(serde_json::Value::as_str)
                            != Some("function_call")
                        {
                            continue;
                        }
                        let index = value
                            .get("output_index")
                            .and_then(serde_json::Value::as_u64)
                            .map(|index| index as usize)
                            .unwrap_or_else(|| {
                                let index = next_index;
                                next_index += 1;
                                index
                            });
                        if let Some(item_id) = item.get("id").and_then(serde_json::Value::as_str) {
                            item_indexes.insert(item_id.to_string(), index);
                        }
                        let id = item
                            .get("call_id")
                            .or_else(|| item.get("id"))
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string);
                        let name = item
                            .get("name")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string);
                        call_metadata.insert(index, (id.clone(), name.clone()));
                        chunk.tool_call_deltas.push(ToolCallDelta {
                            index,
                            id,
                            name,
                            arguments: item
                                .get("arguments")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                        });
                    }
                    "response.function_call_arguments.delta" => {
                        let index = value
                            .get("output_index")
                            .and_then(serde_json::Value::as_u64)
                            .map(|index| index as usize)
                            .or_else(|| {
                                value
                                    .get("item_id")
                                    .and_then(serde_json::Value::as_str)
                                    .and_then(|item_id| item_indexes.get(item_id).copied())
                            })
                            .unwrap_or(0);
                        let (id, name) = call_metadata.get(&index).cloned().unwrap_or_default();
                        call_arguments_started.insert(index, true);
                        chunk.tool_call_deltas.push(ToolCallDelta {
                            index,
                            id,
                            name,
                            arguments: value
                                .get("delta")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                        });
                    }
                    "response.function_call_arguments.done" => {
                        let index = value
                            .get("output_index")
                            .and_then(serde_json::Value::as_u64)
                            .map(|index| index as usize)
                            .or_else(|| {
                                value
                                    .get("item_id")
                                    .and_then(serde_json::Value::as_str)
                                    .and_then(|item_id| item_indexes.get(item_id).copied())
                            })
                            .unwrap_or(0);
                        if !call_arguments_started.contains_key(&index) {
                            let (id, name) = call_metadata.get(&index).cloned().unwrap_or_default();
                            chunk.tool_call_deltas.push(ToolCallDelta {
                                index,
                                id,
                                name,
                                arguments: value
                                    .get("arguments")
                                    .and_then(serde_json::Value::as_str)
                                    .unwrap_or_default()
                                    .to_string(),
                            });
                        }
                    }
                    _ => continue,
                }
                if !chunk.content_delta.is_empty()
                    || !chunk.reasoning_delta.is_empty()
                    || !chunk.tool_call_deltas.is_empty()
                {
                    sink.send(chunk)
                        .await
                        .map_err(|_| ModelError("stream sink closed".into()))?;
                }
            }
        }
        sink.send(ModelChunk {
            done: true,
            ..Default::default()
        })
        .await
        .map_err(|_| ModelError("stream sink closed".into()))?;
        Ok(())
    }
}
impl Seam for OpenAiModelProvider {}

#[async_trait]
impl ModelProvider for OpenAiModelProvider {
    fn name(&self) -> &'static str {
        "openai-compatible"
    }

    async fn chat(&self, request: ModelRequest) -> Result<ModelResponse, ModelError> {
        if is_responses_endpoint(&self.base_url) {
            return self.chat_responses(request).await;
        }
        let wire = WireRequest {
            model: self.request_model(&request),
            messages: request
                .messages
                .iter()
                .map(|message: &ChatMessage| WireMessage {
                    role: wire_role(message.role).to_string(),
                    content: wire_content(message),
                    tool_call_id: message.tool_call_id.clone(),
                    reasoning_content: message.reasoning_content.clone(),
                    tool_calls: message.tool_calls.as_ref().map(|calls| {
                        calls
                            .iter()
                            .map(|call| WireToolCall {
                                id: call.id.clone(),
                                kind: "function".to_string(),
                                function: WireFunctionCall {
                                    name: call.name.clone(),
                                    arguments: call.arguments.to_string(),
                                },
                            })
                            .collect()
                    }),
                })
                .collect(),
            tools: if request.tools.is_empty() {
                None
            } else {
                Some(
                    request
                        .tools
                        .iter()
                        .map(|schema| WireTool {
                            kind: "function".to_string(),
                            function: WireFunction {
                                name: schema.name.clone(),
                                description: schema.description.clone(),
                                parameters: schema.parameters.clone(),
                            },
                        })
                        .collect(),
                )
            },
            temperature: request.temperature,
            stream: None,
        };

        let url = endpoint_url(&self.base_url);
        let mut builder = self.http.post(&url).json(&wire).timeout(self.timeout);
        if let Some(key) = &self.api_key {
            builder = builder.bearer_auth(key);
        }

        let response = builder
            .send()
            .await
            .map_err(|e| ModelError(format!("http request failed: {e}")))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| ModelError(format!("read response body failed: {e}")))?;

        if !status.is_success() {
            return Err(ModelError(format!("provider returned {status}: {body}")));
        }

        let parsed: WireResponse = serde_json::from_str(&body)
            .map_err(|e| ModelError(format!("invalid provider response: {e}")))?;
        let message = parsed.choices.into_iter().next().and_then(|c| c.message);
        let content = message
            .as_ref()
            .and_then(|m| m.content.as_ref())
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        let tool_calls = message
            .as_ref()
            .and_then(|m| m.tool_calls.clone())
            .map(|calls| {
                calls
                    .into_iter()
                    .map(|call| ToolCall {
                        id: call.id,
                        name: call.function.name,
                        arguments: serde_json::from_str(&call.function.arguments)
                            .unwrap_or(serde_json::Value::Null),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let reasoning_content = message
            .as_ref()
            .and_then(|message| message.reasoning_content.clone());
        Ok(ModelResponse {
            content,
            tool_calls,
            reasoning_content,
        })
    }

    /// 流式对话:真实 SSE 解析(data: 行,delta.content / delta.tool_calls)。
    async fn stream_chat(
        &self,
        request: ModelRequest,
        sink: tokio::sync::mpsc::Sender<ModelChunk>,
    ) -> Result<(), ModelError> {
        if is_responses_endpoint(&self.base_url) {
            return self.stream_responses(request, sink).await;
        }
        let wire = WireRequest {
            model: self.request_model(&request),
            messages: request
                .messages
                .iter()
                .map(|message: &ChatMessage| WireMessage {
                    role: wire_role(message.role).to_string(),
                    content: wire_content(message),
                    tool_call_id: message.tool_call_id.clone(),
                    reasoning_content: message.reasoning_content.clone(),
                    tool_calls: message.tool_calls.as_ref().map(|calls| {
                        calls
                            .iter()
                            .map(|call| WireToolCall {
                                id: call.id.clone(),
                                kind: "function".to_string(),
                                function: WireFunctionCall {
                                    name: call.name.clone(),
                                    arguments: call.arguments.to_string(),
                                },
                            })
                            .collect()
                    }),
                })
                .collect(),
            tools: if request.tools.is_empty() {
                None
            } else {
                Some(
                    request
                        .tools
                        .iter()
                        .map(|schema| WireTool {
                            kind: "function".to_string(),
                            function: WireFunction {
                                name: schema.name.clone(),
                                description: schema.description.clone(),
                                parameters: schema.parameters.clone(),
                            },
                        })
                        .collect(),
                )
            },
            temperature: request.temperature,
            stream: Some(true),
        };

        let url = endpoint_url(&self.base_url);
        let mut builder = self.http.post(&url).json(&wire).timeout(self.timeout);
        builder = builder.header("accept", "text/event-stream");
        if let Some(key) = &self.api_key {
            builder = builder.bearer_auth(key);
        }
        let response = builder
            .send()
            .await
            .map_err(|e| ModelError(format!("http request failed: {e}")))?;
        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .map_err(|e| ModelError(format!("read error body failed: {e}")))?;
            return Err(ModelError(format!("provider returned {status}: {body}")));
        }

        // SSE:逐行解析 data: JSON,遇 [DONE] 结束;增量经 sink 推送。
        // 用 response.chunk() 逐块读取(不引入 futures-util,镜像缺 futures-io)。
        let mut response = response;
        let mut buffer = String::new();
        while let Some(bytes) = response
            .chunk()
            .await
            .map_err(|e| ModelError(format!("stream read failed: {e}")))?
        {
            buffer.push_str(&String::from_utf8_lossy(&bytes));
            // 按行切分(SSE 事件以 \n 分隔)。
            let mut lines: Vec<String> = Vec::new();
            for line in buffer.split('\n') {
                lines.push(line.to_string());
            }
            buffer = lines.pop().unwrap_or_default();
            for line in lines {
                let line = line.trim();
                if line == "data: [DONE]" {
                    let _ = sink
                        .send(ModelChunk {
                            done: true,
                            ..Default::default()
                        })
                        .await;
                    return Ok(());
                }
                let Some(payload) = line.strip_prefix("data:") else {
                    continue;
                };
                let payload = payload.trim();
                if payload.is_empty() {
                    continue;
                }
                let value: serde_json::Value = serde_json::from_str(payload)
                    .map_err(|e| ModelError(format!("invalid SSE chunk: {e}")))?;
                let mut chunk = ModelChunk::default();
                if let Some(delta) = value
                    .get("choices")
                    .and_then(|c| c.get(0))
                    .and_then(|c| c.get("delta"))
                {
                    if let Some(text) = delta.get("content").and_then(serde_json::Value::as_str) {
                        chunk.content_delta = text.to_string();
                    }
                    if let Some(reasoning) = delta
                        .get("reasoning_content")
                        .and_then(serde_json::Value::as_str)
                    {
                        chunk.reasoning_delta = reasoning.to_string();
                    }
                    if let Some(calls) = delta
                        .get("tool_calls")
                        .and_then(serde_json::Value::as_array)
                    {
                        for call in calls {
                            let index = call
                                .get("index")
                                .and_then(serde_json::Value::as_u64)
                                .unwrap_or(0) as usize;
                            let id = call
                                .get("id")
                                .and_then(serde_json::Value::as_str)
                                .map(str::to_string);
                            let function = call.get("function");
                            let name = function
                                .and_then(|f| f.get("name"))
                                .and_then(serde_json::Value::as_str)
                                .map(str::to_string);
                            let arguments = function
                                .and_then(|f| f.get("arguments"))
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or_default()
                                .to_string();
                            chunk.tool_call_deltas.push(ToolCallDelta {
                                index,
                                id,
                                name,
                                arguments,
                            });
                        }
                    }
                }
                let _ = sink.send(chunk).await;
            }
        }
        let _ = sink
            .send(ModelChunk {
                done: true,
                ..Default::default()
            })
            .await;
        Ok(())
    }
}

/// 真实 LLM provider 插件:注册到 llm seam。
///
/// 配置来源:
/// - [OpenAiPlugin::new][]:显式固定配置(调用方自行解析);
/// - [OpenAiPlugin::from_env][]:仅环境变量(OPENAI_API_KEY 等,向后兼容);
/// - [OpenAiPlugin::from_env_with_credentials][]:构造时从 credentials seam
///   + 环境变量解析(credentials 优先);
/// - [OpenAiPlugin::lazy][]:apply 时先查 credentials seam,再 fallback 环境变量;
///   两者都无 key 时 apply 显式失败(不静默降级)。
pub struct OpenAiPlugin {
    /// None = apply 时从 credentials seam + 环境变量解析。
    config: Option<OpenAiConfig>,
}

impl OpenAiPlugin {
    /// 以显式配置构建。
    pub fn new(config: OpenAiConfig) -> Self {
        Self {
            config: Some(config),
        }
    }

    /// 从环境变量构建;无 OPENAI_API_KEY 时返回 None(向后兼容)。
    pub fn from_env() -> Option<Self> {
        OpenAiConfig::from_env().map(|config| Self {
            config: Some(config),
        })
    }

    /// 从 credentials seam + 环境变量解析(credentials 优先);
    /// 需要 caller 提供已挂载 credentials 的 Context。
    /// 两者都无 key 时返回 None。
    pub fn from_env_with_credentials(ctx: &Context) -> Option<Self> {
        let credentials = ctx.service::<dyn CredentialProvider>(&CREDENTIALS);
        OpenAiConfig::from_env_with_credentials(credentials.as_deref()).map(|config| Self {
            config: Some(config),
        })
    }

    /// 惰性解析:apply 时先查 credentials seam,再 fallback 到环境变量。
    /// 无可用 key 时 apply 显式失败(不静默降级)。
    pub fn lazy() -> Self {
        Self { config: None }
    }
}

impl Plugin for OpenAiPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-openai"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![LLM]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let config = match &self.config {
            Some(config) => config.clone(),
            None => {
                // 惰性解析:credentials seam 优先,环境变量兜底;都无 key 则显式失败。
                let credentials = ctx.service::<dyn CredentialProvider>(&CREDENTIALS);
                OpenAiConfig::from_env_with_credentials(credentials.as_deref()).ok_or_else(|| {
                    PluginError::Apply {
                        plugin: self.name(),
                        message: "no API key: credentials seam (openai.api_key) and OPENAI_API_KEY env both unavailable"
                            .to_string(),
                    }
                })?
            }
        };
        let provider = OpenAiModelProvider::new(config).map_err(|e| PluginError::Apply {
            plugin: self.name(),
            message: e.0,
        })?;
        Ok(vec![ctx.register(
            LLM,
            Arc::new(provider) as Arc<dyn ModelProvider>,
        )])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::llm::ModelRequest;
    use ah_hub::plugin::DynPlugin;

    /// 启动一个真实本地 HTTP 服务器,按 OpenAI chat/completions 协议应答。
    /// 请求体中包含 "fail" 时返回 400,否则返回固定成功响应。
    fn start_test_server() -> String {
        let server = tiny_http::Server::http("127.0.0.1:0").expect("bind test server");
        let addr = server.server_addr().to_string();
        let base_url = format!("http://{addr}");
        let _handle = std::thread::spawn(move || {
            for mut request in server.incoming_requests() {
                let mut body = String::new();
                let _ = request.as_reader().read_to_string(&mut body);
                let response = if body.contains("fail") {
                    tiny_http::Response::from_string(r#"{"error":{"message":"boom"}}"#)
                        .with_status_code(400)
                } else {
                    tiny_http::Response::from_string(
                        r#"{"id":"chatcmpl-test","object":"chat.completion","created":0,"model":"test-model","choices":[{"index":0,"message":{"role":"assistant","content":"hello from test server"},"finish_reason":"stop"}],"usage":null}"#,
                    )
                };
                let _ = request.respond(response);
            }
        });
        // 服务器线程常驻到进程退出;不 join(其循环永不结束)。
        base_url
    }

    fn config(base_url: String) -> OpenAiConfig {
        OpenAiConfig {
            base_url,
            api_key: Some("test-key".to_string()),
            model: "test-model".to_string(),
            timeout: Duration::from_secs(10),
        }
    }

    /// SSE 测试服务器:流式输出三个 data 块 + [DONE]。
    /// 请求体含 "tool" 时输出 tool_calls delta,否则输出 content delta。
    fn start_sse_server() -> String {
        let server = tiny_http::Server::http("127.0.0.1:0").expect("bind sse server");
        let addr = server.server_addr().to_string();
        let base_url = format!("http://{addr}");
        let _handle = std::thread::spawn(move || {
            for mut request in server.incoming_requests() {
                let mut body = String::new();
                let _ = request.as_reader().read_to_string(&mut body);
                let payload = if body.contains("tool") {
                    // 用 serde_json 构造,避免手写转义;arguments 为 JSON 字符串(增量拼接)。
                    let part1 = serde_json::json!({
                        "id": "c1",
                        "object": "chat.completion.chunk",
                        "choices": [{
                            "index": 0,
                            "delta": {
                                "tool_calls": [{
                                    "index": 0,
                                    "id": "call_1",
                                    "type": "function",
                                    "function": { "name": "read_file", "arguments": "{\"path\":\"a" }
                                }]
                            },
                            "finish_reason": null
                        }]
                    });
                    let part2 = serde_json::json!({
                        "id": "c1",
                        "object": "chat.completion.chunk",
                        "choices": [{
                            "index": 0,
                            "delta": {
                                "tool_calls": [{
                                    "index": 0,
                                    "function": { "arguments": ".txt\"}" }
                                }]
                            },
                            "finish_reason": null
                        }]
                    });
                    let part3 = serde_json::json!({
                        "id": "c1",
                        "object": "chat.completion.chunk",
                        "choices": [{
                            "index": 0,
                            "delta": {},
                            "finish_reason": "tool_calls"
                        }]
                    });
                    [
                        format!("data: {part1}"),
                        format!("data: {part2}"),
                        format!("data: {part3}"),
                        "data: [DONE]".to_string(),
                    ]
                } else {
                    let part1 = serde_json::json!({
                        "id": "c1",
                        "object": "chat.completion.chunk",
                        "choices": [{
                            "index": 0,
                            "delta": {
                                "role": "assistant",
                                "content": "hello",
                                "reasoning_content": "think "
                            },
                            "finish_reason": null
                        }]
                    });
                    let part2 = serde_json::json!({
                        "id": "c1",
                        "object": "chat.completion.chunk",
                        "choices": [{
                            "index": 0,
                            "delta": { "content": " world" },
                            "finish_reason": null
                        }]
                    });
                    let part3 = serde_json::json!({
                        "id": "c1",
                        "object": "chat.completion.chunk",
                        "choices": [{
                            "index": 0,
                            "delta": {},
                            "finish_reason": "stop"
                        }]
                    });
                    [
                        format!("data: {part1}"),
                        format!("data: {part2}"),
                        format!("data: {part3}"),
                        "data: [DONE]".to_string(),
                    ]
                }
                .join("\n");
                let response = tiny_http::Response::from_string(payload).with_header(
                    tiny_http::Header::from_bytes("content-type", "text/event-stream")
                        .expect("header"),
                );
                let _ = request.respond(response);
            }
        });
        base_url
    }

    #[tokio::test]
    async fn stream_chat_accumulates_content_deltas_from_sse() {
        use ah_contracts::llm::ModelChunk;
        let base_url = start_sse_server();
        let provider = OpenAiModelProvider::new(config(base_url)).expect("provider");
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);

        provider
            .stream_chat(
                ModelRequest {
                    messages: vec![ChatMessage::new(ChatRole::User, "hi")],
                    ..Default::default()
                },
                tx,
            )
            .await
            .expect("stream chat");

        let mut text = String::new();
        let mut reasoning = String::new();
        let mut seen_done = false;
        while let Some(chunk) = rx.recv().await {
            text.push_str(&chunk.content_delta);
            reasoning.push_str(&chunk.reasoning_delta);
            if chunk.done {
                seen_done = true;
            }
        }
        assert_eq!(
            text, "hello world",
            "SSE content deltas accumulated: {text:?}"
        );
        assert_eq!(reasoning, "think ");
        assert!(seen_done, "done chunk sent");
        let _ = ModelChunk::default();
    }

    #[tokio::test]
    async fn stream_chat_accumulates_tool_call_argument_deltas() {
        let base_url = start_sse_server();
        let provider = OpenAiModelProvider::new(config(base_url)).expect("provider");
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);

        provider
            .stream_chat(
                ModelRequest {
                    messages: vec![ChatMessage::new(ChatRole::User, "use the tool")],
                    ..Default::default()
                },
                tx,
            )
            .await
            .expect("stream chat");

        let mut name = String::new();
        let mut arguments = String::new();
        let mut id = None;
        while let Some(chunk) = rx.recv().await {
            for delta in chunk.tool_call_deltas {
                if let Some(n) = delta.name {
                    name = n;
                }
                if let Some(delta_id) = delta.id {
                    id = Some(delta_id);
                }
                arguments.push_str(&delta.arguments);
            }
        }
        assert_eq!(name, "read_file");
        assert_eq!(id.as_deref(), Some("call_1"));
        let parsed: serde_json::Value = serde_json::from_str(&arguments).expect("valid json");
        assert_eq!(
            parsed["path"], "a.txt",
            "tool call arguments accumulate to valid JSON"
        );
    }

    #[tokio::test]
    async fn chat_returns_content_from_real_http_roundtrip() {
        let base_url = start_test_server();
        let provider = OpenAiModelProvider::new(config(base_url)).expect("provider");
        let response = provider
            .chat(ModelRequest {
                messages: vec![ChatMessage::new(ChatRole::User, "hi")],
                ..Default::default()
            })
            .await
            .expect("chat");
        assert_eq!(response.content, "hello from test server");
    }

    #[tokio::test]
    async fn chat_maps_http_error_status() {
        let base_url = start_test_server();
        let provider = OpenAiModelProvider::new(config(base_url)).expect("provider");
        let error = provider
            .chat(ModelRequest {
                messages: vec![ChatMessage::new(ChatRole::User, "fail")],
                ..Default::default()
            })
            .await
            .expect_err("should error");
        assert!(error.0.contains("400"));
    }

    #[tokio::test]
    async fn plugin_registers_real_provider_on_llm_seam() {
        let base_url = start_test_server();
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(OpenAiPlugin::new(config(base_url)));
        let effects = ctx.mount(&plugin).expect("mount");

        let provider: Arc<dyn ModelProvider> =
            ctx.service(&LLM).expect("llm seam should be registered");
        assert_eq!(provider.name(), "openai-compatible");
        let response = provider
            .chat(ModelRequest {
                messages: vec![ChatMessage::new(ChatRole::User, "hi")],
                ..Default::default()
            })
            .await
            .expect("chat");
        assert_eq!(response.content, "hello from test server");

        drop(effects);
        assert!(!ctx.has_service(&LLM));
    }

    #[test]
    fn tool_call_messages_omit_empty_content() {
        let message = ChatMessage::assistant_with_tool_calls(vec![ToolCall {
            id: "call-1".into(),
            name: "list_dir".into(),
            arguments: json!({"path": "."}),
        }]);
        assert_eq!(wire_content(&message), None);
        assert_eq!(
            wire_content(&ChatMessage::new(ChatRole::User, "hi")),
            Some(json!("hi"))
        );
    }

    #[test]
    fn chat_completion_message_serializes_image_url_parts() {
        let message = ChatMessage::user_with_image(
            "inspect",
            ah_contracts::llm::ChatImage::new("image/png", "data:image/png;base64,AAAA"),
        );
        assert_eq!(
            wire_content(&message),
            Some(json!([
                {"type": "text", "text": "inspect"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAAA"}}
            ]))
        );
    }

    #[test]
    fn responses_endpoint_keeps_explicit_path() {
        assert_eq!(
            endpoint_url("http://159.138.247.92/responses"),
            "http://159.138.247.92/responses"
        );
        assert_eq!(
            endpoint_url("https://api.openai.com/v1"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            endpoint_url("https://api.deepseek.com/chat/completions"),
            "https://api.deepseek.com/chat/completions"
        );
    }

    #[test]
    fn parses_responses_api_message_and_function_call() {
        let response = parse_responses_response(serde_json::json!({
            "output": [
                {
                    "type": "message",
                    "content": [{"type": "output_text", "text": "hello"}]
                },
                {
                    "type": "function_call",
                    "call_id": "call_1",
                    "name": "read_file",
                    "arguments": "{\"path\":\"a.txt\"}"
                }
            ]
        }))
        .expect("responses payload");
        assert_eq!(response.content, "hello");
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].id, "call_1");
        assert_eq!(response.tool_calls[0].name, "read_file");
        assert_eq!(response.tool_calls[0].arguments["path"], "a.txt");
    }
    #[test]
    fn responses_request_matches_openai_input_contract() {
        let mut assistant = ChatMessage::new(ChatRole::Assistant, "previous answer");
        assistant.tool_calls = Some(vec![ToolCall {
            id: "call_1".into(),
            name: "read_file".into(),
            arguments: json!({"path": "a.txt"}),
        }]);
        let request = ModelRequest {
            messages: vec![
                ChatMessage::new(ChatRole::System, "You are concise."),
                ChatMessage::new(ChatRole::User, "Use the tool."),
                assistant,
                ChatMessage::tool("call_1", "result text"),
            ],
            tools: vec![ah_contracts::llm::ToolSchema {
                name: "read_file".into(),
                description: "Read a file.".into(),
                parameters: json!({"type": "object"}),
            }],
            ..Default::default()
        };
        let provider =
            OpenAiModelProvider::new(config("https://api.openai.com/v1/responses".into()))
                .expect("provider");
        let body = serde_json::to_value(provider.responses_request(&request, false))
            .expect("request body");

        assert_eq!(body["instructions"], "You are concise.");
        assert_eq!(body["input"][0]["role"], "user");
        assert_eq!(body["input"][1]["role"], "assistant");
        assert_eq!(body["input"][2]["type"], "function_call");
        assert_eq!(body["input"][2]["id"], "call_1");
        assert_eq!(body["input"][2]["call_id"], "call_1");

        assert_eq!(
            body["input"][3],
            json!({
                "type": "function_call_output",
                "call_id": "call_1",
                "output": "result text"
            })
        );
        assert_eq!(body["tools"][0]["name"], "read_file");
        assert_eq!(body["parallel_tool_calls"], true);
        assert_eq!(body["tool_choice"], "auto");
        assert_eq!(body["store"], false);
        assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
        assert_eq!(body["stream"], false);
    }

    #[test]
    fn responses_input_serializes_image_input() {
        let request = ModelRequest {
            messages: vec![ChatMessage::user_with_image(
                "screen",
                ah_contracts::llm::ChatImage::new("image/png", "data:image/png;base64,AAAA"),
            )],
            ..Default::default()
        };
        let input = responses_input(&request);
        assert_eq!(
            input[0]["content"][0],
            json!({"type":"input_text","text":"screen"})
        );
        assert_eq!(
            input[0]["content"][1],
            json!({
                "type":"input_image",
                "image_url":"data:image/png;base64,AAAA"
            })
        );
    }

    // ------------------------------------------------------------------
    // credentials 集成:真实环境变量 → credentials seam → OpenAiConfig
    // ------------------------------------------------------------------

    use ah_plugins_credentials::EnvCredentialProvider;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// 串行化环境变量修改(edition 2024 下 set_var 为 unsafe;
    /// 全部 env 测试经本锁串行,避免并行测试互相污染)。
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// 环境变量设置的 RAII guard:构造时设置,drop 时恢复原值。
    /// vars 中 value 为 None 表示删除该环境变量。
    struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        saved: Vec<(String, Option<String>)>,
    }

    fn env_guard(vars: &[(&str, Option<&str>)]) -> EnvGuard {
        let _lock = ENV_LOCK.lock().expect("env lock poisoned");
        let saved: Vec<(String, Option<String>)> = vars
            .iter()
            .map(|(name, _)| ((*name).to_string(), std::env::var(name).ok()))
            .collect();
        for (name, value) in vars {
            match value {
                Some(value) => {
                    // SAFETY:ENV_LOCK 保证同进程内唯一修改者;guard drop 时恢复。
                    unsafe { std::env::set_var(name, value) };
                }
                None => {
                    // SAFETY:同上。
                    unsafe { std::env::remove_var(name) };
                }
            }
        }
        EnvGuard { _lock, saved }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (name, previous) in &self.saved {
                match previous {
                    Some(value) => {
                        // SAFETY:guard 仍持有 ENV_LOCK,是唯一修改者。
                        unsafe { std::env::set_var(name, value) };
                    }
                    None => {
                        // SAFETY:同上。
                        unsafe { std::env::remove_var(name) };
                    }
                }
            }
        }
    }

    /// openai.api_key → 给定 env 名的单条目映射 provider。
    fn key_mapping_provider(env_var: &str) -> EnvCredentialProvider {
        let mut mapping = HashMap::new();
        mapping.insert("openai.api_key".to_string(), env_var.to_string());
        EnvCredentialProvider::new(mapping)
    }

    #[test]
    fn config_from_env_with_credentials_falls_back_to_env() {
        // 无 credentials 时与 from_env 完全一致(向后兼容)。
        let _guard = env_guard(&[
            ("OPENAI_API_KEY", Some("env-key")),
            ("OPENAI_BASE_URL", Some("http://env.test/v1")),
            ("OPENAI_MODEL", Some("env-model")),
        ]);
        let config = OpenAiConfig::from_env_with_credentials(None).expect("env key present");
        assert_eq!(config.api_key.as_deref(), Some("env-key"));
        assert_eq!(config.base_url, "http://env.test/v1");
        assert_eq!(config.model, "env-model");
    }

    #[test]
    fn config_from_env_without_any_key_returns_none() {
        let _guard = env_guard(&[("OPENAI_API_KEY", None)]);
        assert!(OpenAiConfig::from_env_with_credentials(None).is_none());
    }

    #[test]
    fn config_from_env_with_credentials_prefers_credentials_over_env() {
        let _guard = env_guard(&[
            ("OPENAI_API_KEY", Some("env-key")),
            ("AH_OPENAI_CRED_KEY", Some("cred-key")),
            ("AH_OPENAI_CRED_BASE", Some("http://cred.test/v1")),
            ("AH_OPENAI_CRED_MODEL", Some("cred-model")),
        ]);
        let mut mapping = HashMap::new();
        mapping.insert(
            "openai.api_key".to_string(),
            "AH_OPENAI_CRED_KEY".to_string(),
        );
        mapping.insert(
            "openai.base_url".to_string(),
            "AH_OPENAI_CRED_BASE".to_string(),
        );
        mapping.insert(
            "openai.model".to_string(),
            "AH_OPENAI_CRED_MODEL".to_string(),
        );
        let provider = EnvCredentialProvider::new(mapping);
        let config =
            OpenAiConfig::from_env_with_credentials(Some(&provider)).expect("credentials present");
        // credentials seam 是更优先的来源,即使 OPENAI_API_KEY 环境变量也存在。
        assert_eq!(config.api_key.as_deref(), Some("cred-key"));
        assert_eq!(config.base_url, "http://cred.test/v1");
        assert_eq!(config.model, "cred-model");
    }

    #[test]
    fn plugin_from_env_with_credentials_resolves_via_ctx() {
        let _guard = env_guard(&[("AH_OPENAI_CTX_KEY", Some("ctx-key"))]);
        let ctx = Context::new();
        let credentials: DynPlugin = Arc::new(ah_plugins_credentials::CredentialsPlugin::new(
            key_mapping_provider("AH_OPENAI_CTX_KEY").mapping().clone(),
        ));
        let effects = ctx.mount(&credentials).expect("mount credentials");

        let plugin = OpenAiPlugin::from_env_with_credentials(&ctx).expect("resolved via ctx");
        assert_eq!(
            plugin.config.as_ref().unwrap().api_key.as_deref(),
            Some("ctx-key"),
            "构造时经 credentials seam 解析出 key"
        );

        drop(effects);
        // 先释放外层 env guard(避免同线程二次加锁),再验证无凭据时解析失败 → None。
        drop(_guard);
        let _clean = env_guard(&[("OPENAI_API_KEY", None)]);
        assert!(OpenAiPlugin::from_env_with_credentials(&ctx).is_none());
    }

    /// 启动一个真实本地 HTTP 服务器,记录请求的 Authorization 头并应答成功。
    fn start_capturing_server() -> (String, Arc<Mutex<Option<String>>>) {
        let server = tiny_http::Server::http("127.0.0.1:0").expect("bind test server");
        let addr = server.server_addr().to_string();
        let base_url = format!("http://{addr}");
        let captured_auth: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let auth_capture = captured_auth.clone();
        let _handle = std::thread::spawn(move || {
            for request in server.incoming_requests() {
                let auth = request
                    .headers()
                    .iter()
                    .find(|header| header.field.equiv("Authorization"))
                    .map(|header| header.value.as_str().to_string());
                *auth_capture.lock().unwrap() = auth;
                let response = tiny_http::Response::from_string(
                    r#"{"id":"chatcmpl-test","object":"chat.completion","created":0,"model":"test-model","choices":[{"index":0,"message":{"role":"assistant","content":"hello from test server"},"finish_reason":"stop"}],"usage":null}"#,
                );
                let _ = request.respond(response);
            }
        });
        (base_url, captured_auth)
    }

    #[tokio::test]
    async fn lazy_plugin_apply_uses_credentials_key_on_real_http_roundtrip() {
        // 真实 e2e:env(OPENAI_API_KEY=env-key)与 credentials(cred-key)都提供 key,
        // credentials seam 必须优先;真实 HTTP 请求携带 Bearer cred-key。
        let (base_url, captured_auth) = start_capturing_server();
        let _guard = env_guard(&[
            ("OPENAI_API_KEY", Some("env-key")),
            ("OPENAI_BASE_URL", Some(&base_url)),
            ("AH_OPENAI_E2E_KEY", Some("cred-key")),
        ]);
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![
            Arc::new(ah_plugins_credentials::CredentialsPlugin::new(
                key_mapping_provider("AH_OPENAI_E2E_KEY").mapping().clone(),
            )),
            Arc::new(OpenAiPlugin::lazy()),
        ];
        let effects = ctx.mount_all(plugins).expect("mount credentials + openai");

        let provider: Arc<dyn ModelProvider> =
            ctx.service(&LLM).expect("llm seam should be registered");
        assert_eq!(provider.name(), "openai-compatible");
        let response = provider
            .chat(ModelRequest {
                messages: vec![ChatMessage::new(ChatRole::User, "hi")],
                ..Default::default()
            })
            .await
            .expect("chat");
        assert_eq!(response.content, "hello from test server");
        let auth = captured_auth.lock().unwrap().clone();
        assert_eq!(
            auth.as_deref(),
            Some("Bearer cred-key"),
            "apply 时经 credentials seam 解析的 key 必须用于真实 HTTP 请求"
        );
        drop(effects);
    }

    #[tokio::test]
    async fn lazy_plugin_apply_fails_explicitly_without_any_key() {
        // 无 credentials、无 OPENAI_API_KEY:apply 必须显式失败,不静默降级。
        let _guard = env_guard(&[("OPENAI_API_KEY", None), ("AH_OPENAI_NO_KEY", None)]);
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(OpenAiPlugin::lazy());
        let error = ctx.mount(&plugin).expect_err("apply must fail without key");
        let message = error.to_string();
        assert!(
            message.contains("no API key") && message.contains("credentials"),
            "explicit failure message, got: {message}"
        );
    }
}
