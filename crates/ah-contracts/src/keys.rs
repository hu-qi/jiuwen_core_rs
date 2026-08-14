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
/// `teams` seam 服务键(多 agent 任务协作)。
pub const TEAMS: ServiceKey = ServiceKey::new("teams");

/// `evolving` seam 服务键(轨迹/评估/优化)。
pub const EVOLVING: ServiceKey = ServiceKey::new("evolving");

/// `rsi` seam 服务键(递归自改进管线)。
pub const RSI: ServiceKey = ServiceKey::new("rsi");

/// `context` seam 服务键(上下文组装与压缩)。
pub const CONTEXT: ServiceKey = ServiceKey::new("context");

/// `store/kv` seam 服务键(通用键值存储)。
pub const KV_STORE: ServiceKey = ServiceKey::new("store-kv");

/// `store/messages` seam 服务键(append-only 消息存储)。
pub const MESSAGE_STORE: ServiceKey = ServiceKey::new("store-messages");

/// `prompt` seam 服务键(模板渲染与版本化注册表)。
pub const PROMPT: ServiceKey = ServiceKey::new("prompt");

/// `queue` seam 服务键(消息队列)。
pub const QUEUE: ServiceKey = ServiceKey::new("queue");

/// `workspace` seam 服务键(工作区清单与目标)。
pub const WORKSPACE: ServiceKey = ServiceKey::new("workspace");

/// `sandbox` seam 服务键(策略化沙箱)。
pub const SANDBOX: ServiceKey = ServiceKey::new("sandbox");

/// `code` seam 服务键(代码执行)。
pub const CODE: ServiceKey = ServiceKey::new("code");

/// `web` seam 服务键(HTTP 客户端)。
pub const WEB: ServiceKey = ServiceKey::new("web");

/// `transport` seam 服务键(agent 传输)。
pub const TRANSPORT: ServiceKey = ServiceKey::new("transport");

/// `teams-swarm` seam 服务键(swarmflow 编排)。
pub const SWARM: ServiceKey = ServiceKey::new("teams-swarm");

/// `git` seam 服务键(本地 git 操作)。
pub const GIT: ServiceKey = ServiceKey::new("git");

/// `ci` seam 服务键(CI gate 运行器)。
pub const CI: ServiceKey = ServiceKey::new("ci");

/// `rsi-analyzer` seam 服务键(评测结果分析)。
pub const RSI_ANALYZER: ServiceKey = ServiceKey::new("rsi-analyzer");
