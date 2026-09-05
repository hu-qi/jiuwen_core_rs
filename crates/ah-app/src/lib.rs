//! # ah-app
//!
//! boot 入口:读取 profile → 组装插件 → 返回已挂载的 Context。
//! demo(main.rs)与交互 CLI(bin/ah-cli.rs)共用本模块。

pub mod audit;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ah_contracts::keys::{AGENT_LOOP, SESSION_MANAGER};
use ah_contracts::session::SessionManager;
use ah_hub::context::Context;
use ah_hub::plugin::DynPlugin;
use ah_hub::profile::Profile;
use ah_plugins_a2a::A2APlugin;
use ah_plugins_ability::AbilityPlugin;
use ah_plugins_agent_control::AgentControlPlugin;
use ah_plugins_agent_loop::AgentLoopPlugin;
use ah_plugins_agentbuilder::AgentBuilderPlugin;
use ah_plugins_anthropic::AnthropicPlugin;
use ah_plugins_application::ApplicationPlugin;
use ah_plugins_autoharness::AutoHarnessPlugin;
use ah_plugins_bridge_compose::BridgeComposePlugin;
use ah_plugins_checkpointer::{CheckpointerPlugin, redis_store::RedisCheckpointerStore};
use ah_plugins_ci::CiPlugin;
use ah_plugins_cli::{CliPlugin, TerminalPermissionApprovalPlugin};
use ah_plugins_code::CodePlugin;
use ah_plugins_common_tools::CommonToolsPlugin;
use ah_plugins_context::ContextPlugin;
use ah_plugins_context_evolver::ContextEvolverPlugin;
use ah_plugins_controller::ControllerPlugin;
use ah_plugins_credentials::CredentialsPlugin;
use ah_plugins_data_loader::DataLoaderPlugin;
use ah_plugins_dataset_curator::DatasetCuratorPlugin;
use ah_plugins_evolving::EvolvingPlugin;
use ah_plugins_experience_scorer::ExperienceScorerPlugin;
use ah_plugins_external::ExternalCliPlugin;
use ah_plugins_external::ExternalClientPlugin;
use ah_plugins_external_format::ExternalFormatPlugin;
use ah_plugins_git::GitPlugin;
use ah_plugins_graph_memory::GraphMemoryPlugin;
use ah_plugins_inbound_render::InboundRenderPlugin;
use ah_plugins_interaction_router::InteractionRouterPlugin;
use ah_plugins_json_parser::JsonParserPlugin;
use ah_plugins_kv_cache::KvcCachePlugin;
use ah_plugins_lsp::LspPlugin;
use ah_plugins_manifest::ManifestPlugin;
use ah_plugins_mcp::{BrowserMcpPlugin, McpPlugin};
use ah_plugins_member_optimizer::MemberOptimizerPlugin;
use ah_plugins_memory::MemoryPlugin;
use ah_plugins_memory_lite::MemoryLitePlugin;
use ah_plugins_messager::MessagerPlugin;
use ah_plugins_mock::MockPlugin;
use ah_plugins_model_allocator::ModelAllocatorPlugin;
use ah_plugins_model_backup::{ModelBackupPlugin, ModelBackupPolicyPlugin};
use ah_plugins_model_catalog::ModelCatalogPlugin;
use ah_plugins_oauth::OAuthPlugin;
use ah_plugins_openai::OpenAiPlugin;
use ah_plugins_operator::OperatorPlugin;
use ah_plugins_optimizer::OptimizerPlugin;
use ah_plugins_pregel::PregelPlugin;
use ah_plugins_prompt::PromptPlugin;
use ah_plugins_prompt_builder::PromptBuilderPlugin;
use ah_plugins_prompt_builder_devtools::PromptBuilderDevtoolsPlugin;
use ah_plugins_queue::QueuePlugin;
use ah_plugins_queue::redis_queue::RedisQueuePlugin;
use ah_plugins_rails::{
    ApprovalRailPlugin, GoalPlugin, PathGuardRailPlugin, ShellGuardRailPlugin,
    TaskPolicyRailPlugin, ToolBudgetRailPlugin,
};
use ah_plugins_reliability_burst::ReliabilityBurstPlugin;
use ah_plugins_reliability_monitor::ReliabilityMonitorPlugin;
use ah_plugins_reliability_tools::ReliabilityToolsPlugin;
use ah_plugins_rerank::RerankPlugin;
use ah_plugins_resources::ResourcesPlugin;
use ah_plugins_retrieval::RetrievalPlugin;
use ah_plugins_rl::RlPlugin;
use ah_plugins_rl_step::RlStepPlugin;
use ah_plugins_roster_diff::RosterDiffPlugin;
use ah_plugins_rsi::RsiPlugin;
use ah_plugins_rsi::analyzer::AnalyzerPlugin;
use ah_plugins_rsi::single_harness::SingleHarnessPlugin;
use ah_plugins_rsi_config::RsiConfigPlugin;
use ah_plugins_rsi_evaluator::RsiEvaluatorPlugin;
use ah_plugins_runner::RunnerPlugin;
use ah_plugins_sandbox::{SandboxPlugin, SandboxRailPlugin};
use ah_plugins_scheduler_render::SchedulerRenderPlugin;
use ah_plugins_stream::StreamPlugin;
use ah_plugins_tag_manager::TagManagerPlugin;
use ah_plugins_team_join_descriptor::TeamJoinDescriptorPlugin;
use ah_plugins_team_prompts::TeamPromptsPlugin;
use ah_plugins_team_schema::TeamSchemaPlugin;
use ah_plugins_team_task_status::TeamTaskStatusPlugin;
use ah_plugins_tools_metadata::ToolsMetadataPlugin;

