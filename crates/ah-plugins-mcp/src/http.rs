//! MCP HTTP 传输(streamable-HTTP 简化版:每请求一个 POST JSON-RPC 2.0)。
//!
//! 与 stdio 传输同一 McpClient seam:initialize / tools/list / tools/call /
//! shutdown 经真实 HTTP POST 发送到远端 MCP 端点。

use std::time::Duration;

use ah_contracts::mcp::{McpClient, McpError, McpInfo, McpTool, McpToolResult};
use ah_contracts::seam::Seam;
use async_trait::async_trait;
use serde_json::{Value, json};

/// 真实 MCP HTTP 客户端。
pub struct McpHttpClient {
    url: String,
    agent: ureq::Agent,
}

impl McpHttpClient {
    /// 以远端端点 URL 创建(如 http://127.0.0.1:port/)。
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(10))
                .build(),
        }
    }

    /// POST 一个 JSON-RPC 2.0 请求,返回 result 或显式错误。
    async fn call(&self, method: &str, params: Value) -> Result<Value, McpError> {
        self.call_with_timeout(method, params, Duration::from_secs(10))
            .await
    }

    async fn call_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, McpError> {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });
        let agent = self.agent.clone();
        let url = self.url.clone();
        let payload = serde_json::to_string(&request).expect("serialize");
        let body = tokio::task::spawn_blocking(move || {
            agent
                .post(&url)
                .timeout(timeout)
                .set("Content-Type", "application/json")
                .send_string(&payload)
                .map_err(|e| McpError(format!("mcp http call failed: {e}")))?
                .into_string()
                .map_err(|e| McpError(format!("read mcp http body failed: {e}")))
        });
        let body = tokio::time::timeout(timeout, body)
            .await
            .map_err(|_| McpError(format!("mcp http call timed out after {timeout:?}")))?
            .map_err(|e| McpError(format!("mcp http task failed: {e}")))??;
        let value: Value = serde_json::from_str(&body)
            .map_err(|e| McpError(format!("parse mcp http body: {e}")))?;
        if let Some(error) = value.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            return Err(McpError(format!("mcp rpc error: {message}")));
        }
        value
            .get("result")
            .cloned()
            .ok_or_else(|| McpError("mcp response missing result".to_string()))
    }
}

impl Seam for McpHttpClient {}

