//! # ah-contracts
//!
//! 契约层:只声明接口(Seam trait)与纯类型,零实现。
//!
//! 设计规则(对齐 DSH/Cordis):
//! - 本 crate 不允许出现 Mock / Unsupported / fallback 实现;
//!   生产路径的实现必须来自插件 crate;
//! - 每个 Seam 由三角构成:Service Definition(接口)、Service Provider(实现)、
//!   Consumer(消费方,通常是模型可见工具);
//! - 插件之间不允许直接依赖彼此的具体类型,只允许依赖本 crate 的契约;
//! - 机制类型([`Effect`]、[`ServiceKey`])也定义在此层,供 seam 接口使用。

pub mod agent;
pub mod agent_builder;
pub mod analyzer;
pub mod autoharness;
pub mod ci;
pub mod cli;
pub mod code;
pub mod context;
pub mod controller;
pub mod credentials;
pub mod effect;
pub mod event;
pub mod evolving;
pub mod external;
pub mod fs;
pub mod git;
pub mod graph_memory;
pub mod keys;
pub mod llm;
pub mod mcp;
pub mod memory;
pub mod oauth;
pub mod operator;
pub mod optimizer;
pub mod pregel;
pub mod prompt;
pub mod queue;
pub mod retrieval;
pub mod reward;
pub mod rsi;
pub mod runner;
pub mod sandbox;
pub mod seam;
pub mod security;
pub mod service;
pub mod session;
pub mod shell;
pub mod single_harness;
pub mod skill;
pub mod store;
pub mod subagent;
pub mod subagents;
pub mod swarm;
pub mod symphony;
pub mod teams;
pub mod telemetry;
pub mod tool_approval;
pub mod tools;
pub mod trainer;
pub mod transport;
pub mod tune;
pub mod web;
pub mod workflow;
pub mod workspace;

pub use effect::Effect;

