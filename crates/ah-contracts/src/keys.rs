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

/// `security` seam 服务键(安全检测)。
pub const SECURITY: ServiceKey = ServiceKey::new("security");

/// `subagent` seam 服务键(子任务委派)。
pub const SUBAGENT: ServiceKey = ServiceKey::new("subagent");

/// `mcp` seam 服务键(Model Context Protocol 客户端)。
pub const MCP: ServiceKey = ServiceKey::new("mcp");

/// `telemetry` seam 服务键(span 记录与导出)。
pub const TELEMETRY: ServiceKey = ServiceKey::new("telemetry");

/// `credentials` seam 服务键(凭据引用)。
pub const CREDENTIALS: ServiceKey = ServiceKey::new("credentials");
