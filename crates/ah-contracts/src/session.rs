//! session seam:append-only 会话事件日志(日志即真相)。

use serde_json::Value;

use crate::llm::ChatMessage;
use crate::seam::Seam;

/// 会话事件类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventKind {
    /// 用户消息。
    User,
    /// 助手消息(可携带 tool_calls)。
    Assistant,
    /// 工具执行结果。
    ToolResult,
    /// 系统消息。
    System,
    /// agent 每轮步进。
    AgentStep,
}

/// 一条会话事件(append-only 日志的最小单元)。
///
/// payload 约定(投影 derive_messages 依赖):
/// - User / Assistant(无 tool_calls):{"content": string}
/// - Assistant 工具调用:{"tool_calls": [{id, name, arguments}]}
/// - ToolResult:{"tool_call_id": string, "output": string}
/// - AgentStep:{"iteration": int, "tool_calls": int, "done": bool}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SessionEvent {
    /// 单调递增序号(append-only)。
    pub seq: u64,
    pub timestamp_ms: u64,
    pub kind: SessionEventKind,
    pub payload: Value,
}

impl crate::event::Event for SessionEvent {
    const ID: &'static str = "session/event";
}

/// 会话日志错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionError(pub String);

impl core::fmt::Display for SessionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SessionError {}

/// 会话日志 Seam(Service Definition)。
///
/// 对齐 DSH 的 session log 原则:**模型可见即已记录**——
/// 任何到达模型请求的输入都必须能从本日志重建。
pub trait SessionLog: Seam {
    /// 追加一条事件并落盘,返回带序号的完整事件。
    fn append(&self, kind: SessionEventKind, payload: Value) -> Result<SessionEvent, SessionError>;

    /// 全部事件(按 seq 升序)。
    fn events(&self) -> Vec<SessionEvent>;

    /// seq 之后的增量事件。
    fn since(&self, seq: u64) -> Vec<SessionEvent>;

    /// 投影:从日志重建模型可见消息序列(日志即真相)。
    fn derive_messages(&self) -> Vec<ChatMessage>;
}
/// 会话管理器 Seam:多会话的创建/打开/fork/检查点/恢复/列举。
///
/// 每个会话对应一份独立的 append-only 日志文件;fork 复制历史到新会话;
/// checkpoint 把会话快照到 checkpoints/{name}.jsonl,restore 从快照回滚。
pub trait SessionManager: Seam {
    /// 创建(或打开已存在)会话,返回会话日志句柄。
    fn create(&self, id: &str) -> Result<std::sync::Arc<dyn SessionLog>, SessionError>;

    /// 打开已存在的会话;不存在则报错。
    fn open(&self, id: &str) -> Result<std::sync::Arc<dyn SessionLog>, SessionError>;

    /// 复制会话 from 的历史到新会话 to,返回新会话句柄。
    fn fork(&self, from: &str, to: &str) -> Result<std::sync::Arc<dyn SessionLog>, SessionError>;

    /// 保存命名检查点:把会话当前日志快照到 checkpoints/{name}.jsonl。
    fn checkpoint(&self, id: &str, name: &str) -> Result<(), SessionError>;

    /// 从命名检查点恢复会话(以快照内容替换会话日志),返回新日志句柄;
    /// 恢复后 append 从快照的 seq 继续。
    fn restore(&self, id: &str, name: &str)
    -> Result<std::sync::Arc<dyn SessionLog>, SessionError>;

    /// 列举全部会话 id。
    fn list(&self) -> Vec<String>;
}
