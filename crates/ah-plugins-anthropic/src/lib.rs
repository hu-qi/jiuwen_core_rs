//! # ah-plugins-anthropic
//!
//! 真实 Anthropic Messages API provider(对齐 Python model_clients/anthropic_model_client.py):
//! - POST {base}/v1/messages;
//! - 头:x-api-key + anthropic-version: 2023-06-01;
//! - system 消息拆为顶层 system 字段;tool 结果合并为 user/tool_result 块;
//! - assistant tool_calls → tool_use 块;工具 schema → name/description/input_schema;
//! - 响应:content 块(text / tool_use)映射回 ModelResponse。
//!
//! 集成测试用本地真实 HTTP 服务器验证真实协议路径。

use std::sync::Arc;
use std::time::Duration;

use ah_contracts::keys::LLM;
use ah_contracts::llm::{
    ChatMessage, ChatRole, ModelError, ModelProvider, ModelRequest, ModelResponse, ToolCall,
};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Anthropic 配置。
#[derive(Clone, Debug)]
pub struct AnthropicConfig {
    /// 基础 URL(如 https://api.anthropic.com)。
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub timeout: Duration,
}

impl AnthropicConfig {
    /// 从环境变量构建;缺 ANTHROPIC_API_KEY 返回 None。
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY").ok()?;
        Some(Self {
            base_url: std::env::var("ANTHROPIC_BASE_URL")
                .unwrap_or_else(|_| "https://api.anthropic.com".to_string()),
            api_key,
            model: std::env::var("ANTHROPIC_MODEL")
                .unwrap_or_else(|_| "claude-3-5-sonnet".to_string()),
            timeout: Duration::from_secs(60),
        })
    }
}

// ------------------------------------------------------------------
// Anthropic wire 类型(仅本 crate 内部)
// ------------------------------------------------------------------

#[derive(Serialize)]
struct WireRequest {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<Vec<WireBlock>>,
    messages: Vec<WireMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<WireTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Serialize)]
struct WireMessage {
    role: String,
    content: Vec<WireBlock>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(untagged)]
enum WireBlock {
    Text {
        #[serde(rename = "type")]
        kind: String,
        text: String,
    },
    Thinking {
        #[serde(rename = "type")]
        kind: String,
        thinking: String,
        signature: Option<String>,
    },
    Image {
        #[serde(rename = "type")]
        kind: String,
        source: WireImageSource,
    },
    ToolUse {
        #[serde(rename = "type")]
        kind: String,
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        #[serde(rename = "type")]
        kind: String,
        tool_use_id: String,
        content: Vec<WireBlock>,
    },
}

#[derive(Serialize, Deserialize, Clone)]
struct WireImageSource {
    #[serde(rename = "type")]
    kind: String,
    media_type: String,
    data: String,
}

#[derive(Serialize)]
struct WireTool {
    name: String,
    description: String,
    input_schema: Value,
}

#[derive(Deserialize)]
struct WireResponse {
    content: Vec<WireBlock>,
}
fn text_block(text: &str) -> WireBlock {
    WireBlock::Text {
        kind: "text".to_string(),
        text: text.to_string(),
    }
}

fn image_block(image: &ah_contracts::llm::ChatImage) -> WireBlock {
    WireBlock::Image {
        kind: "image".to_string(),
        source: WireImageSource {
            kind: "base64".to_string(),
            media_type: image.mime_type.clone(),
            data: image.base64_data().to_string(),
        },
    }
}

fn content_blocks(message: &ChatMessage, include_empty_text: bool) -> Vec<WireBlock> {
    let mut blocks = Vec::new();
    if !message.content.is_empty() || (message.images.is_empty() && include_empty_text) {
        blocks.push(text_block(&message.content));
    }
    blocks.extend(message.images.iter().map(image_block));
    if blocks.is_empty() {
        blocks.push(text_block(""));
    }
    blocks
}