use ah_plugins_prompt_attachment::PromptAttachmentPlugin;
use ah_plugins_security::{SecurityRailPlugin, TieredPolicyRailPlugin};
use ah_plugins_session_log::SessionLogPlugin;
use ah_plugins_sharing::{LocalSharingBackend, SharingPlugin};
use ah_plugins_signals::SignalsPlugin;
use ah_plugins_skill::SkillPlugin;
use ah_plugins_skill_creator::SkillCreatorPlugin;
use ah_plugins_store::StorePlugin;
use ah_plugins_store::pg_store::PgStorePlugin;
use ah_plugins_store::redis_store::RedisStorePlugin;
use ah_plugins_subagent::SubagentPlugin;
use ah_plugins_subagents::{MobileAdbPlugin, SubagentsPlugin};
use ah_plugins_symphony::SymphonyPlugin;
use ah_plugins_sysop::SysopPlugin;
use ah_plugins_team_context::TeamContextPlugin;
use ah_plugins_team_context_text::TeamContextTextPlugin;
use ah_plugins_team_dispatch::TeamDispatchPlugin;
use ah_plugins_team_i18n::TeamI18nPlugin;
use ah_plugins_team_message::TeamMessagePlugin;
use ah_plugins_team_monitor::TeamMonitorPlugin;
use ah_plugins_team_pool::TeamPoolPlugin;
use ah_plugins_team_scheduler::TeamSchedulerPlugin;
use ah_plugins_team_skill::TeamSkillPlugin;
use ah_plugins_team_skill_generator::TeamSkillGeneratorPlugin;
use ah_plugins_team_status::TeamStatusPlugin;
use ah_plugins_team_verdict::TeamVerdictPlugin;
use ah_plugins_teams::{SqliteTeamsPlugin, SwarmflowPlugin, TeamsPlugin};
use ah_plugins_telemetry::TelemetryPlugin;
use ah_plugins_timefmt::TimefmtPlugin;
use ah_plugins_tokenizer::TokenizerPlugin;
use ah_plugins_tools::ToolsPlugin;
use ah_plugins_tracer_otel::OtelTracerPlugin;
use ah_plugins_trainer::TrainerPlugin;
use ah_plugins_transport::TransportPlugin;
use ah_plugins_tune::TuneKitPlugin;
use ah_plugins_tune::TunePlugin;
use ah_plugins_web::WebPlugin;
use ah_plugins_workflow::WorkflowPlugin;
use ah_plugins_workspace::WorkspacePlugin;
use ah_plugins_worktree::WorktreePlugin;

/// 插件目录:名称 → 插件对象。
///
/// - ah-plugins-openai 惰性解析配置:apply 时先查 credentials seam
///   (openai.api_key),再 fallback 到 OPENAI_API_KEY 环境变量;
///   两者都无 key 时挂载显式失败(不静默降级)。
fn redis_url() -> String {
    std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379/".to_string())
}

