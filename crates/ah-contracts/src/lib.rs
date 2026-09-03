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

pub mod a2a;
pub mod ability;
pub mod agent;
pub mod agent_builder;
pub mod analyzer;
pub mod autoharness;
pub mod bridge_compose;
pub mod checkpointer;
pub mod ci;
pub mod cli;
pub mod code;
pub mod context;
pub mod context_evolver;
pub mod controller;
pub mod credentials;
pub mod dataset_curator;
pub mod effect;
pub mod event;
pub mod evolving;
pub mod external;
pub mod external_client;
pub mod external_format;
pub mod fs;
pub mod git;
pub mod graph_memory;
pub mod harness_schema;
pub mod inbound_render;
pub mod interaction_router;
pub mod json_parser;
pub mod keys;
pub mod kv_cache;
pub mod llm;
pub mod lsp;
pub mod manifest;
pub mod mcp;
pub mod member_optimizer;
pub mod memory;
pub mod memory_lite;
pub mod messager;
pub mod model_allocator;
pub mod model_backup;
pub mod model_catalog;
pub mod oauth;
pub mod operator;
pub mod optimizer;
pub mod pregel;
pub mod prompt;
pub mod prompt_attachment;
pub mod prompt_builder;
pub mod prompt_builder_devtools;
pub mod queue;
pub mod reliability_config;
pub mod reliability_detectors;
pub mod reliability_rail;
pub mod rerank;
pub mod resources;
pub mod retrieval;
pub mod reward;
pub mod rl_step;
pub mod roster_diff;
pub mod rsi;
pub mod rsi_config;
pub mod rsi_evaluator;
pub mod rsi_learner;
pub mod runner;
pub mod sandbox;
pub mod scheduler;
pub mod scheduler_render;
pub mod scoring;
pub mod seam;
pub mod security;
pub mod service;
pub mod session;
pub mod sharing;
pub mod shell;
pub mod signals;
pub mod single_harness;
pub mod skill;
pub mod skill_creator;
pub mod store;
pub mod stream;
pub mod subagent;
pub mod subagents;
pub mod swarm;
pub mod symphony;
pub mod tag_manager;
pub mod team_context;
pub mod team_context_text;
pub mod team_dispatch;
pub mod team_i18n;
pub mod team_join_descriptor;
pub mod team_message;
pub mod team_monitor;
pub mod team_pool;
pub mod team_prompts;
pub mod team_schema;
pub mod team_skill;
pub mod team_skill_generator;
pub mod team_status;
pub mod team_task_status;
pub mod team_verdict;
pub mod teams;
pub mod telemetry;
pub mod timefmt;
pub mod tokenizer;
pub mod tool_approval;
pub mod tools;
pub mod tracer_otel;
pub mod trainer;
pub mod transport;
pub mod tune;
pub mod tune_kit;
pub mod web;
pub mod workflow;
pub mod workspace;
pub mod worktree;

pub use effect::Effect;

