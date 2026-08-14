//! # ah-plugins-mock
//!
//! **仅作 boot 桩**:`llm` 真实 provider 需要凭据,接入前用确定性桩保证系统可启动。
//! 本插件**不算完成交付物**;真实 LLM provider(ah-plugins-openai)落地后移除。

use std::sync::Arc;

use ah_contracts::keys::LLM;
use ah_contracts::llm::{ChatRole, ModelError, ModelProvider, ModelRequest, ModelResponse};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;

/// `llm` seam 服务键(兼容别名,指向契约层稳定键)。
pub const LLM_KEY: ServiceKey = LLM;

/// 确定性 mock 模型:回显最后一条用户消息(boot 桩)。
pub struct MockModelProvider;

impl Seam for MockModelProvider {}

#[async_trait]
impl ModelProvider for MockModelProvider {
    fn name(&self) -> &'static str {
        "mock"
    }

    async fn chat(&self, request: ModelRequest) -> Result<ModelResponse, ModelError> {
        let content = request
            .messages
            .iter()
            .rev()
            .find(|message| message.role == ChatRole::User)
            .map(|message| format!("mock reply to: {}", message.content))
            .unwrap_or_else(|| "mock reply".to_string());
        Ok(ModelResponse { content })
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
        let provider: Arc<dyn ModelProvider> = Arc::new(MockModelProvider);
        Ok(vec![ctx.register(LLM, provider)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::llm::{ChatMessage, ModelRequest};
    use ah_hub::plugin::DynPlugin;

    #[tokio::test]
    async fn mock_plugin_registers_and_serves_llm_seam() {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(MockPlugin);
        let effects = ctx.mount(&plugin).expect("mount");

        let provider: Arc<dyn ModelProvider> =
            ctx.service(&LLM).expect("llm seam should be registered");
        assert_eq!(provider.name(), "mock");

        let response = provider
            .chat(ModelRequest {
                messages: vec![ChatMessage::new(ChatRole::User, "hello")],
                ..Default::default()
            })
            .await
            .expect("mock chat");
        assert_eq!(response.content, "mock reply to: hello");

        drop(effects);
        assert!(!ctx.has_service(&LLM));
    }
}
