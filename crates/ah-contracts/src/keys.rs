//! 稳定服务键:seam 的公开契约。
//!
//! 插件与消费方共享这些键;新增 seam 时在此登记。

use crate::service::ServiceKey;

/// `llm` seam 服务键。
pub const LLM: ServiceKey = ServiceKey::new("llm");

/// `tools` seam 服务键。
pub const TOOLS: ServiceKey = ServiceKey::new("tools");

/// `fs` seam 服务键。
pub const FS: ServiceKey = ServiceKey::new("fs");

/// `shell` seam 服务键。
pub const SHELL: ServiceKey = ServiceKey::new("shell");

/// `agent-loop` 服务键(agent 循环)。
pub const AGENT_LOOP: ServiceKey = ServiceKey::new("agent-loop");

/// `sessions` seam 服务键(会话事件日志)。
pub const SESSIONS: ServiceKey = ServiceKey::new("sessions");

/// `session-manager` seam 服务键(多会话管理)。
pub const SESSION_MANAGER: ServiceKey = ServiceKey::new("session-manager");

/// `workflow` seam 服务键(工作流引擎)。
pub const WORKFLOW: ServiceKey = ServiceKey::new("workflow");

/// `memory` seam 服务键(持久化记忆)。
pub const MEMORY: ServiceKey = ServiceKey::new("memory");

/// `retrieval` seam 服务键(知识库检索)。
pub const RETRIEVAL: ServiceKey = ServiceKey::new("retrieval");