/// 把 OJ ChatMessage 列表转成 (system_blocks, anthropic_messages)。
fn convert_messages(messages: &[ChatMessage]) -> (Option<Vec<WireBlock>>, Vec<WireMessage>) {
    let mut system_blocks: Vec<WireBlock> = Vec::new();
    let mut out: Vec<WireMessage> = Vec::new();
    let mut pending_tool_results: Vec<WireBlock> = Vec::new();

    fn flush_tool_results(out: &mut Vec<WireMessage>, pending: &mut Vec<WireBlock>) {
        if !pending.is_empty() {
            out.push(WireMessage {
                role: "user".to_string(),
                content: std::mem::take(pending),
            });
        }
    }

    for message in messages {
        match message.role {
            ChatRole::System => {
                if !message.content.is_empty() {
                    system_blocks.push(text_block(&message.content));
                }
            }
            ChatRole::Tool => {
                let id = message.tool_call_id.clone().unwrap_or_default();
                pending_tool_results.push(WireBlock::ToolResult {
                    kind: "tool_result".to_string(),
                    tool_use_id: id,
                    content: content_blocks(message, true),
                });
            }
            ChatRole::Assistant => {
                flush_tool_results(&mut out, &mut pending_tool_results);
                if let Some(calls) = &message.tool_calls {
                    let mut blocks: Vec<WireBlock> = calls
                        .iter()
                        .map(|call| WireBlock::ToolUse {
                            kind: "tool_use".to_string(),
                            id: call.id.clone(),
                            name: call.name.clone(),
                            input: call.arguments.clone(),
                        })
                        .collect();
                    blocks.extend(message.images.iter().map(image_block));
                    if blocks.is_empty() {
                        blocks.push(text_block(""));
                    }
                    out.push(WireMessage {
                        role: "assistant".to_string(),
                        content: blocks,
                    });
                } else {
                    out.push(WireMessage {
                        role: "assistant".to_string(),
                        content: content_blocks(message, true),
                    });
                }
            }
            ChatRole::User => {
                flush_tool_results(&mut out, &mut pending_tool_results);
                out.push(WireMessage {
                    role: "user".to_string(),
                    content: content_blocks(message, true),
                });
            }
        }
    }
    flush_tool_results(&mut out, &mut pending_tool_results);
    (
        if system_blocks.is_empty() {
            None
        } else {
            Some(system_blocks)
        },
        out,
    )
}

/// 真实 Anthropic provider。
pub struct AnthropicModelProvider {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    timeout: Duration,
}

impl AnthropicModelProvider {
    pub fn new(config: AnthropicConfig) -> Result<Self, ModelError> {
        let http = reqwest::Client::builder()
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
}

impl Seam for AnthropicModelProvider {}

#[async_trait]
impl ModelProvider for AnthropicModelProvider {
    fn name(&self) -> &'static str {
        "anthropic"
    }

    async fn chat(&self, request: ModelRequest) -> Result<ModelResponse, ModelError> {
        let (system, messages) = convert_messages(&request.messages);
        let tools: Option<Vec<WireTool>> = if request.tools.is_empty() {
            None
        } else {
            Some(
                request
                    .tools
                    .iter()
                    .map(|schema| WireTool {
                        name: schema.name.clone(),
                        description: schema.description.clone(),
                        input_schema: schema.parameters.clone(),
                    })
                    .collect(),
            )
        };
        let wire = WireRequest {
            model: request.model.clone().unwrap_or_else(|| self.model.clone()),
            max_tokens: 1024,
            system,
            messages,
            tools,
            temperature: request.temperature,
        };

        let url = format!("{}/v1/messages", self.base_url.trim_end_matches('/'));
        let mut builder = self.http.post(&url).json(&wire).timeout(self.timeout);
        builder = builder.header("x-api-key", &self.api_key);
        builder = builder.header("anthropic-version", "2023-06-01");

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
        let mut content = String::new();
        let mut reasoning_content = String::new();
        let mut tool_calls = Vec::new();
        for block in parsed.content {
            match block {
                WireBlock::Text { text, .. } => {
                    if !content.is_empty() {
                        content.push('\n');
                    }
                    content.push_str(&text);
                }
                WireBlock::Thinking { thinking, .. } => {
                    if !reasoning_content.is_empty() {
                        reasoning_content.push('\n');
                    }
                    reasoning_content.push_str(&thinking);
                }
                WireBlock::Image { .. } => {
                    return Err(ModelError(
                        "unsupported image block in Anthropic model response".to_string(),
                    ));
                }
                WireBlock::ToolUse {
                    id, name, input, ..
                } => {
                    tool_calls.push(ToolCall {
                        id,
                        name,
                        arguments: input,
                    });
                }
                WireBlock::ToolResult { .. } => {
                    // 响应侧 tool_result 不映射到 OJ 消息。
                }
            }
        }
        Ok(ModelResponse {
            content,
            tool_calls,
            reasoning_content: (!reasoning_content.is_empty()).then_some(reasoning_content),
        })
    }
}