pub mod prelude {
    pub use crate::agent::AgentStep;
    pub use crate::agent_builder::{AgentBuilder, AgentDesign, BuildError};
    pub use crate::analyzer::{
        AnalysisArtifact, AnalysisSignal, AnalyzerCase, AnalyzerError, EvaluationAnalyzer,
        EvidenceRef, SignalKind, TeamIssue,
    };
    pub use crate::autoharness::{
        AutoHarness, AutoHarnessConfig, AutoHarnessError, CycleResult, StageKind, StageResult,
    };
    pub use crate::ci::{CiError, CiGateRequest, CiGateResult, CiGateRunner};
    pub use crate::cli::{CliChunk, CliError, CliRenderer, TodoItem, TodoStatus};
    pub use crate::code::{CodeError, CodeExecRequest, CodeExecResult, CodeProvider};
    pub use crate::context::{
        AssembledContext, ContextEngine, ContextError, ContextSummary, SummarySource,
    };
    pub use crate::controller::{
        Controller, ControllerError, Intent, IntentType, Task, TaskExecutor, TaskFilter, TaskStatus,
    };
    pub use crate::credentials::{Credential, CredentialError, CredentialProvider};
    pub use crate::effect::Effect;
    pub use crate::event::Event;
    pub use crate::evolving::{
        Evaluation, EvolvingError, EvolvingRuntime, Refinement, RefinementTarget, StepOutcome,
        Trajectory, TrajectoryStep, Verdict,
    };
    pub use crate::external::{
        CliAgentAdapter, CompletionStrategy, ExternalCliError, ExternalCliRuntime, ExternalCliTurn,
        InputFormat, TeamJoinDescriptor, codex_narration, codex_proto_line,
    };
    pub use crate::fs::{FsError, FsProvider};
    pub use crate::git::{GitCommit, GitError, GitProvider, GitStatusEntry};
    pub use crate::graph_memory::{
        AddMemoryResult, Entity, Episode, GraphHit, GraphMemory, GraphMemoryError, Relation,
    };
    pub use crate::keys::{
        AGENT_LOOP, CREDENTIALS, FS, LLM, MCP, MEMORY, RETRIEVAL, SESSION_MANAGER, SESSIONS, SHELL,
        TELEMETRY, TOOLS, WORKFLOW,
    };
    pub use crate::llm::{
        ChatMessage, ChatRole, ModelChunk, ModelError, ModelProvider, ModelRequest, ModelResponse,
        ToolCall, ToolCallDelta, ToolSchema,
    };
    pub use crate::mcp::{McpClient, McpContent, McpError, McpInfo, McpTool, McpToolResult};
    pub use crate::memory::{MemoryError, MemoryProvider, MemoryRecord};
    pub use crate::oauth::{
        DeviceAuthRequest, DeviceCode, OAuthClient, OAuthError, PollResult, TokenResponse,
    };
    pub use crate::operator::{
        Operator, OperatorError, OperatorRegistry, ParameterUpdated, TunableKind, TunableSpec,
    };
    pub use crate::optimizer::{Optimizer, OptimizerError, TextualGradient, UpdateResult};
    pub use crate::pregel::{
        PregelEdge, PregelEngine, PregelError, PregelGraph, PregelNodeKind, PregelNodeSpec,
        PregelResult,
    };
    pub use crate::prompt::{PromptError, PromptRegistry, PromptTemplate, RenderedPrompt};
    pub use crate::queue::{MessageQueue, QueueError, QueueMessage};
    pub use crate::retrieval::{RetrievalError, RetrievalHit, RetrievalProvider};
    pub use crate::reward::{RewardCase, RewardConfig, RewardError, RewardFunction, RewardOutput};
    pub use crate::rsi::{RsiCase, RsiCheckpoint, RsiError, RsiReport, RsiRunOutcome, RsiRuntime};
    pub use crate::runner::{
        CallbackChain, CallbackMetrics, ChainAction, ChainCallback, ChainContext, ChainResult,
        RunnerError,
    };
    pub use crate::sandbox::{
        CommandDecision, FsDecision, SandboxError, SandboxPolicy, SandboxProvider,
    };
    pub use crate::seam::Seam;
    pub use crate::security::{
        Guardrail, GuardrailDecision, SecurityError, SecurityProvider, SecurityVerdict, Severity,
    };
    pub use crate::service::ServiceKey;
    pub use crate::session::{SessionError, SessionEvent, SessionEventKind, SessionLog};
    pub use crate::shell::{ShellError, ShellOutput, ShellProvider};
    pub use crate::single_harness::{
        EpochOutcome, SingleHarnessCheckpoint, SingleHarnessError, SingleHarnessRequest,
        SingleHarnessResult, SingleHarnessRuntime,
    };
    pub use crate::skill::{Skill, SkillError, SkillEvaluation, SkillRegistry};
    pub use crate::store::{
        BaseKVStore, BaseMessageStore, KvEntry, StoreError, StoreProvider, StoredMessage,
    };
    pub use crate::subagent::{SubagentError, SubagentResult, SubagentRuntime, SubagentSpec};
    pub use crate::subagents::{SubagentKind, SubagentProfile, TypedSubagentError, TypedSubagents};
    pub use crate::swarm::{
        SwarmAgentActivity, SwarmAgentRecord, SwarmAgentStatus, SwarmError, SwarmEvent,
        SwarmEventKind, SwarmFlowScript, SwarmPhase, SwarmPhaseRecord, SwarmRun, SwarmRunStatus,
        SwarmflowRunner,
    };
    pub use crate::symphony::{
        Capability, CapabilityFingerprint, ExecutionStep, OrchestrationPlan, Symphony,
        SymphonyError,
    };
    pub use crate::teams::{
        TeamError, TeamMemberSpec, TeamMessage, TeamRunResult, TeamRuntime, TeamSpec, TeamTask,
        TeamTaskEvent, TeamTaskStatus,
    };
    pub use crate::telemetry::{Span, TelemetryError, TelemetryProvider};
    pub use crate::tool_approval::{ToolApproval, ToolApprovalError};
    pub use crate::tools::{Tool, ToolError, ToolRegistry};
    pub use crate::trainer::{TrainEpoch, TrainRequest, TrainResult, Trainer, TrainerError};
    pub use crate::transport::{
        AgentCard, AgentHandler, AgentMessage, AgentTransport, TransportError,
    };
    pub use crate::tune::{TuneError, TunePipeline, TuneRequest, TuneResult, TuneRoundResult};
    pub use crate::web::{WebError, WebFetchRequest, WebFetchResult, WebProvider};
    pub use crate::workflow::{
        EdgeSpec, NodeKind, NodeSpec, WorkflowEngine, WorkflowError, WorkflowOutput, WorkflowSpec,
    };
    pub use crate::workspace::{
        Goal, GoalStatus, WorkspaceError, WorkspaceManifest, WorkspaceService,
    };
}
