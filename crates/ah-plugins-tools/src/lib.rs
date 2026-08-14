//! # ah-plugins-tools
//!
//! 提供 `tools` seam 的注册表实现(真实基础设施)。
//! 注册表本身是通用存储;工具的能力由各插件注册到它上面
//! (如 ah-plugins-sysop 注册真实的 read_file/run_shell 等)。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ah_contracts::keys::TOOLS;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::tools::{Tool, ToolError, ToolRegistry};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use serde_json::Value;

/// 内存工具注册表:实现 `ToolRegistry` seam。
pub struct LocalToolRegistry {
    tools: Arc<Mutex<HashMap<String, Arc<dyn Tool>>>>,
}

impl LocalToolRegistry {
    /// 创建空注册表。
    pub fn new() -> Self {
        Self {
            tools: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for LocalToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl Seam for LocalToolRegistry {}

#[async_trait]
impl ToolRegistry for LocalToolRegistry {
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

/// 提供 `tools` seam 的插件。
pub struct ToolsPlugin;

impl Plugin for ToolsPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-tools"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![TOOLS]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let registry: Arc<dyn ToolRegistry> = Arc::new(LocalToolRegistry::new());
        Ok(vec![ctx.register(TOOLS, registry)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_hub::plugin::DynPlugin;

    struct NoopTool;

    #[async_trait]
    impl Tool for NoopTool {
        fn name(&self) -> &'static str {
            "noop"
        }

        fn description(&self) -> &'static str {
            "do nothing"
        }

        async fn invoke(&self, _arguments: Value) -> Result<Value, ToolError> {
            Ok(serde_json::json!({ "ok": true }))
        }
    }

    #[test]
    fn registry_register_get_invoke_roundtrip() {
        let registry = LocalToolRegistry::new();
        let effect = registry.register(Arc::new(NoopTool));

        assert!(registry.get("noop").is_some());
        assert_eq!(registry.names(), vec!["noop".to_string()]);

        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(registry.invoke("noop", Value::Null));
        assert_eq!(result.unwrap(), serde_json::json!({ "ok": true }));

        drop(effect);
        assert!(registry.get("noop").is_none());
    }

    #[tokio::test]
    async fn tools_plugin_registers_tools_seam() {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(ToolsPlugin);
        let effects = ctx.mount(&plugin).expect("mount");

        let registry: Arc<dyn ToolRegistry> = ctx
            .service(&TOOLS)
            .expect("tools seam should be registered");
        assert!(registry.names().is_empty());

        drop(effects);
        assert!(!ctx.has_service(&TOOLS));
    }
}
