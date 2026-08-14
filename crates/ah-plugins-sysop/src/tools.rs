//! 真实系统工具:包装 fs/shell provider 并注册到 tools seam。

use std::sync::Arc;
use std::time::Duration;

use ah_contracts::fs::FsProvider;
use ah_contracts::shell::ShellProvider;
use ah_contracts::tools::{Tool, ToolError};
use async_trait::async_trait;
use serde_json::{Value, json};

/// read_file 工具:读取 workspace 内文件。
pub struct ReadFileTool {
    fs: Arc<dyn FsProvider>,
}

impl ReadFileTool {
    pub fn new(fs: Arc<dyn FsProvider>) -> Self {
        Self { fs }
    }
}

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &'static str {
        "read_file"
    }

    fn description(&self) -> &'static str {
        "read a file inside the workspace; arguments: {path}"
    }

    async fn invoke(&self, arguments: Value) -> Result<Value, ToolError> {
        let path = arguments
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError("missing string field path".to_string()))?;
        let content = self
            .fs
            .read(path)
            .map_err(|e| ToolError(format!("read failed: {e}")))?;
        let text = String::from_utf8_lossy(&content).into_owned();
        Ok(json!({ "path": path, "content": text }))
    }
}

/// write_file 工具:写入 workspace 内文件。
pub struct WriteFileTool {
    fs: Arc<dyn FsProvider>,
}

impl WriteFileTool {
    pub fn new(fs: Arc<dyn FsProvider>) -> Self {
        Self { fs }
    }
}

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &'static str {
        "write_file"
    }

    fn description(&self) -> &'static str {
        "write a file inside the workspace; arguments: {path, content}"
    }

    async fn invoke(&self, arguments: Value) -> Result<Value, ToolError> {
        let path = arguments
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError("missing string field path".to_string()))?;
        let content = arguments
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError("missing string field content".to_string()))?;
        self.fs
            .write(path, content.as_bytes())
            .map_err(|e| ToolError(format!("write failed: {e}")))?;
        Ok(json!({ "path": path, "written": content.len() }))
    }
}

/// list_dir 工具:列出 workspace 内目录条目。
pub struct ListDirTool {
    fs: Arc<dyn FsProvider>,
}

impl ListDirTool {
    pub fn new(fs: Arc<dyn FsProvider>) -> Self {
        Self { fs }
    }
}

#[async_trait]
impl Tool for ListDirTool {
    fn name(&self) -> &'static str {
        "list_dir"
    }

    fn description(&self) -> &'static str {
        "list entries inside the workspace; arguments: {path}"
    }

    async fn invoke(&self, arguments: Value) -> Result<Value, ToolError> {
        let path = arguments.get("path").and_then(Value::as_str).unwrap_or(".");
        let entries = self
            .fs
            .list(path)
            .map_err(|e| ToolError(format!("list failed: {e}")))?;
        Ok(json!({ "path": path, "entries": entries }))
    }
}

/// run_shell 工具:在受限工作目录执行命令。
pub struct ShellTool {
    shell: Arc<dyn ShellProvider>,
}

impl ShellTool {
    pub fn new(shell: Arc<dyn ShellProvider>) -> Self {
        Self { shell }
    }
}

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &'static str {
        "run_shell"
    }

    fn description(&self) -> &'static str {
        "run a command in the workspace; arguments: {command, args?, timeout_ms?}"
    }

    async fn invoke(&self, arguments: Value) -> Result<Value, ToolError> {
        let command = arguments
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError("missing string field command".to_string()))?;
        let args: Vec<String> = arguments
            .get("args")
            .and_then(Value::as_array)
            .map(|array| {
                array
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let timeout_ms = arguments
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .unwrap_or(30_000);
        let output = self
            .shell
            .run(command, &args, Duration::from_millis(timeout_ms))
            .await
            .map_err(|e| ToolError(format!("shell failed: {e}")))?;
        Ok(json!({
            "exit_code": output.exit_code,
            "stdout": output.stdout,
            "stderr": output.stderr,
        }))
    }
}
