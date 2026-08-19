//! lsp seam:语言服务器子系统的确定性契约。
//!
//! 对齐 `openjiuwen/harness/lsp/` 的确定性部分:
//! - types:LspServerState 状态机(5 态 + 迁移边 + 崩溃恢复阈值)、
//!   SpawnHandle / ScopedLspServerConfig / LspServerStatus / InitializeOptions;
//! - diagnostic_registry:LspDiagnosticItem / LspDiagnosticFile、`_diag_key` 去重键、
//!   `get_and_clear` 六步算法(合并去重 → 跨轮去重 → severity 排序 → 双上限 → 历史);
//! - file_uri:file:// URI ↔ 路径转换;
//! - servers 注册表:5 语言 server 配置(go/java/python/rust/typescript)。
//!
//! 子进程 stdio JSON-RPC(spawn / 读写 / 握手)与 gitignore 过滤依赖进程与 FS,
//! 由插件注入命令执行器实现;本契约只定义纯类型与判定逻辑。

use std::collections::BTreeMap;

use crate::seam::Seam;

/// LSP 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LspError(pub String);

impl core::fmt::Display for LspError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LspError {}

/// 崩溃恢复尝试上限(对齐 constants.py:25)。
pub const MAX_CRASH_RECOVERY_ATTEMPTS: u32 = 3;
/// 每文件诊断默认上限(对齐 diagnostic_registry.py:28)。
pub const MAX_DIAG_PER_FILE: usize = 10;
/// 全局诊断默认上限(对齐 diagnostic_registry.py:29)。
pub const MAX_DIAG_TOTAL: usize = 30;
/// 默认启动超时毫秒(对齐 core/types.py:50)。
pub const DEFAULT_STARTUP_TIMEOUT_MS: i64 = 45_000;

/// LSP server 生命周期状态(对齐 `LspServerState`)。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LspServerState {
    #[default]
    Stopped,
    Starting,
    Running,
    Stopping,
    Error,
}

impl LspServerState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Error => "error",
        }
    }
}

/// spawn 参数(对齐 `SpawnHandle`)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SpawnHandle {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub initialization_options: Option<serde_json::Value>,
    #[serde(default = "default_startup_timeout")]
    pub startup_timeout: i64,
}

fn default_startup_timeout() -> i64 {
    DEFAULT_STARTUP_TIMEOUT_MS
}

impl SpawnHandle {
    /// 构造 spawn 参数(默认启动超时 45s)。
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            env: BTreeMap::new(),
            initialization_options: None,
            startup_timeout: DEFAULT_STARTUP_TIMEOUT_MS,
        }
    }
}

/// 单个 LSP server 的完整配置(对齐 `ScopedLspServerConfig`)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScopedLspServerConfig {
    pub server_id: String,
    pub command: String,
    pub workspace_folder: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub initialization_options: Option<serde_json::Value>,
    #[serde(default = "default_startup_timeout")]
    pub startup_timeout: i64,
    #[serde(default)]
    pub extension_to_language: BTreeMap<String, String>,
}

/// server 运行状态快照(对齐 `LspServerStatus`)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LspServerStatus {
    pub server_id: String,
    pub name: String,
    pub running: bool,
    #[serde(default)]
    pub state: LspServerState,
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub crash_count: u32,
    #[serde(default)]
    pub last_error: Option<String>,
}

/// 初始化选项(对齐 `types.InitializeOptions`)。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InitializeOptions {
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub languages: Vec<String>,
}

/// 一条 LSP 诊断(对齐 `LspDiagnosticItem`)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LspDiagnosticItem {
    pub message: String,
    /// 1=Error, 2=Warning, 3=Info, 4=Hint。
    pub severity: i64,
    pub range: serde_json::Value,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub code: Option<serde_json::Value>,
}

/// 一个文件的诊断交付(对齐 `LspDiagnosticFile`)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LspDiagnosticFile {
    pub uri: String,
    #[serde(default)]
    pub diagnostics: Vec<LspDiagnosticItem>,
    #[serde(default)]
    pub server_name: String,
    #[serde(default)]
    pub local_path: String,
}

