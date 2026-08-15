//! # ah-plugins-mcp
//!
//! 真实 MCP stdio transport:通过 tokio 子进程的 stdin/stdout 讲
//! newline-delimited JSON-RPC 2.0(每行一个 JSON 对象;request 带 `id`,
//! response 按 `id` 匹配),真实 `initialize` 握手与 `shutdown`。
//!
//! 本插件没有 mock:子进程在第一次方法调用时真实 spawn
//! (插件 apply 只注册 seam,不拉起外部命令),所有调用都是真实协议往返。

pub mod client;
pub mod http;
pub use http::McpHttpClient;

use std::sync::Arc;

use ah_contracts::keys::{MCP, TOOLS};
use ah_contracts::mcp::{McpClient, McpContent};
use ah_contracts::prelude::Effect;
use ah_contracts::service::ServiceKey;
use ah_contracts::tools::{Tool, ToolError, ToolRegistry};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::client::StdioMcpClient;

/// MCP 插件:注册 `mcp` seam,并把 `mcp_call_tool` 注入 tools seam。
///
/// 服务器命令与参数来自 `McpPlugin::new`(如默认 `npx` + MCP server 包);
/// 客户端懒 spawn,apply 本身不拉起外部进程。
pub struct McpPlugin {
    command: String,
    args: Vec<String>,
}

impl McpPlugin {
    /// 以服务器命令与参数构造插件。
    pub fn new(command: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            command: command.into(),
            args,
        }
    }
}

impl Plugin for McpPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-mcp"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![MCP]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![TOOLS]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let client: Arc<dyn McpClient> =
            Arc::new(StdioMcpClient::new(self.command.clone(), self.args.clone()));
        let mut effects = vec![ctx.register(MCP, client.clone())];

        let registry =
            ctx.service::<dyn ToolRegistry>(&TOOLS)
                .ok_or_else(|| PluginError::Apply {
                    plugin: self.name(),
                    message: "tools seam not registered".to_string(),
                })?;
        effects.push(registry.register(Arc::new(McpCallTool::new(client))));
        Ok(effects)
    }
}

/// `mcp_call_tool` 工具:经 MCP seam 调用远端工具。
pub struct McpCallTool {
    mcp: Arc<dyn McpClient>,
}

impl McpCallTool {
    pub fn new(mcp: Arc<dyn McpClient>) -> Self {
        Self { mcp }
    }
}

#[async_trait]
impl Tool for McpCallTool {
    fn name(&self) -> &'static str {
        "mcp_call_tool"
    }

    fn description(&self) -> &'static str {
        "call a tool exposed by the MCP server; arguments: {tool, arguments?}"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tool": { "type": "string" },
                "arguments": { "type": "object" },
            },
            "required": ["tool"],
        })
    }

    async fn invoke(&self, arguments: Value) -> Result<Value, ToolError> {
        let tool = arguments
            .get("tool")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError("missing string field tool".to_string()))?;
        let tool_arguments = arguments.get("arguments").cloned().unwrap_or(json!({}));
        let result = self
            .mcp
            .call_tool(tool, tool_arguments)
            .await
            .map_err(|e| ToolError(format!("mcp call failed: {e}")))?;
        let text: Vec<String> = result
            .content
            .iter()
            .map(|c| match c {
                McpContent::Text(t) => t.clone(),
            })
            .collect();
        Ok(json!({ "is_error": result.is_error, "content": text.join("\n") }))
    }
}
