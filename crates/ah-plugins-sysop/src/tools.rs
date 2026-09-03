//! 真实系统工具:包装 fs/shell provider 并注册到 tools seam。

use std::sync::Arc;
use std::time::Duration;

use ah_contracts::fs::{FsError, FsProvider};
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
        "read a file inside the workspace"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"],
        })
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
        "write a file inside the workspace"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "content": { "type": "string" },
            },
            "required": ["path", "content"],
        })
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
        "list entries inside the workspace"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
        })
    }
    fn idempotent(&self) -> bool {
        true
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
        "run a command in the workspace"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string" },
                "args": { "type": "array", "items": { "type": "string" } },
                "timeout_ms": { "type": "integer" },
            },
            "required": ["command"],
        })
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

/// edit 工具(P2-01):在 workspace 文件内做 old_string → new_string 替换。
///
/// 语义:读文件 → 未命中 old_string 显式报错(不落盘)→ 替换(首处或全部)→ 写回。
/// 结构化输出 `{path, replaced}`;写轴权限由 pre-execute rails(ApprovalRail 等)把关。
pub struct EditFileTool {
    fs: Arc<dyn FsProvider>,
}

impl EditFileTool {
    pub fn new(fs: Arc<dyn FsProvider>) -> Self {
        Self { fs }
    }
}

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &'static str {
        "edit"
    }

    fn description(&self) -> &'static str {
        "replace old_string with new_string in a workspace file"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "old_string": { "type": "string" },
                "new_string": { "type": "string" },
                "replace_all": { "type": "boolean" },
            },
            "required": ["path", "old_string"],
        })
    }

    async fn invoke(&self, arguments: Value) -> Result<Value, ToolError> {
        let path = arguments
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError("missing string field path".to_string()))?;
        let old_string = arguments
            .get("old_string")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError("missing string field old_string".to_string()))?;
        let new_string = arguments
            .get("new_string")
            .and_then(Value::as_str)
            .unwrap_or("");
        let replace_all = arguments
            .get("replace_all")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        if old_string.is_empty() {
            return Err(ToolError("old_string must not be empty".to_string()));
        }
        let content = self
            .fs
            .read(path)
            .map_err(|e| ToolError(format!("read failed: {e}")))?;
        let text = String::from_utf8_lossy(&content).into_owned();
        let replaced = if replace_all {
            text.matches(old_string).count()
        } else if text.contains(old_string) {
            1
        } else {
            0
        };
        if replaced == 0 {
            return Err(ToolError(format!("old_string not found in {path}")));
        }
        let updated = if replace_all {
            text.replace(old_string, new_string)
        } else {
            text.replacen(old_string, new_string, 1)
        };
        self.fs
            .write(path, updated.as_bytes())
            .map_err(|e| ToolError(format!("write failed: {e}")))?;
        Ok(json!({ "path": path, "replaced": replaced }))
    }
}

/// glob 工具(P2-01):在 workspace 内递归查找匹配 `*` / `?` 通配的文件/目录。
///
/// 模式不含 `/` 时匹配条目名;含 `/` 时匹配相对路径。递归本身已隐含 `**` 语义。
/// 结构化输出 `{path, matches}`(排序、相对路径)。
pub struct GlobTool {
    fs: Arc<dyn FsProvider>,
}

impl GlobTool {
    pub fn new(fs: Arc<dyn FsProvider>) -> Self {
        Self { fs }
    }

    fn walk(&self, rel: &str, out: &mut Vec<String>) -> Result<(), FsError> {
        let entries = self.fs.list(rel)?;
        for entry in entries {
            let child = if rel == "." {
                entry.clone()
            } else {
                format!("{rel}/{entry}")
            };
            out.push(child.clone());
            // 目录可继续 list;文件 list 报错即视为叶子。
            if self.fs.list(&child).is_ok() {
                self.walk(&child, out)?;
            }
        }
        Ok(())
    }
}

/// `*` 匹配任意序列(含空),`?` 匹配单字符。
fn glob_match(pattern: &str, text: &str) -> bool {
    fn rec(p: &[u8], t: &[u8]) -> bool {
        match p.first() {
            None => t.is_empty(),
            Some(b'*') => rec(&p[1..], t) || (!t.is_empty() && rec(p, &t[1..])),
            Some(&pc) => match t.first() {
                Some(&tc) if pc == b'?' || pc == tc => rec(&p[1..], &t[1..]),
                _ => false,
            },
        }
    }
    rec(pattern.as_bytes(), text.as_bytes())
}

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &'static str {
        "glob"
    }

    fn description(&self) -> &'static str {
        "find workspace paths matching a glob pattern (* and ?)"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string" },
                "path": { "type": "string" },
            },
            "required": ["pattern"],
        })
    }

    async fn invoke(&self, arguments: Value) -> Result<Value, ToolError> {
        let pattern = arguments
            .get("pattern")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError("missing string field pattern".to_string()))?;
        let root = arguments.get("path").and_then(Value::as_str).unwrap_or(".");
        let mut all = Vec::new();
        self.walk(root, &mut all)
            .map_err(|e| ToolError(format!("walk failed: {e}")))?;
        let matches: Vec<String> = all
            .into_iter()
            .filter(|path| {
                if pattern.contains('/') {
                    glob_match(pattern, path)
                } else {
                    path.rsplit('/')
                        .next()
                        .map(|base| glob_match(pattern, base))
                        .unwrap_or(false)
                }
            })
            .collect();
        Ok(json!({ "path": root, "matches": matches }))
    }
}

/// grep 工具(P2-01):在 workspace 内递归按正则/子串搜索文件内容。
///
/// 结构化输出 `{pattern, matches: [{path, line_number, line}]}`,上限 100 条;
/// 非法正则显式报错。读取轴权限由 pre-execute rails 把关。
pub struct GrepTool {
    fs: Arc<dyn FsProvider>,
}

impl GrepTool {
    pub fn new(fs: Arc<dyn FsProvider>) -> Self {
        Self { fs }
    }
}

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &'static str {
        "grep"
    }

    fn description(&self) -> &'static str {
        "search file contents in the workspace for a regex pattern"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string" },
                "path": { "type": "string" },
                "literal": { "type": "boolean" },
            },
            "required": ["pattern"],
        })
    }

    async fn invoke(&self, arguments: Value) -> Result<Value, ToolError> {
        let pattern = arguments
            .get("pattern")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError("missing string field pattern".to_string()))?;
        let root = arguments.get("path").and_then(Value::as_str).unwrap_or(".");
        let literal = arguments
            .get("literal")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let regex = if literal {
            regex::Regex::new(&regex::escape(pattern))
        } else {
            regex::Regex::new(pattern)
        }
        .map_err(|e| ToolError(format!("invalid regex: {e}")))?;

        let mut all = Vec::new();
        GlobTool::new(self.fs.clone())
            .walk(root, &mut all)
            .map_err(|e| ToolError(format!("walk failed: {e}")))?;
        let mut matches: Vec<Value> = Vec::new();
        for path in all {
            if matches.len() >= 100 {
                break;
            }
            let Ok(bytes) = self.fs.read(&path) else {
                continue;
            };
            let text = String::from_utf8_lossy(&bytes);
            for (idx, line) in text.lines().enumerate() {
                if regex.is_match(line) {
                    matches.push(json!({
                        "path": path,
                        "line_number": idx + 1,
                        "line": line,
                    }));
                    if matches.len() >= 100 {
                        break;
                    }
                }
            }
        }
        Ok(json!({
            "pattern": pattern,
            "match_count": matches.len(),
            "matches": matches,
        }))
    }
}
