//! LLM Seam(Service Definition 示例)。

use async_trait::async_trait;

use crate::seam::Seam;

/// 对话消息角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatRole {
    /// 系统指令。
    System,
    /// 用户消息。
    User,
    /// 助手消息。
    Assistant,
    /// 工具执行结果。
    Tool,
}

/// 一条对话消息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

impl ChatMessage {
    /// 构造一条消息。
    pub fn new(role: ChatRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
        }
    }
}

/// 模型请求。
#[derive(Debug, Clone, Default)]
pub struct ModelRequest {
    pub messages: Vec<ChatMessage>,
    pub model: Option<String>,
    pub temperature: Option<f32>,
}

/// 模型响应。
#[derive(Debug, Clone)]
pub struct ModelResponse {
    pub content: String,
}

/// 模型错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelError(pub String);

impl core::fmt::Display for ModelError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ModelError {}

/// LLM Seam(Service Definition)。
///
/// 每个模型提供方(OpenAI、DeepSeek、本地 vLLM、mock)实现本 trait,
/// 消费方(agent 循环、workflow 的 LLM 组件)只依赖本 trait。
#[async_trait]
pub trait ModelProvider: Seam {
    /// 提供方稳定名称,如 `mock`、`openai-compatible`。
    fn name(&self) -> &'static str;

    /// 发送一轮对话,返回助手消息。
    async fn chat(&self, request: ModelRequest) -> Result<ModelResponse, ModelError>;
}
