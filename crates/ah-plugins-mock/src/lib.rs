//! # ah-plugins-mock
//!
//! Mock 插件集:仅用于 dev/test profile,生产 profile 禁止选择。
//!
//! 当前提供:
//! - `MockModelProvider`:确定性模型,回显用户消息(`llm` seam);
//! - `MockToolRegistry` + `EchoTool`/`AddTool`:确定性工具(`tools` seam);
//! - `MockPlugin`:同时注册 `llm` 与 `tools` 两个 seam 的示例插件。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ah_contracts::keys::{LLM, TOOLS};
use ah_contracts::llm::{ChatRole, ModelError, ModelProvider, ModelRequest, ModelResponse};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::tools::{Tool, ToolError, ToolRegistry};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use serde_json::{Value, json};

/// `llm` seam 服务键(兼容别名,指向契约层稳定键)。
pub const LLM_KEY: ServiceKey = LLM;

/// `tools` seam 服务键(兼容别名)。
pub const TOOLS_KEY: ServiceKey = TOOLS;

// ------------------------------------------------------------------
// llm seam
// ------------------------------------------------------------------

/// 确定性 mock 模型:回显最后一条用户消息。
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

// ------------------------------------------------------------------
// tools seam
// ------------------------------------------------------------------

/// 回显工具:原样返回输入参数。
pub struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &'static str {
        "echo"
    }

    fn description(&self) -> &'static str {
        "return the input arguments unchanged"
    }

    async fn invoke(&self, arguments: Value) -> Result<Value, ToolError> {
        Ok(arguments)
    }
}

/// 加法工具:求 `{a, b}` 两个整数的和。
pub struct AddTool;

#[async_trait]
impl Tool for AddTool {
    fn name(&self) -> &'static str {
        "add"
    }

    fn description(&self) -> &'static str {
        "sum two integers {a} and {b}"
    }

    async fn invoke(&self, arguments: Value) -> Result<Value, ToolError> {
        let a = arguments
            .get("a")
            .and_then(Value::as_i64)
            .ok_or_else(|| ToolError("missing integer field `a`".to_string()))?;
        let b = arguments
            .get("b")
            .and_then(Value::as_i64)
            .ok_or_else(|| ToolError("missing integer field `b`".to_string()))?;
        Ok(json!({ "sum": a + b }))
    }
}

/// 内存工具注册表:实现 `ToolRegistry` seam。
pub struct MockToolRegistry {
    tools: Arc<Mutex<HashMap<String, Arc<dyn Tool>>>>,
}

impl MockToolRegistry {
    /// 创建空注册表。
    pub fn new() -> Self {
        Self {
            tools: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for MockToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl Seam for MockToolRegistry {}

#[async_trait]
impl ToolRegistry for MockToolRegistry {
    fn register(&self, tool: Arc<dyn Tool>) -> Effect {
        let name = tool.name().to_string();
        self.tools.lock().unwrap().insert(name.clone(), tool);
        let tools = self.tools.clone();
        Effect::new(move || {
            tools.lock().unwrap().remove(&name);
        })
    }

    fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.lock().unwrap().get(name).cloned()
    }

    fn names(&self) -> Vec<String> {
        self.tools.lock().unwrap().keys().cloned().collect()
    }

    async fn invoke(&self, name: &str, arguments: Value) -> Result<Value, ToolError> {
        let tool = self
            .get(name)
            .ok_or_else(|| ToolError(format!("tool not found: {name}")))?;
        tool.invoke(arguments).await
    }
}

// ------------------------------------------------------------------
// 插件:同时提供 llm 与 tools 两个 seam
// ------------------------------------------------------------------

/// 示例插件:把 mock 模型与 mock 工具注册到对应 seam。
pub struct MockPlugin;

impl Plugin for MockPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-mock"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![LLM, TOOLS]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let mut effects = Vec::new();

        // llm seam
        let provider: Arc<dyn ModelProvider> = Arc::new(MockModelProvider);
        effects.push(ctx.register(LLM, provider));

        // tools seam:注册表 + 两个确定性工具
        let registry = Arc::new(MockToolRegistry::new());
        effects.push(registry.register(Arc::new(EchoTool)));
        effects.push(registry.register(Arc::new(AddTool)));
        let registry_service: Arc<dyn ToolRegistry> = registry;
        effects.push(ctx.register(TOOLS, registry_service));

        Ok(effects)
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

    #[tokio::test]
    async fn mock_plugin_registers_and_serves_tools_seam() {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(MockPlugin);
        let effects = ctx.mount(&plugin).expect("mount");

        let registry: Arc<dyn ToolRegistry> = ctx
            .service(&TOOLS)
            .expect("tools seam should be registered");
        let mut names = registry.names();
        names.sort();
        assert_eq!(names, vec!["add", "echo"]);

        let sum = registry
            .invoke("add", json!({ "a": 2, "b": 40 }))
            .await
            .expect("add");
        assert_eq!(sum, json!({ "sum": 42 }));

        let echoed = registry
            .invoke("echo", json!({ "x": 1 }))
            .await
            .expect("echo");
        assert_eq!(echoed, json!({ "x": 1 }));

        let missing = registry.invoke("nope", json!({})).await;
        assert!(matches!(missing, Err(ToolError(message)) if message.contains("nope")));

        let bad_args = registry.invoke("add", json!({ "a": "x" })).await;
        assert!(matches!(bad_args, Err(ToolError(_))));

        drop(effects);
        assert!(!ctx.has_service(&TOOLS));
    }

    #[test]
    fn tool_registration_is_reversible() {
        let registry = MockToolRegistry::new();
        let effect = registry.register(Arc::new(EchoTool));
        assert_eq!(registry.names(), vec!["echo".to_string()]);
        drop(effect);
        assert!(registry.names().is_empty());
    }
}