/// 解析原始诊断条目(对齐 `_parse_raw`):非 dict 丢弃、空消息丢弃、
/// severity 缺失默认 3、range 缺失 {}、source 假值 None、code 原样。
pub fn parse_raw_diagnostic(raw: &serde_json::Value) -> Option<LspDiagnosticItem> {
    let obj = raw.as_object()?;
    let message = obj.get("message")?.as_str()?;
    if message.is_empty() {
        return None;
    }
    let severity = obj.get("severity").and_then(|v| v.as_i64()).unwrap_or(3);
    let range = obj
        .get("range")
        .cloned()
        .unwrap_or(serde_json::Value::Object(Default::default()));
    let source = obj
        .get("source")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let code = obj.get("code").cloned().filter(|c| !c.is_null());
    Some(LspDiagnosticItem {
        message: message.to_string(),
        severity,
        range,
        source,
        code,
    })
}

/// 稳定去重键(对齐 `_diag_key`):
/// `message|severity|line:char|code`,range 非 dict 或 start 缺失取 0。
pub fn diag_key(item: &LspDiagnosticItem) -> String {
    let (line, char_) = range_start(&item.range);
    let code_str = item
        .code
        .as_ref()
        .and_then(|c| c.as_str())
        .map(str::to_string)
        .or_else(|| item.code.as_ref().map(|c| c.to_string()))
        .unwrap_or_default();
    format!(
        "{}|{}|{}:{}|{}",
        item.message, item.severity, line, char_, code_str
    )
}

