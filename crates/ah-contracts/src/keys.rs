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

/// `auto-harness` seam 服务键(自动化改进周期)。
pub const AUTO_HARNESS: ServiceKey = ServiceKey::new("auto-harness");

/// `reward` seam 服务键(RL 奖励计算)。
pub const REWARD: ServiceKey = ServiceKey::new("reward");

/// `subagents` seam 服务键(类型化子代理)。
pub const SUBAGENTS: ServiceKey = ServiceKey::new("subagents");

/// `skill` seam 服务键(技能注册与评估)。
pub const SKILL: ServiceKey = ServiceKey::new("skill");

/// `pregel` seam 服务键(超级步图执行)。
pub const PREGEL: ServiceKey = ServiceKey::new("pregel");

/// `tune` seam 服务键(训练流水线)。
pub const TUNE: ServiceKey = ServiceKey::new("tune");

/// `tool-approval` seam 服务键(渐进工具披露)。
pub const TOOL_APPROVAL: ServiceKey = ServiceKey::new("tool-approval");

/// `oauth` seam 服务键(设备码授权)。
pub const OAUTH: ServiceKey = ServiceKey::new("oauth");

/// `agent-builder` seam 服务键(NL → 设计 → DSL → 执行)。
pub const AGENT_BUILDER: ServiceKey = ServiceKey::new("agent-builder");

/// `symphony` seam 服务键(能力编排)。
pub const SYMPHONY: ServiceKey = ServiceKey::new("symphony");
pub const CLI_RENDERER: ServiceKey = ServiceKey::new("cli-renderer");
pub const EXTERNAL_CLI: ServiceKey = ServiceKey::new("external-cli");
pub const CONTROLLER: ServiceKey = ServiceKey::new("controller");
pub const OPERATOR: ServiceKey = ServiceKey::new("operator");
pub const RUNNER: ServiceKey = ServiceKey::new("runner");
pub const GRAPH_MEMORY: ServiceKey = ServiceKey::new("graph-memory");
pub const SINGLE_HARNESS: ServiceKey = ServiceKey::new("single-harness");
pub const OPTIMIZER: ServiceKey = ServiceKey::new("optimizer");
pub const TRAINER: ServiceKey = ServiceKey::new("trainer");
pub const TEAM_SKILL: ServiceKey = ServiceKey::new("team-skill");
pub const MEMBER_OPTIMIZER: ServiceKey = ServiceKey::new("member-optimizer");
pub const TEAM_MONITOR: ServiceKey = ServiceKey::new("team-monitor");
pub const MEMORY_EVOLVER: ServiceKey = ServiceKey::new("memory-evolver");
pub const TOKENIZER: ServiceKey = ServiceKey::new("tokenizer");
pub const RL_STEP: ServiceKey = ServiceKey::new("rl-step");
pub const RERANK: ServiceKey = ServiceKey::new("rerank");
pub const SHARING: ServiceKey = ServiceKey::new("sharing");
pub const EXPERIENCE_SCORER: ServiceKey = ServiceKey::new("experience-scorer");
pub const JSON_PARSER: ServiceKey = ServiceKey::new("json-parser");
pub const MODEL_CATALOG: ServiceKey = ServiceKey::new("model-catalog");
pub const SIGNALS: ServiceKey = ServiceKey::new("signals");
pub const DATASET_CURATOR: ServiceKey = ServiceKey::new("dataset-curator");
pub const TEAM_DISPATCH: ServiceKey = ServiceKey::new("team-dispatch");
pub const TEAM_POOL: ServiceKey = ServiceKey::new("team-pool");
pub const TEAM_STATUS: ServiceKey = ServiceKey::new("team-status");
pub const TEAM_VERDICT: ServiceKey = ServiceKey::new("team-verdict");
pub const TEAM_MESSAGE: ServiceKey = ServiceKey::new("team-message");
pub const INBOUND_RENDER: ServiceKey = ServiceKey::new("inbound-render");
pub const TIMEFMT: ServiceKey = ServiceKey::new("timefmt");
pub const TEAM_SCHEDULER: ServiceKey = ServiceKey::new("team-scheduler");
pub const ROSTER_DIFF: ServiceKey = ServiceKey::new("roster-diff");
pub const TEAM_I18N: ServiceKey = ServiceKey::new("team-i18n");
pub const TEAM_CONTEXT_TEXT: ServiceKey = ServiceKey::new("team-context-text");
pub const EXTERNAL_FORMAT: ServiceKey = ServiceKey::new("external-format");
pub const INTERACTION_ROUTER: ServiceKey = ServiceKey::new("interaction-router");
pub const SCHEDULER_RENDER: ServiceKey = ServiceKey::new("scheduler-render");
pub const MODEL_ALLOCATOR: ServiceKey = ServiceKey::new("model-allocator");
pub const TEAM_JOIN_DESCRIPTOR: ServiceKey = ServiceKey::new("team-join-descriptor");
pub const TEAM_TASK_STATUS: ServiceKey = ServiceKey::new("team-task-status");
pub const PROMPT_ATTACHMENT: ServiceKey = ServiceKey::new("prompt-attachment");
