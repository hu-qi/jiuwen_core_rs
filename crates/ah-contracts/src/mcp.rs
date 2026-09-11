//! mcp seam:Model Context Protocol 客户端契约。
//!
//! 只声明类型与接口(零实现)。真实 stdio 子进程实现位于 `ah-plugins-mcp`。
//!
//! 协议形态(newline-delimited JSON-RPC 2.0):
//! - 每行一个 JSON 对象;request 带 `id`,response 按 `id` 匹配;
//! - `initialize` 握手后客户端发送 `notifications/initialized`;
//! - `shutdown` 请求 + `notifications/exit` 后关闭 stdin 并等待子进程退出。

use async_trait::async_trait;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::time::Duration;

use crate::seam::Seam;

/// MCP tool calls are bounded by default so an unresponsive server cannot
/// suspend an agent turn indefinitely.
pub const DEFAULT_MCP_TOOL_CALL_TIMEOUT: Duration = Duration::from_secs(30);
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

/// 一次 MCP 工具调用返回的内容块。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpContent {
    Text(String),
    Image { mime_type: String, data: String },
}

impl Serialize for McpContent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Text(text) => {
                serde_json::json!({"type":"text","text":text}).serialize(serializer)
            }
            Self::Image { mime_type, data } => {
                serde_json::json!({"type":"image","data":data,"mimeType":mime_type})
                    .serialize(serializer)
            }
        }
    }
}

impl<'de> Deserialize<'de> for McpContent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        match value.get("type").and_then(Value::as_str) {
            Some("text") => value
                .get("text")
                .and_then(Value::as_str)
                .map(|text| Self::Text(text.to_string()))
                .ok_or_else(|| serde::de::Error::custom("text content missing text")),
            Some("image") => {
                let data = value
                    .get("data")
                    .and_then(Value::as_str)
                    .ok_or_else(|| serde::de::Error::custom("image content missing data"))?;
                let mime_type = value
                    .get("mimeType")
                    .and_then(Value::as_str)
                    .ok_or_else(|| serde::de::Error::custom("image content missing mimeType"))?;
                Ok(Self::Image {
                    mime_type: mime_type.to_string(),
                    data: data.to_string(),
                })
            }
            Some(kind) => Err(serde::de::Error::custom(format!(
                "unsupported MCP content type: {kind}"
            ))),
            None => Err(serde::de::Error::custom("MCP content missing type")),
        }
    }
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

    /// 调用一个工具;参数为 JSON 对象,使用默认有界超时。
    async fn call_tool(&self, name: &str, arguments: Value) -> Result<McpToolResult, McpError> {
        self.call_tool_with_timeout(name, arguments, DEFAULT_MCP_TOOL_CALL_TIMEOUT)
            .await
    }

    /// 调用一个工具并为本次调用指定超时。
    async fn call_tool_with_timeout(
        &self,
        name: &str,
        arguments: Value,
        timeout: Duration,
    ) -> Result<McpToolResult, McpError>;

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
    #[test]
    fn mcp_image_content_roundtrips_standard_wire_shape() {
        let content = McpContent::Image {
            mime_type: "image/png".to_string(),
            data: "AAAA".to_string(),
        };
        let wire = serde_json::to_value(&content).expect("serialize image");
        assert_eq!(
            wire,
            json!({"type":"image","data":"AAAA","mimeType":"image/png"})
        );
        assert_eq!(serde_json::from_value::<McpContent>(wire).unwrap(), content);
    }
}
