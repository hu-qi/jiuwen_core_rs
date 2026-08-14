//! mcp seam:Model Context Protocol 客户端契约。
//!
//! 只声明类型与接口(零实现)。真实 stdio 子进程实现位于 `ah-plugins-mcp`。
//!
//! 协议形态(newline-delimited JSON-RPC 2.0):
//! - 每行一个 JSON 对象;request 带 `id`,response 按 `id` 匹配;
//! - `initialize` 握手后客户端发送 `notifications/initialized`;
//! - `shutdown` 请求 + `notifications/exit` 后关闭 stdin 并等待子进程退出。

use async_trait::async_trait;
use serde_json::Value;

use crate::seam::Seam;

/// 一个由 MCP server 暴露的工具。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct McpTool {
    pub name: String,
    /// MCP 规范中 description 可选;缺省为空字符串。
    #[serde(default)]
    pub description: String,
    /// 参数 JSON Schema(线上字段名为 `inputSchema`)。
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

/// 一次 `tools/call` 的结果内容。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct McpToolResult {
    pub content: Vec<McpContent>,
    /// MCP 规范中 `isError` 缺省为 false。
    #[serde(rename = "isError", default)]
    pub is_error: bool,
}

/// MCP 内容块(线上形态:`{"type":"text","text":"..."}`)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", content = "text", rename_all = "snake_case")]
pub enum McpContent {
    Text(String),
}

/// `initialize` 握手结果(客户端视角的摘要)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct McpInfo {
    pub protocol_version: String,
    pub server_name: String,
}

/// MCP 客户端错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpError(pub String);

impl core::fmt::Display for McpError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for McpError {}

/// MCP Seam(Service Definition):stdio/http 传输之上的客户端接口。
///
/// 实现方负责真实子进程/网络与 newline-delimited JSON-RPC 2.0 往返;
/// 消费方只依赖本 trait,不 import 具体实现。
#[async_trait]
pub trait McpClient: Seam {
    /// 握手:`initialize` 请求 + `notifications/initialized` 通知。
    async fn initialize(&self) -> Result<McpInfo, McpError>;

    /// 列出 server 暴露的工具。
    async fn list_tools(&self) -> Result<Vec<McpTool>, McpError>;

    /// 调用一个工具;参数为 JSON 对象。
    async fn call_tool(&self, name: &str, arguments: Value) -> Result<McpToolResult, McpError>;

    /// 关闭连接:`shutdown` 请求 + `notifications/exit`,等待子进程退出。
    async fn shutdown(&self) -> Result<(), McpError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn mcp_content_serde_roundtrip() {
        let content = McpContent::Text("hello".to_string());
        let wire = serde_json::to_value(&content).expect("serialize");
        // 线上形态必须与 MCP 规范一致。
        assert_eq!(wire, json!({ "type": "text", "text": "hello" }));
        let back: McpContent = serde_json::from_value(wire).expect("deserialize");
        assert_eq!(back, content);
    }

    #[test]
    fn mcp_tool_serde_uses_input_schema_wire_name() {
        let tool = McpTool {
            name: "echo".to_string(),
            description: "echo back".to_string(),
            input_schema: json!({ "type": "object" }),
        };
        let wire = serde_json::to_value(&tool).expect("serialize");
        assert_eq!(wire["name"], "echo");
        assert_eq!(wire["inputSchema"]["type"], "object");
        assert!(wire.get("input_schema").is_none());
        let back: McpTool = serde_json::from_value(wire).expect("deserialize");
        assert_eq!(back, tool);
    }

    #[test]
    fn mcp_tool_result_is_error_defaults_false() {
        let wire = json!({ "content": [{ "type": "text", "text": "ok" }] });
        let result: McpToolResult = serde_json::from_value(wire).expect("deserialize");
        assert!(!result.is_error);
        assert_eq!(result.content, vec![McpContent::Text("ok".to_string())]);
    }

    #[test]
    fn mcp_info_roundtrip() {
        let info = McpInfo {
            protocol_version: "2024-11-05".to_string(),
            server_name: "fake-mcp-server".to_string(),
        };
        let wire = serde_json::to_value(&info).expect("serialize");
        let back: McpInfo = serde_json::from_value(wire).expect("deserialize");
        assert_eq!(back, info);
    }

    #[test]
    fn mcp_error_displays_message() {
        let error = McpError("boom".to_string());
        assert_eq!(error.to_string(), "boom");
    }
}
