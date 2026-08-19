//! team_context seam:团队 session 上下文(session_id 隔离)。
//!
//! 对齐 `openjiuwen/agent_teams/context.py`:
//! - `set_session_id` / `get_session_id` / `reset_session_id`(contextvar 语义,
//!   session_id 同时作为日志 trace id);
//! - `LOG_DEFAULT_TRACE_ID = "default_trace_id"` 占位。
//!
//! Rust 侧以每会话上下文句柄建模(无全局 contextvar):宿主把当前
//! [`TeamSessionContext`] 传入团队运行时,消息/主题隔离经 `session_id`
//! 判定。契约零实现:存储由插件提供。

use crate::seam::Seam;

/// 日志默认 trace id 占位(对齐 `_LOG_DEFAULT_TRACE_ID`)。
pub const LOG_DEFAULT_TRACE_ID: &str = "default_trace_id";

/// team_context 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamContextError(pub String);

impl core::fmt::Display for TeamContextError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TeamContextError {}

/// 会话上下文 Seam(Service Definition):session_id 隔离原语。
///
/// 对齐 `set_session_id` / `get_session_id` / `reset_session_id` 的可观察语义:
/// 设置后本上下文内 `get_session_id` 返回该值;reset 恢复之前值(无则空)。
pub trait TeamSessionContext: Seam {
    /// 设置当前 session_id,返回可逆 token。
    fn set_session_id(&self, session_id: &str) -> Result<SessionToken, TeamContextError>;

    /// 取当前 session_id(未设置 → 空串,对齐 `get_session_id() or ""`)。
    fn get_session_id(&self) -> String;

    /// 重置到 token 之前的 session_id(无前值 → 空串)。
    fn reset_session_id(&self, token: SessionToken) -> Result<(), TeamContextError>;
}

/// session_id 设置的可逆 token(对齐 contextvars.Token)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionToken {
    /// 该 token 捕获的前一个 session_id(None = 之前未设置)。
    pub previous: Option<String>,
}

impl SessionToken {
    /// 构造 token。
    pub fn new(previous: Option<String>) -> Self {
        Self { previous }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_default_trace_id_placeholder() {
        assert_eq!(LOG_DEFAULT_TRACE_ID, "default_trace_id");
    }

    #[test]
    fn token_carries_previous_value() {
        let token = SessionToken::new(Some("prev".to_string()));
        assert_eq!(token.previous.as_deref(), Some("prev"));
        let empty = SessionToken::new(None);
        assert_eq!(empty.previous, None);
    }
}