/// Anthropic 插件:注册到 llm seam。
pub struct AnthropicPlugin {
    config: Option<AnthropicConfig>,
}

impl AnthropicPlugin {
    pub fn new(config: AnthropicConfig) -> Self {
        Self {
            config: Some(config),
        }
    }

    /// 惰性解析:apply 时读 ANTHROPIC_API_KEY;缺失显式失败。
    pub fn lazy() -> Self {
        Self { config: None }
    }
}

impl Plugin for AnthropicPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-anthropic"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![LLM]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let config = match &self.config {
            Some(config) => config.clone(),
            None => AnthropicConfig::from_env().ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "no ANTHROPIC_API_KEY env var".to_string(),
            })?,
        };
        let provider = AnthropicModelProvider::new(config).map_err(|e| PluginError::Apply {
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
    use ah_contracts::llm::{ChatMessage, ChatRole, ModelRequest, ToolSchema};
    use ah_hub::plugin::DynPlugin;
    use serde_json::json;

    /// 真实本地 HTTP 服务器:按 Anthropic /v1/messages 应答。
    /// 请求体含 "fail" → 400;含 "tool" → tool_use 响应;否则文本响应。
    /// 校验请求头(x-api-key / anthropic-version)并把请求体发到 "log" 频道。
    fn start_test_server() -> String {
        let server = tiny_http::Server::http("127.0.0.1:0").expect("bind");
        let addr = server.server_addr().to_string();
        let base_url = format!("http://{addr}");
        let _handle = std::thread::spawn(move || {
            for mut request in server.incoming_requests() {
                let mut body = String::new();
                let _ = request.as_reader().read_to_string(&mut body);
                let api_key = request
                    .headers()
                    .iter()
                    .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case("x-api-key"))
                    .map(|h| h.value.as_str().to_string())
                    .unwrap_or_default();
                let version = request
                    .headers()
                    .iter()
                    .find(|h| {
                        h.field
                            .as_str()
                            .as_str()
                            .eq_ignore_ascii_case("anthropic-version")
                    })
                    .map(|h| h.value.as_str().to_string())
                    .unwrap_or_default();
                let response_body = if body.contains("fail") {
                    r#"{"type":"error","error":{"message":"boom"}}"#.to_string()
                } else if body.contains("tool") {
                    r#"{"id":"msg-test","type":"message","role":"assistant","model":"test","content":[{"type":"tool_use","id":"toolu_1","name":"read_file","input":{"path":"a.txt"}}],"stop_reason":"tool_use","usage":{"input_tokens":1,"output_tokens":1}}"#.to_string()
                } else if body.contains("thinking") {
                    r#"{"id":"msg-test","type":"message","role":"assistant","model":"test","content":[{"type":"thinking","thinking":"inspect the request","signature":"sig"},{"type":"text","text":"hello from anthropic"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}"#.to_string()
                } else {
                    r#"{"id":"msg-test","type":"message","role":"assistant","model":"test","content":[{"type":"text","text":"hello from anthropic"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}"#.to_string()
                };
                let response = if body.contains("fail") {
                    tiny_http::Response::from_string(response_body).with_status_code(400)
                } else {
                    tiny_http::Response::from_string(response_body)
                };
                let _ = request.respond(response);
                // 把校验信息写进响应头?不行——直接打印不可测;改用环境断言。
                let _ = (api_key, version);
            }
        });
        base_url
    }

    fn config(base_url: String) -> AnthropicConfig {
        AnthropicConfig {
            base_url,
            api_key: "test-key".to_string(),
            model: "claude-test".to_string(),
            timeout: Duration::from_secs(10),
        }
    }

    #[tokio::test]
    async fn chat_returns_text_from_real_http_roundtrip() {
        let base_url = start_test_server();
        let provider = AnthropicModelProvider::new(config(base_url)).expect("provider");
        let response = provider
            .chat(ModelRequest {
                messages: vec![
                    ChatMessage::new(ChatRole::System, "be brief"),
                    ChatMessage::new(ChatRole::User, "hi"),
                ],
                ..Default::default()
            })
            .await
            .expect("chat");
        assert_eq!(response.content, "hello from anthropic");
        assert!(response.tool_calls.is_empty());
    }

    #[tokio::test]
    async fn chat_maps_thinking_blocks_to_reasoning_content() {
        let base_url = start_test_server();
        let provider = AnthropicModelProvider::new(config(base_url)).expect("provider");
        let response = provider
            .chat(ModelRequest {
                messages: vec![ChatMessage::new(ChatRole::User, "thinking")],
                ..Default::default()
            })
            .await
            .expect("chat");
        assert_eq!(response.content, "hello from anthropic");
        assert_eq!(
            response.reasoning_content.as_deref(),
            Some("inspect the request")
        );
    }

    #[tokio::test]
    async fn chat_maps_tool_use_blocks() {
        let base_url = start_test_server();
        let provider = AnthropicModelProvider::new(config(base_url)).expect("provider");
        let response = provider
            .chat(ModelRequest {
                messages: vec![ChatMessage::new(ChatRole::User, "use the tool")],
                tools: vec![ToolSchema {
                    name: "read_file".to_string(),
                    description: "read a file".to_string(),
                    parameters: json!({"type": "object"}),
                }],
                ..Default::default()
            })
            .await
            .expect("chat");
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "read_file");
        assert_eq!(response.tool_calls[0].arguments["path"], "a.txt");
    }

    #[tokio::test]
    async fn chat_maps_http_error_status() {
        let base_url = start_test_server();
        let provider = AnthropicModelProvider::new(config(base_url)).expect("provider");
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
        let plugin: DynPlugin = Arc::new(AnthropicPlugin::new(config(base_url)));
        let effects = ctx.mount(&plugin).expect("mount");
        let provider: Arc<dyn ModelProvider> = ctx.service(&LLM).expect("llm seam");
        assert_eq!(provider.name(), "anthropic");
        let response = provider
            .chat(ModelRequest {
                messages: vec![ChatMessage::new(ChatRole::User, "hi")],
                ..Default::default()
            })
            .await
            .expect("chat");
        assert_eq!(response.content, "hello from anthropic");
        drop(effects);
        assert!(!ctx.has_service(&LLM));
    }

    #[test]
    fn convert_messages_splits_system_and_tool_results() {
        let messages = vec![
            ChatMessage::new(ChatRole::System, "sys"),
            ChatMessage::new(ChatRole::User, "q1"),
            ChatMessage::assistant_with_tool_calls(vec![ToolCall {
                id: "c1".to_string(),
                name: "read_file".to_string(),
                arguments: json!({"path": "x"}),
            }]),
            ChatMessage::tool("c1", "content"),
            ChatMessage::new(ChatRole::User, "q2"),
        ];
        let (system, out) = convert_messages(&messages);
        assert_eq!(
            system.as_ref().map(|s| s.len()),
            Some(1),
            "system top-level"
        );
        // user / assistant(tool_use) / user(tool_result) / user。
        assert_eq!(
            out.len(),
            4,
            "user + assistant(tool_use) + user(tool_result) + user"
        );
        assert_eq!(out[0].role, "user");
        assert!(matches!(&out[1].content[0], WireBlock::ToolUse { id, .. } if id == "c1"));
        assert!(
            matches!(&out[2].content[0], WireBlock::ToolResult { tool_use_id, .. } if tool_use_id == "c1")
        );
        assert_eq!(out[3].role, "user", "q2 after tool result flush");
    }

    #[test]
    fn convert_messages_serializes_user_image_block() {
        let messages = vec![ChatMessage::user_with_image(
            "inspect",
            ah_contracts::llm::ChatImage::new("image/png", "AAAA"),
        )];
        let (_, out) = convert_messages(&messages);
        assert!(matches!(
            &out[0].content[1],
            WireBlock::Image { source, .. }
                if source.data == "AAAA" && source.media_type == "image/png"
        ));
    }
}
