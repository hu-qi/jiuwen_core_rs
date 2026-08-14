//! # ah-plugins-openai
//!
//! 真实 OpenAI 兼容 HTTP 模型 provider(chat/completions)。
//! 协议层移植自 agent-core_rs 的 OpenAiCompatibleClient;本 crate 是真实实现,
//! 无 mock 业务逻辑。集成测试用本地真实 HTTP 服务器验证真实协议路径。

use std::sync::Arc;
use std::time::Duration;

use ah_contracts::keys::LLM;
use ah_contracts::llm::{
    ChatMessage, ChatRole, ModelError, ModelProvider, ModelRequest, ModelResponse,
};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// provider 配置(配置字段,来自环境或调用方,不硬编码)。
#[derive(Clone, Debug)]
pub struct OpenAiConfig {
    /// 基础 URL,如 https://api.openai.com/v1
    pub base_url: String,
    /// 可选 API key;缺失时按无认证请求。
    pub api_key: Option<String>,
    /// 默认模型名。
    pub model: String,
    /// 请求超时。
    pub timeout: Duration,
}

impl OpenAiConfig {
    /// 从环境变量构建:OPENAI_API_KEY(必须)、OPENAI_BASE_URL、OPENAI_MODEL。
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
}

// ------------------------------------------------------------------
// OpenAI wire 协议类型(仅本 crate 内部使用)
// ------------------------------------------------------------------

#[derive(Serialize)]
struct WireRequest {
    model: String,
    messages: Vec<WireMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[derive(Serialize, Deserialize)]
struct WireMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
}

#[derive(Deserialize)]
struct WireResponse {
    choices: Vec<WireChoice>,
}

#[derive(Deserialize)]
struct WireChoice {
    message: Option<WireMessage>,
}

fn wire_role(role: ChatRole) -> &'static str {
    match role {
        ChatRole::System => "system",
        ChatRole::User => "user",
        ChatRole::Assistant => "assistant",
        ChatRole::Tool => "tool",
    }
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
}

impl Seam for OpenAiModelProvider {}

#[async_trait]
impl ModelProvider for OpenAiModelProvider {
    fn name(&self) -> &'static str {
        "openai-compatible"
    }

    async fn chat(&self, request: ModelRequest) -> Result<ModelResponse, ModelError> {
        let wire = WireRequest {
            model: self.request_model(&request),
            messages: request
                .messages
                .iter()
                .map(|message: &ChatMessage| WireMessage {
                    role: wire_role(message.role).to_string(),
                    content: Some(message.content.clone()),
                })
                .collect(),
            temperature: request.temperature,
            stream: None,
        };

        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
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
        let content = parsed
            .choices
            .into_iter()
            .next()
            .and_then(|choice| choice.message)
            .and_then(|message| message.content)
            .unwrap_or_default();

        Ok(ModelResponse { content })
    }
}

/// 真实 LLM provider 插件:注册到 llm seam。
pub struct OpenAiPlugin {
    config: OpenAiConfig,
}

impl OpenAiPlugin {
    /// 以显式配置构建。
    pub fn new(config: OpenAiConfig) -> Self {
        Self { config }
    }

    /// 从环境变量构建;无 OPENAI_API_KEY 时返回 None。
    pub fn from_env() -> Option<Self> {
        OpenAiConfig::from_env().map(Self::new)
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
        let provider =
            OpenAiModelProvider::new(self.config.clone()).map_err(|e| PluginError::Apply {
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
}