pub fn plugin_catalog(
    workspace_root: &Path,
    session_path: &PathBuf,
    session_dir: &PathBuf,
    memory_dir: &PathBuf,
    retrieval_dir: &PathBuf,
    telemetry_dir: &PathBuf,
) -> Vec<(&'static str, DynPlugin)> {
    let redis_url = redis_url();
    let mut catalog: Vec<(&'static str, DynPlugin)> = vec![
        ("ah-plugins-mock", Arc::new(MockPlugin) as DynPlugin),
        (
            // 真实 credentials seam:环境变量 provider,无目录参数。
            "ah-plugins-credentials",
            Arc::new(CredentialsPlugin::default()) as DynPlugin,
        ),
        ("ah-plugins-tools", Arc::new(ToolsPlugin) as DynPlugin),
        (
            "ah-plugins-browser-mcp",
            Arc::new(BrowserMcpPlugin::from_env()) as DynPlugin,
        ),
        (
            "ah-plugins-mobile-adb",
            Arc::new(MobileAdbPlugin::from_env()) as DynPlugin,
        ),
        (
            "ah-plugins-common-tools",
            Arc::new(CommonToolsPlugin::new(workspace_root.to_path_buf())) as DynPlugin,
        ),
        ("ah-plugins-manifest", Arc::new(ManifestPlugin) as DynPlugin),
        (
            "ah-plugins-tokenizer",
            Arc::new(TokenizerPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-sysop",
            Arc::new(SysopPlugin::new(workspace_root)) as DynPlugin,
        ),
        (
            "ah-plugins-rails",
            Arc::new(ShellGuardRailPlugin) as DynPlugin,
        ),
        ("ah-plugins-goal", Arc::new(GoalPlugin) as DynPlugin),
        (
            "ah-plugins-rails-path",
            Arc::new(PathGuardRailPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-rails-budget",
            Arc::new(ToolBudgetRailPlugin::new(100)) as DynPlugin,
        ),
        (
            "ah-plugins-rails-policy",
            Arc::new(TaskPolicyRailPlugin::new(ah_contracts::rails::RailConfig {
                planning_prompt: Some(
                    "Plan the task in small verifiable steps; execute only after each step is checked."
                        .to_string(),
                ),
                max_rounds: Some(8),
                completion_promise: None,
                required_confirmations: 1,
                allow_promise_details: false,
                max_model_retries: 2,
                max_tool_retries: 1,
                retry_backoff_ms: vec![100, 250, 500],
            })) as DynPlugin,
        ),
        (
            "ah-plugins-rails-approval",
            Arc::new(ApprovalRailPlugin::new(
                workspace_root.join("approvals"),
                &[
                    "list_dir",
                    "read_file",
                    "write_file",
                    "remove_file",
                    "run_shell",
                    "web_fetch",
                    "run_code",
                    "ingest_knowledge",
                    "search_knowledge",
                    "delegate_task",
                    "remember",
                    "recall",
                    "forget",
                    "mcp_call_tool",
                ],
            )) as DynPlugin,
        ),
        (
            "ah-plugins-security",
            Arc::new(SecurityRailPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-security-tiered-policy",
            Arc::new(TieredPolicyRailPlugin::new(serde_json::json!({}))) as DynPlugin,
        ),
        ("ah-plugins-subagent", Arc::new(SubagentPlugin) as DynPlugin),
        (
            "ah-plugins-messager",
            Arc::new(MessagerPlugin::from_env()) as DynPlugin,
        ),
        ("ah-plugins-teams", Arc::new(TeamsPlugin) as DynPlugin),
        (
            "ah-plugins-teams-sqlite",
            Arc::new(SqliteTeamsPlugin::new(workspace_root.join("teams.db"))) as DynPlugin,
        ),
        (
            "ah-plugins-teams-workflow",
            Arc::new(SwarmflowPlugin::new(workspace_root.join("swarm-journals"))) as DynPlugin,
        ),
        (
            "ah-plugins-team-monitor",
            Arc::new(TeamMonitorPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-evolving",
            Arc::new(EvolvingPlugin::new(workspace_root.join("evolving"))) as DynPlugin,
        ),
        (
            "ah-plugins-rsi",
            Arc::new(RsiPlugin::new(workspace_root.join("rsi"))) as DynPlugin,
        ),
        (
            "ah-plugins-rsi-evaluator",
            Arc::new(RsiEvaluatorPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-rsi-analyzer",
            Arc::new(AnalyzerPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-rsi-single-harness",
            Arc::new(SingleHarnessPlugin::new(
                workspace_root.join("single-harness"),
            )) as DynPlugin,
        ),
        (
            "ah-plugins-context",
            Arc::new(ContextPlugin::new(workspace_root.join("context"))) as DynPlugin,
        ),
        (
            "ah-plugins-context-evolver",
            Arc::new(ContextEvolverPlugin::new(
                workspace_root.join("context-evolver"),
            )) as DynPlugin,
        ),
        (
            "ah-plugins-controller",
            Arc::new(ControllerPlugin::with_snapshot_path(
                workspace_root.join("controller/tasks.json"),
            )) as DynPlugin,
        ),
        (
            "ah-plugins-store",
            Arc::new(StorePlugin::new(workspace_root.join("store"))) as DynPlugin,
        ),
        (
            "ah-plugins-store-redis",
            Arc::new(RedisStorePlugin::new(redis_url.clone())) as DynPlugin,
        ),
        (
            "ah-plugins-store-pg",
            Arc::new(PgStorePlugin::new(
                "postgres://postgres:ah@127.0.0.1:64329/ah",
            )) as DynPlugin,
        ),
        (
            "ah-plugins-prompt",
            Arc::new(PromptPlugin::new(workspace_root.join("prompts"))) as DynPlugin,
        ),
        (
            "ah-plugins-queue",
            Arc::new(QueuePlugin::new(workspace_root.join("queue"))) as DynPlugin,
        ),
        (
            "ah-plugins-queue-redis",
            Arc::new(RedisQueuePlugin::new(redis_url.clone())) as DynPlugin,
        ),
        (
            "ah-plugins-workspace",
            Arc::new(WorkspacePlugin::new(workspace_root)) as DynPlugin,
        ),
        ("ah-plugins-worktree", Arc::new(WorktreePlugin) as DynPlugin),
        (
            "ah-plugins-sandbox",
            Arc::new(SandboxPlugin::new(workspace_root.join("sandbox"))) as DynPlugin,
        ),
        (
            "ah-plugins-code",
            Arc::new(CodePlugin::new(workspace_root.join("scratch"))) as DynPlugin,
        ),
        ("ah-plugins-web", Arc::new(WebPlugin) as DynPlugin),
        (
            "ah-plugins-transport",
            Arc::new(TransportPlugin::new(ah_contracts::transport::AgentCard {
                name: "agent-harness".to_string(),
                description: "agent-harness local agent endpoint".to_string(),
                url: "http://127.0.0.1:0/".to_string(),
                skills: vec!["agent".to_string()],
            })) as DynPlugin,
        ),
        ("ah-plugins-a2a", Arc::new(A2APlugin) as DynPlugin),
        ("ah-plugins-git", Arc::new(GitPlugin) as DynPlugin),
        (
            "ah-plugins-graph-memory",
            Arc::new(GraphMemoryPlugin::new(workspace_root.join("graph-memory"))) as DynPlugin,
        ),
        ("ah-plugins-ci", Arc::new(CiPlugin) as DynPlugin),
        (
            "ah-plugins-bridge-compose",
            Arc::new(BridgeComposePlugin) as DynPlugin,
        ),
        (
            "ah-plugins-checkpointer",
            // 真实 Redis checkpointer;连接失败显式阻止 production boot。
            Arc::new(CheckpointerPlugin::new(Arc::new(|info| {
                RedisCheckpointerStore::open(&info.url)
                    .map(|store| Arc::new(store) as Arc<dyn ah_contracts::checkpointer::RedisStore>)
            }))) as DynPlugin,
        ),
        ("ah-plugins-cli", Arc::new(CliPlugin) as DynPlugin),
        (
            "ah-plugins-cli-permission-ui",
            Arc::new(TerminalPermissionApprovalPlugin::new(
                workspace_root.join("permissions/approval_overrides.json"),
            )) as DynPlugin,
        ),
        (
            // 真实外部 CLI 运行时:通用流式 adapter(boot 不拉起,首次 start 才 spawn)。
            "ah-plugins-external",
            Arc::new(ExternalCliPlugin::new(
                ah_contracts::external::CliAgentAdapter::generic_streaming("__DONE__"),
            )) as DynPlugin,
        ),
        (
            "ah-plugins-autoharness",
            Arc::new(AutoHarnessPlugin) as DynPlugin,
        ),
        ("ah-plugins-rl", Arc::new(RlPlugin) as DynPlugin),
        ("ah-plugins-rl-step", Arc::new(RlStepPlugin) as DynPlugin),
        (
            "ah-plugins-resources",
            Arc::new(ResourcesPlugin) as DynPlugin,
        ),
        ("ah-plugins-rerank", Arc::new(RerankPlugin) as DynPlugin),
        (
            "ah-plugins-experience-scorer",
            Arc::new(ExperienceScorerPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-json-parser",
            Arc::new(JsonParserPlugin) as DynPlugin,
        ),
        ("ah-plugins-kv-cache", Arc::new(KvcCachePlugin) as DynPlugin),
        (
            "ah-plugins-model-allocator",
            Arc::new(ModelAllocatorPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-model-catalog",
            Arc::new(ModelCatalogPlugin) as DynPlugin,
        ),
        ("ah-plugins-signals", Arc::new(SignalsPlugin) as DynPlugin),
        (
            "ah-plugins-team-dispatch",
            Arc::new(TeamDispatchPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-team-pool",
            Arc::new(TeamPoolPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-team-status",
            Arc::new(TeamStatusPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-team-verdict",
            Arc::new(TeamVerdictPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-team-message",
            Arc::new(TeamMessagePlugin) as DynPlugin,
        ),
        (
            "ah-plugins-data-loader",
            Arc::new(DataLoaderPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-dataset-curator",
            Arc::new(DatasetCuratorPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-timefmt",
            Arc::new(TimefmtPlugin::new()) as DynPlugin,
        ),
        (
            "ah-plugins-team-scheduler",
            Arc::new(TeamSchedulerPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-roster-diff",
            Arc::new(RosterDiffPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-team-i18n",
            Arc::new(TeamI18nPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-team-context-text",
            Arc::new(TeamContextTextPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-team-context",
            Arc::new(TeamContextPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-scheduler-render",
            Arc::new(SchedulerRenderPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-team-join-descriptor",
            Arc::new(TeamJoinDescriptorPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-team-task-status",
            Arc::new(TeamTaskStatusPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-prompt-builder",
            Arc::new(PromptBuilderPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-prompt-builder-devtools",
            // 真实 dev_tools 提示构建器;LLM 模型由宿主注入。此默认模型显式报错,
            // 宿主注入真实模型前不可调用(不静默 fallback)。
            Arc::new(PromptBuilderDevtoolsPlugin::new(Arc::new(
                UninjectedPromptBuilderModel,
            ))) as DynPlugin,
        ),
        ("ah-plugins-stream", Arc::new(StreamPlugin) as DynPlugin),
        (
            "ah-plugins-prompt-attachment",
            Arc::new(PromptAttachmentPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-tag-manager",
            Arc::new(TagManagerPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-memory-lite",
            Arc::new(MemoryLitePlugin) as DynPlugin,
        ),
        (
            "ah-plugins-tools-metadata",
            Arc::new(ToolsMetadataPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-rsi-config",
            Arc::new(RsiConfigPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-reliability-burst",
            Arc::new(ReliabilityBurstPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-reliability-tools",
            Arc::new(ReliabilityToolsPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-reliability-monitor",
            Arc::new(ReliabilityMonitorPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-sharing",
            Arc::new(SharingPlugin::new(
                Arc::new(LocalSharingBackend::new(
                    std::env::temp_dir().join("ah-hub-local"),
                    0.85,
                )),
                Some(std::env::temp_dir().join("ah-sharing-cache")),
            )) as DynPlugin,
        ),
        ("ah-plugins-runner", Arc::new(RunnerPlugin) as DynPlugin),
        (
            "ah-plugins-subagents",
            Arc::new(SubagentsPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-skill",
            Arc::new(SkillPlugin::new(workspace_root.join("skills"))) as DynPlugin,
        ),
        (
            "ah-plugins-skill-creator",
            // 真实技能创建流水线;抓取器/LLM 生成器由宿主注入,缺失时显式报错。
            Arc::new(SkillCreatorPlugin::new(
                Arc::new(UninjectedSkillFetcher),
                Arc::new(UninjectedSkillGenerator),
            )) as DynPlugin,
        ),
        ("ah-plugins-pregel", Arc::new(PregelPlugin) as DynPlugin),
        ("ah-plugins-tune", Arc::new(TunePlugin) as DynPlugin),
        ("ah-plugins-tune-kit", Arc::new(TuneKitPlugin) as DynPlugin),
        (
            "ah-plugins-tracer-otel",
            Arc::new(OtelTracerPlugin) as DynPlugin,
        ),
        ("ah-plugins-trainer", Arc::new(TrainerPlugin) as DynPlugin),
        ("ah-plugins-oauth", Arc::new(OAuthPlugin) as DynPlugin),
        ("ah-plugins-operator", Arc::new(OperatorPlugin) as DynPlugin),
        (
            "ah-plugins-optimizer",
            Arc::new(OptimizerPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-symphony",
            Arc::new(SymphonyPlugin::new(workspace_root.join("symphony"))) as DynPlugin,
        ),
        (
            "ah-plugins-team-skill",
            Arc::new(TeamSkillPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-team-skill-generator",
            Arc::new(TeamSkillGeneratorPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-team-prompts",
            Arc::new(TeamPromptsPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-team-schema",
            Arc::new(TeamSchemaPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-sandbox-rail",
            Arc::new(SandboxRailPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-session-log",
            Arc::new(SessionLogPlugin::new(session_path, session_dir)) as DynPlugin,
        ),
        ("ah-plugins-workflow", Arc::new(WorkflowPlugin) as DynPlugin),
        (
            "ah-plugins-memory",
            Arc::new(MemoryPlugin::new(memory_dir)) as DynPlugin,
        ),
        (
            "ah-plugins-retrieval",
            Arc::new(RetrievalPlugin::new(retrieval_dir)) as DynPlugin,
        ),
        (
            // 真实 MCP stdio 客户端:懒 spawn(首次调用才拉起子进程),boot 无副作用。
            "ah-plugins-mcp",
            Arc::new(McpPlugin::new(
                "npx",
                vec![
                    "-y".to_string(),
                    "@modelcontextprotocol/server-everything".to_string(),
                ],
            )) as DynPlugin,
        ),
        ("ah-plugins-lsp", Arc::new(LspPlugin) as DynPlugin),
        (
            "ah-plugins-member-optimizer",
            Arc::new(MemberOptimizerPlugin::new(
                vec![],
                String::new(),
                workspace_root.join("member-optimizer"),
            )) as DynPlugin,
        ),
        (
            "ah-plugins-telemetry",
            Arc::new(TelemetryPlugin::new(telemetry_dir)) as DynPlugin,
        ),
        (
            "ah-plugins-agent-control",
            Arc::new(AgentControlPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-agent-loop",
            Arc::new(AgentLoopPlugin::default()) as DynPlugin,
        ),
        (
            "ah-plugins-application",
            Arc::new(ApplicationPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-ability",
            Arc::new(AbilityPlugin::default()) as DynPlugin,
        ),
        (
            "ah-plugins-model-backup",
            Arc::new(ModelBackupPlugin::new(Vec::new())) as DynPlugin,
        ),
        (
            "ah-plugins-external-format",
            Arc::new(ExternalFormatPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-inbound-render",
            Arc::new(InboundRenderPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-interaction-router",
            Arc::new(InteractionRouterPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-agentbuilder",
            Arc::new(AgentBuilderPlugin) as DynPlugin,
        ),
        (
            // 真实外部成员客户端工厂(ExternalTeamClient):依赖 external-format /
            // team-message / team-i18n / team-context / timefmt / inbound-render
            // / team-prompt-loader seam,须排在它们之后挂载(apply 时解析,缺失显式报错)。
            "ah-plugins-external-client",
            Arc::new(ExternalClientPlugin) as DynPlugin,
        ),
    ];
    // 惰性解析:apply 时先查 credentials seam(openai.api_key),再 fallback 到
    // OPENAI_API_KEY 环境变量;两者都无 key 时挂载显式失败(不静默降级)。
    catalog.push((
        "ah-plugins-openai",
        Arc::new(OpenAiPlugin::lazy()) as DynPlugin,
    ));
    catalog.push((
        "ah-plugins-anthropic",
        Arc::new(AnthropicPlugin::lazy()) as DynPlugin,
    ));
    catalog
}

fn tiered_policy_config(profile: &Profile) -> Result<Option<serde_json::Value>, String> {
    let Some(bundle) = profile.bundles.iter().find(|bundle| {
        bundle
            .plugins
            .iter()
            .any(|name| name == "ah-plugins-security-tiered-policy")
    }) else {
        return Ok(None);
    };
    let config = bundle
        .config
        .as_ref()
        .ok_or_else(|| "tiered policy bundle requires a config table".to_string())?;
    let config = serde_json::to_value(config)
        .map_err(|error| format!("invalid tiered policy profile configuration: {error}"))?;
    if !config.is_object() {
        return Err("tiered policy configuration must be a TOML table".to_string());
    }
    Ok(Some(config))
}

fn model_backup_policy(
    profile: &Profile,
) -> Result<ah_contracts::model_backup::ModelBackupPolicy, String> {
    let config = profile
        .bundles
        .iter()
        .filter(|bundle| {
            bundle
                .plugins
                .iter()
                .any(|name| name == "ah-plugins-model-backup")
        })
        .find_map(|bundle| bundle.config.as_ref());
    let retries_per_model = match config.and_then(|value| value.get("retries_per_model")) {
        None => 0,
        Some(value) => value
            .as_integer()
            .filter(|value| *value >= 0)
            .ok_or_else(|| "retries_per_model must be a non-negative integer".to_string())?
            as usize,
    };
    let attempt_timeout_ms = match config.and_then(|value| value.get("attempt_timeout_ms")) {
        None => None,
        Some(value) => Some(
            value
                .as_integer()
                .filter(|value| *value > 0)
                .ok_or_else(|| "attempt_timeout_ms must be a positive integer".to_string())?
                as u64,
        ),
    };
    Ok(ah_contracts::model_backup::ModelBackupPolicy {
        attempt_timeout_ms,
        retries_per_model,
    })
}

fn controller_snapshot_path(profile: &Profile, workspace_root: &Path) -> Result<PathBuf, String> {
    let configured = profile
        .bundles
        .iter()
        .filter(|bundle| {
            bundle
                .plugins
                .iter()
                .any(|name| name == "ah-plugins-controller")
        })
        .find_map(|bundle| bundle.config.as_ref())
        .and_then(|config| config.get("task_snapshot_path"));
    let Some(value) = configured else {
        return Ok(workspace_root.join("controller/tasks.json"));
    };
    let path = value
        .as_str()
        .filter(|path| !path.trim().is_empty())
        .ok_or_else(|| "task_snapshot_path must be a non-empty string".to_string())?;
    let relative = PathBuf::from(path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| component == std::path::Component::ParentDir)
    {
        return Err("task_snapshot_path must remain within workspace".to_string());
    }
    Ok(workspace_root.join(relative))
}

fn model_backup_provider_names(profile: &Profile) -> Vec<String> {
    profile
        .bundles
        .iter()
        .filter(|bundle| {
            bundle
                .plugins
                .iter()
                .any(|name| name == "ah-plugins-model-backup")
        })
        .find_map(|bundle| bundle.config.as_ref())
        .and_then(|config| config.get("backup_providers"))
        .and_then(toml::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(toml::Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// 按 profile 组装插件的结果:Context + 必须持有的注册 Effects。
pub type BootResult = (Context, Vec<ah_contracts::Effect>);

/// 按 profile 组装插件并返回已挂载的 Context 与注册 Effects。
///
fn parse_env_line(line: &str, line_number: usize) -> Result<Option<(String, String)>, String> {
    let line = line.trim_start_matches('\u{feff}').trim();
    if line.is_empty() || line.starts_with('#') {
        return Ok(None);
    }
    let assignment = line.strip_prefix("export ").unwrap_or(line);
    let (raw_key, raw_value) = assignment
        .split_once('=')
        .ok_or_else(|| format!("invalid env file line {line_number}: expected KEY=VALUE"))?;
    let key = raw_key.trim();
    if key.is_empty()
        || !key
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Err(format!("invalid env key on line {line_number}"));
    }
    let value = raw_value.trim();
    let value = if value.starts_with('"') {
        if !value.ends_with('"') || value.len() < 2 {
            return Err(format!(
                "unterminated double-quoted env value on line {line_number}"
            ));
        }
        let mut unescaped = String::new();
        let mut escaped = false;
        for character in value[1..value.len() - 1].chars() {
            if escaped {
                unescaped.push(match character {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    other => other,
                });
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else {
                unescaped.push(character);
            }
        }
        if escaped {
            return Err(format!("invalid escape on line {line_number}"));
        }
        unescaped
    } else if value.starts_with('\'') {
        if !value.ends_with('\'') || value.len() < 2 {
            return Err(format!(
                "unterminated single-quoted env value on line {line_number}"
            ));
        }
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
    };
    Ok(Some((key.to_string(), value)))
}

#[cfg(test)]
fn load_env_file(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    load_env_file_mode(path, false)
}

const ENV_FILE_CONTROLLED_VARS: &[&str] = &[
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "OPENAI_MODEL",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_MODEL",
    "REDIS_URL",
];

fn clear_env_file_controlled_vars() {
    for key in ENV_FILE_CONTROLLED_VARS {
        // SAFETY: boot establishes the application configuration before spawning work.
        unsafe { std::env::remove_var(key) };
    }
}

fn load_env_file_mode(
    path: &Path,
    override_existing: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(path)
        .map_err(|error| format!("read env file {}: {error}", path.display()))?;
    for (index, line) in content.lines().enumerate() {
        if let Some((key, value)) = parse_env_line(line, index + 1)
            .map_err(|error| format!("{}: {error}", path.display()))?
            && (override_existing || std::env::var_os(&key).is_none())
        {
            // SAFETY: boot loads configuration before starting application
            // work. Explicit AH_ENV_FILE is an intentional configuration source.
            unsafe { std::env::set_var(key, value) };
        }
    }
    Ok(())
}

fn load_env_for_boot(profile_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(path) = std::env::var_os("AH_ENV_FILE") {
        let path = PathBuf::from(path);
        if path.as_os_str().is_empty() {
            return Err("AH_ENV_FILE must not be empty".into());
        }
        clear_env_file_controlled_vars();
        return load_env_file_mode(&path, true);
    }
    let mut candidates = vec![std::env::current_dir()?.join(".env")];
    let profile = Path::new(profile_path);
    if let Some(parent) = profile.parent().and_then(Path::parent) {
        let candidate = parent.join(".env");
        if !candidates.iter().any(|path| path == &candidate) {
            candidates.push(candidate);
        }
    }
    let Some(path) = candidates.into_iter().find(|path| path.is_file()) else {
        return Err("no env file found; configure AH_ENV_FILE or provide .env".into());
    };
    clear_env_file_controlled_vars();
    // 启动配置以 env 文件为权威来源,避免继承的全局变量污染 provider 选择。
    load_env_file_mode(&path, true)?;
    Ok(())
}

/// 按 profile 组装插件的结果:Context + 必须持有的注册 Effects。
/// **调用方必须持有返回的 Effects 直到不再需要服务**(drop 即反注册)。
pub fn boot(
    profile_path: &str,
    workspace_root: &Path,
    session_path: &PathBuf,
    session_dir: &PathBuf,
    memory_dir: &PathBuf,
    retrieval_dir: &PathBuf,
    telemetry_dir: &PathBuf,
) -> Result<BootResult, Box<dyn std::error::Error>> {
    load_env_for_boot(profile_path)?;
    let profile = Profile::load(profile_path)?;
    let configured_backup_names = model_backup_provider_names(&profile);
    let configured_controller_snapshot = controller_snapshot_path(&profile, workspace_root)
        .map_err(|message| format!("invalid controller profile configuration: {message}"))?;
    let configured_backup_policy = model_backup_policy(&profile)
        .map_err(|message| format!("invalid model-backup profile configuration: {message}"))?;
    let configured_tiered_policy = tiered_policy_config(&profile)?;
    let mut catalog = plugin_catalog(
        workspace_root,
        session_path,
        session_dir,
        memory_dir,
        retrieval_dir,
        telemetry_dir,
    );
    if let Some(config) = configured_tiered_policy
        && let Some((_, plugin)) = catalog
            .iter_mut()
            .find(|(name, _)| *name == "ah-plugins-security-tiered-policy")
    {
        *plugin = Arc::new(TieredPolicyRailPlugin::new(config)) as DynPlugin;
    }
    if let Some((_, plugin)) = catalog
        .iter_mut()
        .find(|(name, _)| *name == "ah-plugins-controller")
    {
        *plugin = Arc::new(ControllerPlugin::with_snapshot_path(
            configured_controller_snapshot,
        )) as DynPlugin;
    }
    if configured_backup_policy != ah_contracts::model_backup::ModelBackupPolicy::default() {
        let policy_plugin = Arc::new(ModelBackupPolicyPlugin {
            policy: configured_backup_policy,
        }) as DynPlugin;
        let insert_at = catalog
            .iter()
            .position(|(name, _)| *name == "ah-plugins-model-backup")
            .unwrap_or(catalog.len());
        catalog.insert(insert_at, ("ah-plugins-model-backup-policy", policy_plugin));
    }
    if !configured_backup_names.is_empty()
        && let Some((_, plugin)) = catalog
            .iter_mut()
            .find(|(name, _)| *name == "ah-plugins-model-backup")
    {
        *plugin = Arc::new(
            ModelBackupPlugin::new(Vec::new()).with_provider_names(configured_backup_names),
        ) as DynPlugin;
    }
    let mut plugin_names = profile.plugin_names();
    if configured_backup_policy != ah_contracts::model_backup::ModelBackupPolicy::default()
        && !plugin_names
            .iter()
            .any(|name| name == "ah-plugins-model-backup-policy")
    {
        plugin_names.push("ah-plugins-model-backup-policy".to_string());
    }
    let plugins: Vec<DynPlugin> = plugin_names
        .iter()
        .map(|name| {
            catalog
                .iter()
                .find(|(candidate, _)| *candidate == name)
                .map(|(_, plugin)| plugin.clone())
                .ok_or_else(|| {
                    format!(
                        "unknown or unavailable plugin: {name} (real providers may need credentials)"
                    )
                })
        })
        .collect::<Result<_, _>>()?;

    let ctx = Context::new();
    let effects = ctx.mount_all(plugins)?;
    Ok((ctx, effects))
}

/// agent 循环 + 会话管理器的解析结果。
pub type AgentManagerPair = (
    std::sync::Arc<dyn ah_contracts::agent::AgentLoopRuntime>,
    std::sync::Arc<dyn SessionManager>,
);

/// 解析 agent 循环与会话管理器(CLI 需要)。
pub fn agent_and_manager(ctx: &Context) -> Result<AgentManagerPair, Box<dyn std::error::Error>> {
    let agent = ctx
        .service::<dyn ah_contracts::agent::AgentLoopRuntime>(&AGENT_LOOP)
        .ok_or("agent-loop service not registered")?;
    let manager = ctx
        .service::<dyn SessionManager>(&SESSION_MANAGER)
        .ok_or("session-manager seam not registered")?;
    Ok((agent, manager))
}

/// 未注入 LLM 模型的 dev_tools 提示构建器模型:任何调用显式报错。
pub struct UninjectedPromptBuilderModel;

impl ah_contracts::prompt_builder_devtools::PromptBuilderModel for UninjectedPromptBuilderModel {
    fn invoke(
        &self,
        _messages: &[ah_contracts::prompt_builder_devtools::ChatMessageView],
    ) -> Result<Option<String>, ah_contracts::prompt_builder_devtools::PromptBuilderError> {
        Err(ah_contracts::prompt_builder_devtools::PromptBuilderError(
            "no LLM model injected for prompt-builder-devtools".to_string(),
        ))
    }
}
pub struct UninjectedSkillFetcher;

impl ah_contracts::skill_creator::SkillFetcher for UninjectedSkillFetcher {
    fn fetch(
        &self,
        url: &str,
    ) -> Result<(Vec<u8>, String), ah_contracts::skill_creator::SkillCreatorError> {
        Err(ah_contracts::skill_creator::SkillCreatorError(format!(
            "no fetcher injected for skill-creator (url: {url})"
        )))
    }
}

/// 未注入 LLM 生成器的技能创建生成器:任何调用显式报错。
pub struct UninjectedSkillGenerator;

impl ah_contracts::skill_creator::SkillGenerator for UninjectedSkillGenerator {
    fn generate_skill_md(
        &self,
        spec: &ah_contracts::skill_creator::SkillGenRequest,
    ) -> Result<String, ah_contracts::skill_creator::SkillCreatorError> {
        Err(ah_contracts::skill_creator::SkillCreatorError(format!(
            "no LLM generator injected for skill-creator (slug: {})",
            spec.slug
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        controller_snapshot_path, load_env_file, load_env_for_boot, model_backup_policy,
        model_backup_provider_names, parse_env_line, redis_url, tiered_policy_config,
    };
    use ah_hub::profile::Profile;
    #[test]
    fn parses_dotenv_lines_without_expanding_values() {
        assert_eq!(
            parse_env_line("export OPENAI_MODEL=deepseek-chat", 1).unwrap(),
            Some(("OPENAI_MODEL".into(), "deepseek-chat".into()))
        );
        assert_eq!(
            parse_env_line("OPENAI_BASE_URL=\"https://example.test/v1\"", 2).unwrap(),
            Some(("OPENAI_BASE_URL".into(), "https://example.test/v1".into()))
        );
        assert!(parse_env_line("OPENAI-API-KEY=bad", 3).is_err());
    }

    #[test]
    fn loads_dotenv_value_without_overwriting_process_environment() {
        let key = format!("AH_APP_ENV_FILE_TEST_{}", std::process::id());
        let path = std::env::temp_dir().join(format!("ah-app-env-{}.env", std::process::id()));
        let previous = std::env::var_os(&key);
        unsafe { std::env::remove_var(&key) };
        std::fs::write(&path, format!("{key}=loaded\n")).expect("write env file");
        load_env_file(&path).expect("load env file");
        assert_eq!(std::env::var(&key).as_deref(), Ok("loaded"));
        match previous {
            Some(value) => unsafe { std::env::set_var(&key, value) },
            None => unsafe { std::env::remove_var(&key) },
        }
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn explicit_env_file_path_is_loaded_before_profile() {
        let key = format!("AH_APP_EXPLICIT_ENV_TEST_{}", std::process::id());
        let path = std::env::temp_dir().join(format!("ah-app-explicit-{}.env", std::process::id()));
        let previous_key = std::env::var_os(&key);
        let previous_file = std::env::var_os("AH_ENV_FILE");
        unsafe {
            std::env::set_var(&key, "inherited");
            std::env::set_var("AH_ENV_FILE", &path);
        }
        std::fs::write(&path, format!("{key}=explicit\n")).expect("write env file");
        load_env_for_boot("profiles/prod.toml").expect("load explicit env file");
        assert_eq!(std::env::var(&key).as_deref(), Ok("explicit"));
        match previous_key {
            Some(value) => unsafe { std::env::set_var(&key, value) },
            None => unsafe { std::env::remove_var(&key) },
        }
        match previous_file {
            Some(value) => unsafe { std::env::set_var("AH_ENV_FILE", value) },
            None => unsafe { std::env::remove_var("AH_ENV_FILE") },
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn redis_url_uses_environment_and_has_local_default() {
        let previous = std::env::var_os("REDIS_URL");
        unsafe { std::env::remove_var("REDIS_URL") };
        assert_eq!(redis_url(), "redis://127.0.0.1:6379/");
        unsafe { std::env::set_var("REDIS_URL", "redis://:secret@example.test:6380/2") };
        assert_eq!(redis_url(), "redis://:secret@example.test:6380/2");
        match previous {
            Some(value) => unsafe { std::env::set_var("REDIS_URL", value) },
            None => unsafe { std::env::remove_var("REDIS_URL") },
        }
    }

    #[test]
    fn parses_controller_snapshot_path_and_rejects_escape() {
        let profile = Profile::from_toml(
            r#"name = "test"
            [[bundles]]
            id = "controller"
            plugins = ["ah-plugins-controller"]
            [bundles.config]
            task_snapshot_path = "state/tasks.json"
        "#,
        )
        .unwrap();
        let root = std::path::PathBuf::from("/tmp/workspace");
        assert_eq!(
            controller_snapshot_path(&profile, &root).unwrap(),
            root.join("state/tasks.json")
        );
        let invalid = Profile::from_toml(
            r#"name = "test"
            [[bundles]]
            id = "controller"
            plugins = ["ah-plugins-controller"]
            [bundles.config]
            task_snapshot_path = "../escape.json"
        "#,
        )
        .unwrap();
        assert!(controller_snapshot_path(&invalid, &root).is_err());
    }

    #[test]
    fn parses_backup_policy_and_names_from_profile() {
        let profile = Profile::from_toml(
            r#"name = "test"
            [[bundles]]
            id = "models"
            plugins = ["ah-plugins-model-backup"]
            [bundles.config]
            backup_providers = ["secondary"]
            retries_per_model = 2
            attempt_timeout_ms = 1500
        "#,
        )
        .unwrap();
        assert_eq!(model_backup_provider_names(&profile), vec!["secondary"]);
        let policy = model_backup_policy(&profile).unwrap();
        assert_eq!(policy.retries_per_model, 2);
        assert_eq!(policy.attempt_timeout_ms, Some(1500));
    }

    #[test]
    fn rejects_non_positive_timeout() {
        let profile = Profile::from_toml(
            r#"name = "test"
            [[bundles]]
            id = "models"
            plugins = ["ah-plugins-model-backup"]
            [bundles.config]
            attempt_timeout_ms = 0
        "#,
        )
        .unwrap();
        assert!(model_backup_policy(&profile).is_err());
    }

    #[test]
    fn rejects_negative_retries() {
        let profile = Profile::from_toml(
            r#"name = "test"
            [[bundles]]
            id = "models"
            plugins = ["ah-plugins-model-backup"]
            [bundles.config]
            retries_per_model = -1
        "#,
        )
        .unwrap();
        assert!(model_backup_policy(&profile).is_err());
    }

    #[test]
    fn extracts_tiered_policy_config_from_profile_bundle() {
        let profile = Profile::from_toml(
            r#"name = "prod"
            [[bundles]]
            id = "security-policy"
            plugins = ["ah-plugins-security-tiered-policy"]
            [bundles.config]
            permission_mode = "strict"
            [bundles.config.defaults]
            "*" = "deny"
            [bundles.config.tools]
            read_file = "allow"
        "#,
        )
        .unwrap();
        let config = tiered_policy_config(&profile).unwrap().unwrap();
        assert_eq!(config["permission_mode"], "strict");
        assert_eq!(config["tools"]["read_file"], "allow");
    }
}