fn range_start(range: &serde_json::Value) -> (i64, i64) {
    let start = range.get("start").and_then(|s| s.as_object());
    let line = start
        .and_then(|s| s.get("line"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let char_ = start
        .and_then(|s| s.get("character"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    (line, char_)
}

/// 诊断注册表(对齐 `LspDiagnosticRegistry` 六步算法)。
///
/// 进程级单例;`register` 与 `get_and_clear` 顺序一致(调用方保证串行)。
#[derive(Debug, Default)]
pub struct LspDiagnosticRegistry {
    /// batch_id → (server_name, uri, items);保留插入顺序。
    pending: Vec<(String, String, Vec<LspDiagnosticItem>)>,
    /// uri → 已交付去重键(只增,clear_all 重置)。
    delivered: BTreeMap<String, Vec<String>>,
}

impl LspDiagnosticRegistry {
    /// 构造空注册表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 待交付批次数(对齐 `pending_count`)。
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// 注册一批原始诊断(对齐 `register`):无有效条目返回 None(空 batch_id)。
    pub fn register(
        &mut self,
        server_name: &str,
        uri: &str,
        raw_diagnostics: &[serde_json::Value],
    ) -> Option<String> {
        let items: Vec<LspDiagnosticItem> = raw_diagnostics
            .iter()
            .filter_map(parse_raw_diagnostic)
            .collect();
        if items.is_empty() {
            return None;
        }
        let batch_id = format!("batch-{}", self.pending.len());
        self.pending
            .push((server_name.to_string(), uri.to_string(), items));
        Some(batch_id)
    }

    /// 取走待交付诊断(对齐 `get_and_clear` 六步算法)。
    pub fn get_and_clear(
        &mut self,
        max_per_file: usize,
        max_total: usize,
    ) -> Vec<LspDiagnosticFile> {
        if self.pending.is_empty() {
            return Vec::new();
        }
        // 1) 按 uri 合并批次 + 轮内去重(保序,server_name 取首个)。
        let mut by_uri: Vec<(String, String, Vec<LspDiagnosticItem>)> = Vec::new();
        {
            let batches = std::mem::take(&mut self.pending);
            for (server_name, uri, items) in batches {
                if let Some((_, _, existing)) = by_uri.iter_mut().find(|(_, u, _)| *u == uri) {
                    for item in items {
                        let key = diag_key(&item);
                        if !existing.iter().any(|e| diag_key(e) == key) {
                            existing.push(item);
                        }
                    }
                } else {
                    by_uri.push((server_name, uri, items));
                }
            }
        }
        // 2) 跨轮去重:仅保留未交付键。
        let mut fresh: Vec<(String, String, Vec<LspDiagnosticItem>)> = Vec::new();
        for (server_name, uri, items) in by_uri {
            let delivered = self.delivered.get(&uri).cloned().unwrap_or_default();
            let kept: Vec<LspDiagnosticItem> = items
                .into_iter()
                .filter(|item| !delivered.contains(&diag_key(item)))
                .collect();
            if !kept.is_empty() {
                fresh.push((server_name, uri, kept));
            }
        }
        if fresh.is_empty() {
            return Vec::new();
        }
        // 3) severity 升序稳定排序;4) 每文件上限。
        for (_, _, items) in fresh.iter_mut() {
            items.sort_by_key(|item| item.severity);
            if items.len() > max_per_file {
                items.truncate(max_per_file);
            }
        }
        // 5) 全局上限(按 uri 插入顺序)。
        let mut out = Vec::new();
        let mut total = 0usize;
        for (server_name, uri, items) in fresh {
            if total >= max_total {
                break;
            }
            let remaining = max_total.saturating_sub(total);
            let clipped: Vec<LspDiagnosticItem> = items.into_iter().take(remaining).collect();
            let local_path = file_uri_to_path(&uri).unwrap_or_else(|| uri.clone());
            out.push(LspDiagnosticFile {
                uri: uri.clone(),
                diagnostics: clipped,
                server_name,
                local_path,
            });
            total += out.last().map(|f| f.diagnostics.len()).unwrap_or(0);
        }
        // 6) 更新交付历史。
        for file in &out {
            let entry = self.delivered.entry(file.uri.clone()).or_default();
            for item in &file.diagnostics {
                let key = diag_key(item);
                if !entry.contains(&key) {
                    entry.push(key);
                }
            }
        }
        out
    }

    /// 完全重置(对齐 `clear_all`)。
    pub fn clear_all(&mut self) {
        self.pending.clear();
        self.delivered.clear();
    }
}

/// file:// URI → 路径(对齐 `file_uri.file_uri_to_path` 确定性部分,Unix 分支)。
///
/// 去掉 `file://` 前缀后 unquote 百分号编码;非 file:// 原样返回。
pub fn file_uri_to_path(uri: &str) -> Option<String> {
    if !uri.starts_with("file://") {
        return Some(uri.to_string());
    }
    let path = &uri[7..];
    // 百分号解码(简化:%XX → 字节;畸形保留原样)。
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let mut malformed = false;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                decoded.push(hi * 16 + lo);
                i += 3;
                continue;
            }
            malformed = true;
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    let path = if malformed {
        path.to_string()
    } else {
        String::from_utf8_lossy(&decoded).to_string()
    };
    Some(path)
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// 路径 → file:// URI(对齐 `file_uri.path_to_file_uri`)。
pub fn path_to_file_uri(path: &str) -> String {
    if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    }
}

/// LSP server 定义(注册表条目,对齐 `servers/types.ServerDefinition`)。
#[derive(Debug, Clone)]
pub struct ServerDefinition {
    pub id: String,
    pub find_root_include: Vec<String>,
    pub find_root_exclude: Vec<String>,
    /// 构建 spawn 参数:返回 None 表示该语言 server 不可用(如二进制缺失)。
    pub spawn: fn(&ServerDefinition, &str) -> Option<SpawnHandle>,
}

/// LSP Seam(Service Definition):manager 生命周期 + 诊断。
pub trait LspService: Seam {
    /// 初始化(幂等、懒加载)。
    fn initialize(&self, options: &InitializeOptions) -> Result<(), LspError>;

    /// 关闭全部 server。
    fn shutdown(&self) -> Result<(), LspError>;

    /// 取 server 状态快照。
    fn status(&self) -> Vec<LspServerStatus>;

    /// 取待交付诊断(默认上限 10/30)。
    fn pending_diagnostics(&self, max_per_file: usize, max_total: usize) -> Vec<LspDiagnosticFile>;
}

/// 内建 5 语言 server id(对齐 servers/servers/*.py 注册顺序
/// rust→typescript→java→python→go)。
pub const SERVER_RUST: &str = "rust-analyzer";
pub const SERVER_TYPESCRIPT: &str = "typescript";
pub const SERVER_JAVA: &str = "jdtls";
pub const SERVER_PYTHON: &str = "pyright";
pub const SERVER_GO: &str = "gopls";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_state_strings() {
        assert_eq!(LspServerState::Stopped.as_str(), "stopped");
        assert_eq!(LspServerState::Starting.as_str(), "starting");
        assert_eq!(LspServerState::Running.as_str(), "running");
        assert_eq!(LspServerState::Stopping.as_str(), "stopping");
        assert_eq!(LspServerState::Error.as_str(), "error");
    }

    #[test]
    fn parse_raw_diagnostic_normalizes() {
        // 完整条目。
        let raw = serde_json::json!({
            "message": "undefined var",
            "severity": 1,
            "range": {"start": {"line": 3, "character": 5}},
            "source": "pyright",
            "code": "undef-var"
        });
        let item = parse_raw_diagnostic(&raw).expect("parse");
        assert_eq!(item.severity, 1);
        assert_eq!(item.source.as_deref(), Some("pyright"));
        assert_eq!(diag_key(&item), "undefined var|1|3:5|undef-var");

        // 缺失 severity/range → 默认。
        let minimal = serde_json::json!({"message": "x"});
        let item = parse_raw_diagnostic(&minimal).expect("parse");
        assert_eq!(item.severity, 3);
        assert_eq!(diag_key(&item), "x|3|0:0|");

        // 空消息 / 非 dict → None。
        assert!(parse_raw_diagnostic(&serde_json::json!({"message": ""})).is_none());
        assert!(parse_raw_diagnostic(&serde_json::json!([1, 2])).is_none());
    }

    #[test]
    fn registry_merges_dedupes_and_orders() {
        let mut reg = LspDiagnosticRegistry::new();
        // 两批同 uri:Error(1) 与 Info(3),重复条目去重。
        reg.register(
            "server-a",
            "file:///a.py",
            &[
                serde_json::json!({"message": "err", "severity": 1, "range": {"start": {"line": 0, "character": 0}}}),
                serde_json::json!({"message": "info", "severity": 3, "range": {"start": {"line": 1, "character": 0}}}),
            ],
        );
        reg.register(
            "server-a",
            "file:///a.py",
            &[serde_json::json!({"message": "warn", "severity": 2, "range": {"start": {"line": 0, "character": 1}}})],
        );
        // 第二轮同 key 的 info 被跨轮去重。
        reg.register(
            "server-a",
            "file:///a.py",
            &[serde_json::json!({"message": "info", "severity": 3, "range": {"start": {"line": 1, "character": 0}}})],
        );
        let files = reg.get_and_clear(10, 30);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].server_name, "server-a");
        assert_eq!(files[0].local_path, "/a.py");
        let msgs: Vec<&str> = files[0]
            .diagnostics
            .iter()
            .map(|d| d.message.as_str())
            .collect();
        // severity 升序:err(1) → warn(2) → info(3);info 跨轮去重只留一个。
        assert_eq!(msgs, vec!["err", "warn", "info"]);
        // 第二轮同键 → 空。
        let second = reg.get_and_clear(10, 30);
        assert!(second.is_empty());
    }

    #[test]
    fn registry_respects_caps() {
        let mut reg = LspDiagnosticRegistry::new();
        let mut raws = Vec::new();
        for i in 0..15 {
            raws.push(serde_json::json!({
                "message": format!("m{i}"),
                "severity": (i % 4) + 1,
                "range": {"start": {"line": i, "character": 0}}
            }));
        }
        reg.register("s", "file:///big.py", &raws);
        let files = reg.get_and_clear(10, 30);
        // 每文件上限 10。
        assert_eq!(files[0].diagnostics.len(), 10);
        let files2 = reg.get_and_clear(10, 5);
        assert!(files2.is_empty() || files2[0].diagnostics.len() <= 5);
        reg.clear_all();
        // clear_all 后同键可再次交付。
        let files3 = reg.get_and_clear(10, 30);
        assert!(files3.is_empty());
    }

    #[test]
    fn file_uri_round_trip() {
        assert_eq!(
            file_uri_to_path("file:///a/b.py").as_deref(),
            Some("/a/b.py")
        );
        assert_eq!(file_uri_to_path("http://x").as_deref(), Some("http://x"));
        // 百分号解码。
        assert_eq!(
            file_uri_to_path("file:///d%3A/foo").as_deref(),
            Some("/d:/foo")
        );
        assert_eq!(path_to_file_uri("/a/b.py"), "file:///a/b.py");
    }

    #[test]
    fn spawn_handle_defaults() {
        let handle = SpawnHandle::new("gopls");
        assert_eq!(handle.startup_timeout, DEFAULT_STARTUP_TIMEOUT_MS);
        assert!(handle.args.is_empty());
    }
}
