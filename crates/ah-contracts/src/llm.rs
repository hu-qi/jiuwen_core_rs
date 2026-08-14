//! LLM Seam(Service Definition)。

use async_trait::async_trait;
use serde_json::Value;

use crate::seam::Seam;

/// 对话消息角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatRole {
    /// 系统指令。
    System,
    /// 用户消息。
    User,
    /// 助手消息(可携带工具调用)。
    Assistant,
    /// 工具执行结果。
    Tool,
}

/// 模型请求中的工具调用。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// 模型可见的工具 schema(进入请求的 tools 字段)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// 一条对话消息。
#[derive(Debug, Clone, PartialEq)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
    /// Tool 角色消息对应的工具调用 id(OpenAI 协议要求)。
    pub tool_call_id: Option<String>,
    /// Assistant 消息携带的工具调用。
    pub tool_calls: Option<Vec<ToolCall>>,
}

impl ChatMessage {
    /// 构造一条普通消息。
    pub fn new(role: ChatRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            tool_call_id: None,
            tool_calls: None,
        }
    }

    /// 构造带工具调用的助手消息。
    pub fn assistant_with_tool_calls(tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: String::new(),
            tool_call_id: None,
            tool_calls: Some(tool_calls),
        }
    }

    /// 构造工具执行结果消息。
    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Tool,
            content: content.into(),
            tool_call_id: Some(tool_call_id.into()),
            tool_calls: None,
        }
    }
}

/// 模型请求。
#[derive(Debug, Clone, Default)]
pub struct ModelRequest {
    pub messages: Vec<ChatMessage>,
    /// 模型可见工具 schema(由 agent 循环从 tools seam 组装)。
    pub tools: Vec<ToolSchema>,
    pub model: Option<String>,
    pub temperature: Option<f32>,
}

/// 模型响应。
#[derive(Debug, Clone, Default)]
pub struct ModelResponse {
    pub content: String,
    /// 模型请求执行的工具调用(空表示直接给出最终回答)。
    pub tool_calls: Vec<ToolCall>,
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

    /// 发送一轮对话,返回助手消息(可含工具调用)。
    async fn chat(&self, request: ModelRequest) -> Result<ModelResponse, ModelError>;
}