#[async_trait]
impl McpClient for McpHttpClient {
    async fn initialize(&self) -> Result<McpInfo, McpError> {
        let result = self
            .call(
                "initialize",
                json!({
                    "protocolVersion": "2025-03-26",
                    "capabilities": {},
                    "clientInfo": { "name": "agent-harness", "version": "0.1" },
                }),
            )
            .await?;
        Ok(McpInfo {
            protocol_version: result
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            server_name: result
                .get("serverInfo")
                .and_then(|s| s.get("name"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        })
    }

    async fn list_tools(&self) -> Result<Vec<McpTool>, McpError> {
        let result = self.call("tools/list", json!({})).await?;
        serde_json::from_value(result.get("tools").cloned().unwrap_or(Value::Null))
            .map_err(|e| McpError(format!("parse tools/list: {e}")))
    }

    async fn call_tool_with_timeout(
        &self,
        name: &str,
        arguments: Value,
        timeout: Duration,
    ) -> Result<McpToolResult, McpError> {
        let result = self
            .call_with_timeout(
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
                timeout,
            )
            .await
            .map_err(|error| {
                if error.0.contains("timed out") {
                    McpError(format!("MCP tool '{name}' timed out after {timeout:?}"))
                } else {
                    error
                }
            })?;
        serde_json::from_value(result).map_err(|e| McpError(format!("parse tools/call: {e}")))
    }

    async fn shutdown(&self) -> Result<(), McpError> {
        let _ = self.call("shutdown", json!({})).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// 真实本地 MCP HTTP 端点(测试服务器):手工 HTTP/1.1 + JSON-RPC 分发。
    struct TestMcpServer;

    impl TestMcpServer {
        fn spawn() -> (Self, String) {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let addr = listener.local_addr().expect("addr");
            let url = format!("http://{addr}/");
            let handle = listener.try_clone().expect("clone");
            std::thread::spawn(move || {
                for stream in handle.incoming().flatten() {
                    let _ = handle_conn(stream);
                }
            });
            // listener 需保持存活:交给 spawn 的线程持有克隆即可,原 listener 在此 drop 前
            // 由变量保活到函数返回;测试期间线程持续 accept 克隆句柄。
            let _ = &listener;
            (Self, url)
        }
    }

    fn handle_conn(mut stream: std::net::TcpStream) -> Result<(), McpError> {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        // 阶段 1:读至请求头结束(CRLF 分隔符出现为止)。
        loop {
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
            if buf.len() > 64 * 1024 {
                break;
            }
            let n = stream
                .read(&mut chunk)
                .map_err(|e| McpError(e.to_string()))?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        // 阶段 2:续读至 Content-Length 声明字节完整到达。
        let header_end = buf
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|i| i + 4)
            .unwrap_or(buf.len());
        let headers = String::from_utf8_lossy(&buf[..header_end]).into_owned();
        let mut content_length = 0usize;
        for line in headers.lines().skip(1) {
            if let Some((name, value)) = line.split_once(':')
                && name.trim().eq_ignore_ascii_case("content-length")
            {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
        while buf.len() < header_end + content_length {
            let n = stream
                .read(&mut chunk)
                .map_err(|e| McpError(e.to_string()))?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        let body_bytes = &buf[header_end..];
        let body: String = body_bytes
            .iter()
            .take(content_length)
            .map(|&b| b as char)
            .collect();
        let req: Value = serde_json::from_str(body.trim()).unwrap_or(Value::Null);
        let method = req
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if method == "tools/call" && req["params"]["name"].as_str() == Some("hang") {
            std::thread::sleep(Duration::from_millis(100));
        }
        let result = match method {
            "initialize" => json!({
                "protocolVersion": "2025-03-26",
                "serverInfo": { "name": "test-mcp-server", "version": "1.0" },
            }),
            "tools/list" => json!({ "tools": [
                { "name": "echo", "description": "echo input", "inputSchema": { "type": "object" } }
            ]}),
            "tools/call" => json!({
                "content": [{ "type": "text", "text": "echoed" }],
                "isError": false,
            }),
            "shutdown" => json!({}),
            other => {
                let resp = json!({
                    "jsonrpc": "2.0", "id": req.get("id").cloned().unwrap_or(Value::Null),
                    "error": { "code": -32601, "message": format!("method not found: {other}") }
                });
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    resp.to_string().len(),
                    resp
                );
                stream
                    .write_all(resp.as_bytes())
                    .map_err(|e| McpError(e.to_string()))?;
                return Ok(());
            }
        };
        let resp = json!({ "jsonrpc": "2.0", "id": req.get("id").cloned().unwrap_or(Value::Null), "result": result });
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            resp.to_string().len(),
            resp
        );
        stream
            .write_all(resp.as_bytes())
            .map_err(|e| McpError(e.to_string()))
    }

    #[tokio::test]
    async fn http_client_full_roundtrip() {
        let (_server, url) = TestMcpServer::spawn();
        let client = McpHttpClient::new(url);

        let info = client.initialize().await.expect("initialize");
        assert_eq!(info.protocol_version, "2025-03-26");
        assert_eq!(info.server_name, "test-mcp-server");

        let tools = client.list_tools().await.expect("list");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "echo");

        let result = client
            .call_tool("echo", serde_json::json!({ "x": 1 }))
            .await
            .expect("call");
        assert!(!result.is_error);
        assert!(
            matches!(result.content[0], ah_contracts::mcp::McpContent::Text(ref t) if t == "echoed")
        );

        client.shutdown().await.expect("shutdown");
    }

    #[tokio::test]
    async fn unknown_method_errors_explicitly() {
        let (_server, url) = TestMcpServer::spawn();
        let client = McpHttpClient::new(url);
        // 直接构造未知方法:错误须显式传播。
        let err = client
            .call("no/suchMethod", json!({}))
            .await
            .expect_err("unknown");
        assert!(err.0.contains("method not found"));
    }

    #[tokio::test]
    async fn tool_call_timeout_bounds_hanging_http_server() {
        let (_server, url) = TestMcpServer::spawn();
        let client = McpHttpClient::new(url);
        let error = client
            .call_tool_with_timeout("hang", json!({}), Duration::from_millis(20))
            .await
            .expect_err("hanging HTTP tool must time out");
        assert!(error.0.contains("hang"), "got: {}", error.0);
        assert!(error.0.contains("20ms"), "got: {}", error.0);
    }
}
