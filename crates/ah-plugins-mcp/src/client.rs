//! 真实 MCP stdio 客户端:newline-delimited JSON-RPC 2.0 子进程传输。
//!
//! - 子进程:tokio::process::Command(stdin/stdout 管道,kill_on_drop);
//! - 线上协议:每行一个 JSON 对象;request 带 `id`,response 按 `id` 匹配;
//! - 真实握手:`initialize` 请求 -> 解析 serverInfo -> `notifications/initialized`;
//! - 真实关闭:`shutdown` 请求 -> `notifications/exit` -> 关闭 stdin(子进程读到
//!   EOF 自行退出)-> 有界等待子进程退出(超时 kill)。

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use ah_contracts::mcp::{McpClient, McpContent, McpError, McpInfo, McpTool, McpToolResult};
use ah_contracts::seam::Seam;
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

/// 一次请求/响应之间的独占传输锁 + 子进程管道。
struct ClientTransport {
    /// write 端;shutdown 时取出并 drop,子进程读到 EOF 后自行退出。
    stdin: tokio::sync::Mutex<Option<ChildStdin>>,
    reader: tokio::sync::Mutex<BufReader<ChildStdout>>,
    child: tokio::sync::Mutex<Option<Child>>,
    /// 串行化请求-响应交换(单飞)。
    request_lock: tokio::sync::Mutex<()>,
    next_id: AtomicU64,
    shutdown_sent: AtomicBool,
}

impl ClientTransport {
    fn spawn(
        command: &str,
        args: &[String],
        cwd: Option<&PathBuf>,
        env: &HashMap<String, String>,
    ) -> Result<Self, McpError> {
        let mut command_builder = Command::new(command);
        command_builder
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        if let Some(cwd) = cwd {
            command_builder.current_dir(cwd);
        }
        command_builder.envs(env);
        let mut child = command_builder
            .spawn()
            .map_err(|e| McpError(format!("spawn mcp server '{command}' failed: {e}")))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpError("mcp server stdin not piped".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpError("mcp server stdout not piped".to_string()))?;
        Ok(Self {
            stdin: tokio::sync::Mutex::new(Some(stdin)),
            reader: tokio::sync::Mutex::new(BufReader::new(stdout)),
            child: tokio::sync::Mutex::new(Some(child)),
            request_lock: tokio::sync::Mutex::new(()),
            next_id: AtomicU64::new(1),
            shutdown_sent: AtomicBool::new(false),
        })
    }

    /// 发送一个 request 并等待 id 匹配的 response(跳过通知与其他 id)。
    async fn request(&self, method: &str, params: Option<Value>) -> Result<Value, McpError> {
        let _lock = self.request_lock.lock().await;
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let mut request = json!({ "jsonrpc": "2.0", "id": id, "method": method });
        if let Some(params) = params {
            request["params"] = params;
        }
        let line = serde_json::to_string(&request)
            .map_err(|e| McpError(format!("serialize request failed: {e}")))?;
        self.send_line(&line).await?;
        self.read_response(id).await
    }

    /// 写一行 JSON + 换行到子进程 stdin。
    async fn send_line(&self, line: &str) -> Result<(), McpError> {
        let mut guard = self.stdin.lock().await;
        let stdin = guard
            .as_mut()
            .ok_or_else(|| McpError("mcp server stdin already closed".to_string()))?;
        let mut bytes = line.as_bytes().to_vec();
        bytes.push(b'\n');
        stdin
            .write_all(&bytes)
            .await
            .map_err(|e| McpError(format!("write to mcp server stdin failed: {e}")))?;
        stdin
            .flush()
            .await
            .map_err(|e| McpError(format!("flush mcp server stdin failed: {e}")))?;
        Ok(())
    }

