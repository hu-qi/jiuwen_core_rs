//! # ah-plugins-mock
//!
//! Mock 插件集:仅用于 dev/test profile,生产 profile 禁止选择。
//!
//! 当前提供:
//! - `MockModelProvider`:确定性模型,回显用户消息;
//! - `MockPlugin`:把 mock 模型注册到 `llm` seam 的示例插件。

use std::sync::Arc;

use ah_contracts::llm::{ChatRole, ModelError, ModelProvider, ModelRequest, ModelResponse};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::effect::Effect;
use ah_hub::plugin::{Plugin, PluginError};

/// `llm` seam 的服务键(与消费方共享的稳定键)。
pub const LLM_KEY: ServiceKey = ServiceKey::new("llm");

/// 确定性 mock 模型:回显最后一条用户消息。
pub struct MockModelProvider;

impl Seam for MockModelProvider {}

#[async_trait::async_trait]
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

/// 示例插件:把 mock 模型注册到 `llm` seam。
pub struct MockPlugin;

impl Plugin for MockPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-mock"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![LLM_KEY]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let provider: Arc<dyn ModelProvider> = Arc::new(MockModelProvider);
        let effect = ctx.register(LLM_KEY, provider);
        Ok(vec![effect])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::llm::{ChatMessage, ChatRole, ModelRequest};
    use ah_hub::plugin::DynPlugin;

    #[tokio::test]
    async fn mock_plugin_registers_and_serves_llm_seam() {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(MockPlugin);
        let effects = ctx.mount(&plugin).expect("mount");

        let provider: Arc<dyn ModelProvider> = ctx
            .service(&LLM_KEY)
            .expect("llm seam should be registered");
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
        assert!(!ctx.has_service(&LLM_KEY));
    }
}
