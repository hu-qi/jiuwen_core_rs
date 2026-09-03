//! # ah-plugins-mock
//!
//! **仅作 boot 桩**:`llm` 真实 provider 需要凭据,接入前用确定性桩保证系统可启动。
//! 本插件**不算完成交付物**;真实 LLM provider(ah-plugins-openai)落地后移除。

use std::sync::{Arc, Mutex};

use ah_contracts::keys::LLM;
use ah_contracts::llm::{
    ChatRole, ModelError, ModelProvider, ModelRequest, ModelResponse, ToolCall,
};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use serde_json::json;

/// `llm` seam 服务键(兼容别名,指向契约层稳定键)。
pub const LLM_KEY: ServiceKey = LLM;

/// 确定性 mock 模型(boot 桩):驱动 ReAct 循环的结构。
///
/// 行为:对话中尚无工具结果时,请求调用 `list_dir`;
/// 收到工具结果后,给出引用结果的最终回答。
pub struct MockModelProvider {
    calls: Mutex<usize>,
}

impl MockModelProvider {
    /// 创建 mock 模型。
    pub fn new() -> Self {
        Self {
            calls: Mutex::new(0),
        }
    }
}

impl Default for MockModelProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Seam for MockModelProvider {}

#[async_trait]
impl ModelProvider for MockModelProvider {
    fn name(&self) -> &'static str {
        "mock"
    }

    async fn chat(&self, request: ModelRequest) -> Result<ModelResponse, ModelError> {
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;

        let has_tool_result = request
            .messages
            .iter()
            .any(|message| message.role == ChatRole::Tool);

        if !has_tool_result {
            // 第一轮:请求执行真实工具 list_dir。
            return Ok(ModelResponse {
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: format!("mock-call-{}", *calls),
                    name: "list_dir".to_string(),
                    arguments: json!({ "path": "." }),
                }],
                reasoning_content: None,
            });
        }

        // 后续轮:引用最后一条工具结果给出最终回答。
        let last_tool = request
            .messages
            .iter()
            .rev()
            .find(|message| message.role == ChatRole::Tool)
            .map(|message| message.content.clone())
            .unwrap_or_default();
        Ok(ModelResponse {
            content: format!("mock final answer; last tool result: {last_tool}"),
            tool_calls: Vec::new(),
            reasoning_content: None,
        })
    }
}

/// Mock 插件:注册 `llm` seam(boot 桩)。
pub struct MockPlugin;

impl Plugin for MockPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-mock"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![LLM]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let provider: Arc<dyn ModelProvider> = Arc::new(MockModelProvider::new());
        Ok(vec![ctx.register(LLM, provider)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::llm::ChatMessage;
    use ah_hub::plugin::DynPlugin;

    #[tokio::test]
    async fn mock_plugin_registers_and_serves_llm_seam() {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(MockPlugin);
        let effects = ctx.mount(&plugin).expect("mount");

        let provider: Arc<dyn ModelProvider> =
            ctx.service(&LLM).expect("llm seam should be registered");
        assert_eq!(provider.name(), "mock");

        // 第一轮:请求工具调用(驱动 ReAct)。
        let first = provider
            .chat(ModelRequest {
                messages: vec![ChatMessage::new(ChatRole::User, "hello")],
                ..Default::default()
            })
            .await
            .expect("mock chat");
        assert_eq!(first.tool_calls.len(), 1);
        assert_eq!(first.tool_calls[0].name, "list_dir");

        // 第二轮:收到工具结果后给最终回答。
        let second = provider
            .chat(ModelRequest {
                messages: vec![
                    ChatMessage::new(ChatRole::User, "hello"),
                    ChatMessage::assistant_with_tool_calls(first.tool_calls.clone()),
                    ChatMessage::tool("mock-call-1", "\"plan.md\""),
                ],
                ..Default::default()
            })
            .await
            .expect("mock chat");
        assert!(second.tool_calls.is_empty());
        assert!(second.content.contains("mock final answer"));
        assert!(second.content.contains("plan.md"));

        drop(effects);
        assert!(!ctx.has_service(&LLM));
    }
}