    /// 逐行读 stdout,直到拿到 id 匹配的 response。
    async fn read_response(&self, expected_id: u64) -> Result<Value, McpError> {
        let mut reader = self.reader.lock().await;
        loop {
            let mut line = String::new();
            let read = reader
                .read_line(&mut line)
                .await
                .map_err(|e| McpError(format!("read from mcp server stdout failed: {e}")))?;
            if read == 0 {
                return Err(McpError(
                    "mcp server closed stdout while awaiting response".to_string(),
                ));
            }
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let message: Value = serde_json::from_str(line)
                .map_err(|e| McpError(format!("invalid JSON-RPC line from server: {e}")))?;
            // 通知(无 id)跳过。
            if message.get("id").is_none() {
                continue;
            }
            let id = message["id"]
                .as_u64()
                .ok_or_else(|| McpError("JSON-RPC response id is not a number".to_string()))?;
            if id != expected_id {
                continue;
            }
            // JSON-RPC 错误映射。
            if let Some(error) = message.get("error") {
                let code = error
                    .get("code")
                    .and_then(Value::as_i64)
                    .unwrap_or_default();
                let text = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown JSON-RPC error")
                    .to_string();
                return Err(McpError(format!("JSON-RPC error {code}: {text}")));
            }
            return Ok(message.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    /// 优雅关闭:shutdown 请求 + exit 通知 + 关闭 stdin + 有界等待退出。
    async fn shutdown(&self) -> Result<(), McpError> {
        if self.shutdown_sent.swap(true, Ordering::SeqCst) {
            return Ok(()); // 幂等。
        }
        self.request("shutdown", None).await?;
        self.send_line(&json!({ "jsonrpc": "2.0", "method": "notifications/exit" }).to_string())
            .await?;
        // 关闭 write 端(drop ChildStdin):子进程读 stdin 得到 EOF 后自行退出。
        drop(self.stdin.lock().await.take());
        let child = self.child.lock().await.take();
        if let Some(mut child) = child {
            match tokio::time::timeout(Duration::from_secs(3), child.wait()).await {
                Ok(Ok(_status)) => {}
                Ok(Err(e)) => {
                    return Err(McpError(format!("wait for mcp server exit failed: {e}")));
                }
                Err(_) => {
                    child
                        .kill()
                        .await
                        .map_err(|e| McpError(format!("kill mcp server failed: {e}")))?;
                    let _ = child.wait().await;
                    return Err(McpError(
                        "mcp server did not exit after shutdown; killed".to_string(),
                    ));
                }
            }
        }
        Ok(())
    }
}

/// 真实 MCP stdio 客户端:懒 spawn 子进程,首次方法调用时建立传输。
///
/// 构造不拉起进程(插件 apply / boot 不产生副作用);第一次调用
/// (initialize 等)才真实 spawn `command args` 并开始协议往返。
pub struct StdioMcpClient {
    command: String,
    args: Vec<String>,
    cwd: Option<PathBuf>,
    env: HashMap<String, String>,
    transport: tokio::sync::Mutex<Option<Arc<ClientTransport>>>,
}

impl StdioMcpClient {
    /// 以服务器命令与参数构造客户端(懒 spawn)。
    pub fn new(command: impl Into<String>, args: Vec<String>) -> Self {
        Self::new_with_options(command, args, None, HashMap::new())
    }

    /// 构造带工作目录和环境覆盖的 MCP 客户端。
    pub fn new_with_options(
        command: impl Into<String>,
        args: Vec<String>,
        cwd: Option<PathBuf>,
        env: HashMap<String, String>,
    ) -> Self {
        Self {
            command: command.into(),
            args,
            cwd,
            env,
            transport: tokio::sync::Mutex::new(None),
        }
    }

    async fn transport(&self) -> Result<Arc<ClientTransport>, McpError> {
        let mut slot = self.transport.lock().await;
        if slot.is_none() {
            *slot = Some(Arc::new(ClientTransport::spawn(
                &self.command,
                &self.args,
                self.cwd.as_ref(),
                &self.env,
            )?));
        }
        Ok(slot.as_ref().expect("just spawned").clone())
    }
}

impl Seam for StdioMcpClient {}

#[async_trait]
impl McpClient for StdioMcpClient {
    async fn initialize(&self) -> Result<McpInfo, McpError> {
        let transport = self.transport().await?;
        let result = transport
            .request(
                "initialize",
                Some(json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {
                        "name": "ah-plugins-mcp",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                })),
            )
            .await?;
        let protocol_version = result
            .get("protocolVersion")
            .and_then(Value::as_str)
            .ok_or_else(|| McpError("initialize result missing protocolVersion".to_string()))?
            .to_string();
        let server_name = result
            .get("serverInfo")
            .and_then(|info| info.get("name"))
            .and_then(Value::as_str)
            .ok_or_else(|| McpError("initialize result missing serverInfo.name".to_string()))?
            .to_string();
        // 完成真实握手:客户端通知。
        transport
            .send_line(
                &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string(),
            )
            .await?;
        Ok(McpInfo {
            protocol_version,
            server_name,
        })
    }

    async fn list_tools(&self) -> Result<Vec<McpTool>, McpError> {
        let transport = self.transport().await?;
        let result = transport.request("tools/list", None).await?;
        let tools = result
            .get("tools")
            .and_then(Value::as_array)
            .ok_or_else(|| McpError("tools/list result missing tools array".to_string()))?;
        tools
            .iter()
            .map(|entry| {
                serde_json::from_value::<McpTool>(entry.clone())
                    .map_err(|e| McpError(format!("invalid tool entry from server: {e}")))
            })
            .collect()
    }

    async fn call_tool(&self, name: &str, arguments: Value) -> Result<McpToolResult, McpError> {
        let transport = self.transport().await?;
        let result = transport
            .request(
                "tools/call",
                Some(json!({ "name": name, "arguments": arguments })),
            )
            .await?;
        let is_error = result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let content = result
            .get("content")
            .and_then(Value::as_array)
            .ok_or_else(|| McpError("tools/call result missing content array".to_string()))?;
        let mut parsed = Vec::with_capacity(content.len());
        for entry in content {
            match entry.get("type").and_then(Value::as_str) {
                Some("text") => {
                    let text = entry
                        .get("text")
                        .and_then(Value::as_str)
                        .ok_or_else(|| McpError("text content missing text field".to_string()))?
                        .to_string();
                    parsed.push(McpContent::Text(text));
                }
                Some(other) => {
                    return Err(McpError(format!("unsupported MCP content type: {other}")));
                }
                None => return Err(McpError("MCP content entry missing type field".to_string())),
            }
        }
        Ok(McpToolResult {
            content: parsed,
            is_error,
        })
    }

    async fn shutdown(&self) -> Result<(), McpError> {
        let transport = { self.transport.lock().await.clone() };
        match transport {
            // 从未 spawn:没有可关闭的进程,幂等成功。
            Some(transport) => transport.shutdown().await,
            None => Ok(()),
        }
    }
}