pub mod prelude {
    pub use crate::a2a::{
        A2aError, A2aInterface, A2aPartView, AgentResultView, ArtifactView,
        a2a_artifact_to_artifact, a2a_message_to_result, a2a_status_to_ojw, a2a_task_to_result,
        build_agent_result, build_description, build_interfaces, from_struct, merge_agent_results,
        merge_metadata, message_to_payload, normalize_jsonrpc_interface_url,
        normalize_jsonrpc_route_path, resolve_session_id, resolve_transport_protocols,
        serialize_param_payload, to_a2a_agent_card, to_a2a_part, to_a2a_request, to_struct,
        with_session_id,
    };
    pub use crate::ability::{Ability, AbilityError, AbilityExecution, AbilityManager};
    pub use crate::agent::{
        AgentCallbackContext, AgentCallbackManager, AgentCard, AgentControl, AgentControlError,
        AgentRequest, AgentResult, AgentRunState, AgentStep, ApplicationRuntime, InterruptRuntime,
    };
    pub use crate::agent_builder::{AgentBuilder, AgentDesign, BuildError};
    pub use crate::analyzer::{
        AnalysisArtifact, AnalysisSignal, AnalyzerCase, AnalyzerError, CaseAnalysisInput,
        DeterministicSignals, EvaluationAnalyzer, EvaluationSummaryInput, EvidenceRef, SignalKind,
        TeamIssue, extract_generic_signals, fingerprint_error,
    };
    pub use crate::autoharness::{
        AutoHarness, AutoHarnessConfig, AutoHarnessError, CycleResult, StageKind, StageResult,
    };
    pub use crate::bridge_compose::{
        BridgeCompose, BridgeMailboxInjectMode, REMOTE_UNAVAILABLE_SENTINEL, TeamRole,
        compose_bridge_inbound, wrap_outbound_to_remote,
    };
    pub use crate::checkpointer::{
        Checkpointer, CheckpointerError, CheckpointerProvider, INTERACTIVE_INPUT,
        RedisCheckpointerConfig, RedisConnectionConfig, RedisPipeline, RedisStore, RedisTTLConfig,
        RedisValue, SESSION_NAMESPACE_AGENT, SESSION_NAMESPACE_AGENT_TEAM,
        SESSION_NAMESPACE_WORKFLOW, TASK_STATUS_INTERRUPT, WORKFLOW_NAMESPACE_GRAPH, build_key,
        build_key_with_namespace, ttl_seconds_from_minutes,
    };
    pub use crate::ci::{CiError, CiGateRequest, CiGateResult, CiGateRunner};
    pub use crate::cli::{CliChunk, CliError, CliRenderer, TodoItem, TodoStatus};
    pub use crate::code::{CodeError, CodeExecRequest, CodeExecResult, CodeProvider};
    pub use crate::context::{
        AssembledContext, ContextEngine, ContextError, ContextSummary, SummarySource,
    };
    pub use crate::context_evolver::{
        MemoryEvolver, MemoryEvolverError, MemoryInjection, TaskMemory, TrajectorySummary,
    };
    pub use crate::controller::{
        Controller, ControllerError, Intent, IntentType, Task, TaskExecutor, TaskFilter,
        TaskSnapshotStore, TaskStatus,
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
    pub use crate::external_client::{
        BROADCAST_TARGET, ExternalClientError, ExternalInboxSource, ExternalTeamClient,
        InboxObserver, InboxView, compose_inbox_text,
    };
    pub use crate::fs::{FsError, FsProvider};
    pub use crate::git::{GitCommit, GitError, GitProvider, GitStatusEntry};
    pub use crate::graph_memory::{
        AddMemoryResult, Entity, Episode, GraphHit, GraphMemory, GraphMemoryError, Relation,
    };
    pub use crate::keys::{
        AGENT_CALLBACKS, AGENT_LOOP, APPLICATION, CREDENTIALS, FS, INTERRUPT, LLM, MCP, MEMORY,
        RETRIEVAL, SESSION_MANAGER, SESSIONS, SHELL, TELEMETRY, TOOLS, WORKFLOW,
    };
    pub use crate::kv_cache::{
        ControlDomain, KvcAffinityModel, KvcCacheIdentity, KvcError, KvcHooks, KvcTeamAction,
        SessionKvcAction, SessionKvcSignal, TeamKvcState, build_control_domain,
        is_binding_manageable, is_sticky_subagent_type, normalized_config_value, record_actionable,
        resolve_kvc_action_timeout, resolve_sub_session_id, run_session_kv_action,
        state_after_action, validate_signal_action,
    };
    pub use crate::llm::{
        ChatMessage, ChatRole, ModelChunk, ModelError, ModelProvider, ModelRequest, ModelResponse,
        ToolCall, ToolCallDelta, ToolSchema,
    };
    pub use crate::lsp::{
        DEFAULT_STARTUP_TIMEOUT_MS, InitializeOptions, LspDiagnosticFile, LspDiagnosticItem,
        LspDiagnosticRegistry, LspError, LspServerState, LspServerStatus, LspService,
        MAX_CRASH_RECOVERY_ATTEMPTS, MAX_DIAG_PER_FILE, MAX_DIAG_TOTAL, SERVER_GO, SERVER_JAVA,
        SERVER_PYTHON, SERVER_RUST, SERVER_TYPESCRIPT, ScopedLspServerConfig, ServerDefinition,
        SpawnHandle, diag_key, file_uri_to_path, parse_raw_diagnostic, path_to_file_uri,
    };
    pub use crate::manifest::{
        ConstructionInputModel, ElementFactory, ElementKind, HarnessElementDescriptor,
        InputFieldSpec, InputSource, InterfaceMethod, ManifestCatalog, ManifestContext,
        ManifestError, ManifestFactoryRegistry, ManifestRegistration, MapManifestContext,
        default_interface_methods, factory_ref,
    };
    pub use crate::mcp::{McpClient, McpContent, McpError, McpInfo, McpTool, McpToolResult};
    pub use crate::member_optimizer::{
        Attribution, Lever, MechanismType, MemberOptimizationResult, MemberOptimizer,
        MemberOptimizerError, OptimizationPlan, PublishResult, Verification,
    };
    pub use crate::memory::{MemoryError, MemoryProvider, MemoryRecord};
    pub use crate::model_backup::{
        ModelBackup, ModelBackupError, ModelBackupPolicy, ModelBackupPolicyProvider,
    };
    pub use crate::model_catalog::{ModelCatalogError, ModelProviderCatalog};
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
    pub use crate::reliability_rail::{
        DetectorSpec, LocalSink, PolicyView, ReliabilityFactory, ReliabilityHandler,
        ReliabilityRail, RouteDecision, after_model_call_signal, after_tool_call_signal,
        args_as_dict, before_model_call_signal, before_tool_call_signal, error_text,
        format_anomaly, format_anomaly_event, measure_response, member_detector_specs,
        model_exception_signal, route_decision, severity_value, tool_exception_signal,
    };
    pub use crate::rerank::{RerankConfig, RerankError, RerankedHit, Reranker};
    pub use crate::resources::{
        AgentTemplateSpec, BuiltinToolSpec, ExtensionParts, LoadRecord, McpServerSpec, PluginSpec,
        PromptSectionSpec, RailSpec, ResolvedPromptSection, ResolvedSkill, ResourceKind,
        ResourceRef, ResourcesError, ResourcesResolver, SkillSpec, looks_like_mcp_server_entry,
        mcp_transport_alias, normalize_mcp_server_entry, render_params, render_template,
        select_content, skill_enabled_list, skill_mode, validate_plain_data, validate_plugin_paths,
    };
    pub use crate::retrieval::{RetrievalError, RetrievalHit, RetrievalProvider};
    pub use crate::reward::{RewardCase, RewardConfig, RewardError, RewardFunction, RewardOutput};
    pub use crate::rl_step::{PpoParams, RlSample, RlStep, RlStepError, RlStepResult};
    pub use crate::rsi::{
        GeneratedDataset, RsiCase, RsiCheckpoint, RsiError, RsiReport, RsiRunOutcome, RsiRuntime,
    };
    pub use crate::rsi_evaluator::{
        MAX_SAVED_LLM_MESSAGES, MAX_SAVED_TEXT_CHARS, MAX_SAVED_TOOL_RESULT_CHARS,
        ROLE_TRAJECTORY_DIR_NAME, RsiTrajectoryTools, TRAJECTORY_EVENTS_FILE_NAME,
        add_skill_name_from_args, bound_llm_detail, bound_tool_detail, bounded_messages,
        bounded_trajectory_dict, canonical_tool_name, collect_pre_edit_successful_usage,
        collect_successful_skill_names, collect_successful_tool_names, is_persistent_edit_step,
        safe_role_file_stem, tool_summary, truncate_json_like, truncate_text,
    };
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
    pub use crate::skill_creator::{
        AssetEntry, FilterDecision, MAX_ASSETS, MAX_CONTENT_LENGTH, SUPPORTED_EXTS, SkillCreator,
        SkillCreatorError, SkillFetcher, SkillGenRequest, SkillGenerator, encode_b64, filter_block,
        image_ext, mime_to_ext, save_fetched_assets_manifest, slugify, strip_hallucinated_images,
        strip_json_fence, url_to_slug,
    };
    pub use crate::store::{
        BaseKVStore, BaseMessageStore, KvEntry, StoreError, StoreProvider, StoredMessage,
    };
    pub use crate::stream::{TaggedChunk, tag_chunk};
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
    pub use crate::team_context::{
        LOG_DEFAULT_TRACE_ID, SessionToken, TeamContextError, TeamSessionContext,
    };
    pub use crate::team_monitor::{
        MemberInfo, MonitorError, MonitorEvent, MonitorEventType, TaskInfo, TeamInfo, TeamMonitor,
        status_label,
    };
    pub use crate::team_prompts::{
        MemberSummary, PromptLoadError, TEAM_PLAN_MODE_PROMPT_CN, TEAM_PLAN_MODE_PROMPT_EN,
        TeamPromptLoader, TeamPrompts, build_bridge_brief, build_enter_plan_mode_status,
        build_plan_file_info, build_team_plan_mode_prompt, build_team_plan_mode_section,
        get_team_plan_mode_prompt, resolve_language,
    };
    pub use crate::team_schema::{
        DEFAULT_LEADER_MEMBER_NAME, GraphMutationResult, InfraRegistry, NewTaskSpec,
        RESERVED_MEMBER_NAMES, RegistryError, SchemaError, SshTransportConfig, TaskCreateResult,
        TaskDetail, TaskGraphResult, TaskGraphSpec, TaskListResult, TaskOpResult, TaskSummary,
        USER_PSEUDO_MEMBER_NAME, storage_merged_params, transport_merged_params,
        validate_bridge_consistency, validate_external_cli_unique, validate_hitt_consistency,
        validate_pool_router_exclusive, validate_reserved_names, validate_review_settings,
        validate_ssh_auth, validate_stall_settings, validate_swarmflow_budget,
    };
    pub use crate::team_skill::{
        GenerateTeamSkillRequest, GenerateTeamSkillResult, TeamSkillError, TeamSkillGenerator,
    };
    pub use crate::team_skill_generator::{
        SkillGenError, TeamSkillPlanNormalizer, normalize_roles, normalize_team_skill_plan,
        normalize_workflow_steps, plan_slugify, single_line, string_list, write_skill_md,
    };
    pub use crate::teams::{
        TeamError, TeamMemberSpec, TeamMessage, TeamRunResult, TeamRuntime, TeamSpec, TeamTask,
        TeamTaskEvent, TeamTaskStatus,
    };
    pub use crate::telemetry::{Span, TelemetryError, TelemetryProvider};
    pub use crate::tokenizer::{Token, Tokenizer, TokenizerError};
    pub use crate::tool_approval::{ToolApproval, ToolApprovalError};
    pub use crate::tools::{Tool, ToolError, ToolRegistry};
    pub use crate::tracer_otel::{
        ExporterPlan, GEN_AI_COMPLETION, GEN_AI_OPERATION_NAME, GEN_AI_PROMPT,
        GEN_AI_REQUEST_MODEL, GEN_AI_SYSTEM, GEN_AI_SYSTEM_VALUE, GEN_AI_TOOL_NAME,
        GEN_AI_USAGE_COMPLETION_TOKENS, GEN_AI_USAGE_PROMPT_TOKENS, LLM_SUBSTRINGS,
        OJ_AGENT_ERROR_MESSAGE, OJ_AGENT_INPUTS, OJ_AGENT_INVOKE_TYPE, OJ_AGENT_NAME,
        OJ_AGENT_OUTPUTS, OJ_CHILD_INVOKE_IDS, OJ_ELAPSED_TIME, OJ_END_TIME, OJ_ERROR,
        OJ_INNER_ERROR, OJ_INTERACTIVE_INPUTS, OJ_INVOKE_ID, OJ_META_DATA, OJ_PARENT_INVOKE_ID,
        OJ_PARENT_NODE_ID, OJ_SESSION_ID, OJ_SOURCE_IDS, OJ_START_TIME, OJ_STATUS,
        OJ_STREAM_INPUTS, OJ_STREAM_OUTPUTS, OJ_TRACE_ID, OJ_WORKFLOW_COMPONENT_ID,
        OJ_WORKFLOW_COMPONENT_NAME, OJ_WORKFLOW_COMPONENT_TYPE, OJ_WORKFLOW_ERROR_MESSAGE,
        OJ_WORKFLOW_EXECUTION_ID, OJ_WORKFLOW_ID, OJ_WORKFLOW_INPUTS, OJ_WORKFLOW_INVOKE_DATA,
        OJ_WORKFLOW_LOOP_INDEX, OJ_WORKFLOW_LOOP_NODE_ID, OJ_WORKFLOW_NAME, OJ_WORKFLOW_OUTPUTS,
        OJ_WORKFLOW_VERSION, OtelAgentSpanManager, OtelSpanKind, OtelSpanState, OtelTracer,
        OtelTracerConfig, OtelTracerError, OtelWorkflowSpanManager, ParentContextRef,
        REDACTED_PREFIX, TOOL_SUBSTRINGS, TRUNCATED_SUFFIX, format_elapsed, hash_value,
        is_llm_component, is_tool_component, is_workflow_root, redact, resolve_exporter,
        resolve_parent_context, serialize_value, should_redact, span_kind_for_component, truncate,
        validate_sample_rate, workflow_attrs, workflow_call_start_attrs, workflow_span_name,
    };
    pub use crate::trainer::{TrainEpoch, TrainRequest, TrainResult, Trainer, TrainerError};
    pub use crate::transport::{AgentHandler, AgentMessage, AgentTransport, TransportError};
    pub use crate::tune::{TuneError, TunePipeline, TuneRequest, TuneResult, TuneRoundResult};
    pub use crate::tune_kit::{
        Case, CaseLoader, EvaluatedCase, OptimizeHistory, Progress, TextualParameter, TraceNode,
        TuneKit, TuneKitError, TuneUtils, create_bad_case_text, evaluate_result_to_score,
        extract_optimized_prompt_from_response, find_missing_placeholders,
        find_placeholders_from_prompt, tune_constant,
    };
    pub use crate::web::{WebError, WebFetchRequest, WebFetchResult, WebProvider};
    pub use crate::workflow::{
        EdgeSpec, NodeKind, NodeSpec, WorkflowEngine, WorkflowError, WorkflowOutput, WorkflowSpec,
        WorkflowStreamSink,
    };
    pub use crate::workspace::{
        Goal, GoalStatus, WorkspaceError, WorkspaceManifest, WorkspaceService,
    };
    pub use crate::worktree::{
        MAX_SLUG_LENGTH, MEMBER_PART_LENGTH, MemberWorktreeInfo, TEAM_PART_LENGTH, WorktreeError,
        WorktreeMemberState, WorktreeNaming, WorktreeOwnerScope, build_teammate_worktree_name,
        info_from_options, info_matches_scope, matches_scope, sha256, sha256_hex_prefix, slug_part,
        validate_slug,
    };
}
