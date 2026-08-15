//! # ah-app
//!
//! boot 入口:读取 profile → 组装插件 → 返回已挂载的 Context。
//! demo(main.rs)与交互 CLI(bin/ah-cli.rs)共用本模块。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ah_contracts::keys::{AGENT_LOOP, SESSION_MANAGER};
use ah_contracts::session::SessionManager;
use ah_hub::context::Context;
use ah_hub::plugin::DynPlugin;
use ah_hub::profile::Profile;
use ah_plugins_agent_loop::AgentLoopPlugin;
use ah_plugins_agentbuilder::AgentBuilderPlugin;
use ah_plugins_anthropic::AnthropicPlugin;
use ah_plugins_autoharness::AutoHarnessPlugin;
use ah_plugins_ci::CiPlugin;
use ah_plugins_cli::CliPlugin;
use ah_plugins_code::CodePlugin;
use ah_plugins_context::ContextPlugin;
use ah_plugins_controller::ControllerPlugin;
use ah_plugins_credentials::CredentialsPlugin;
use ah_plugins_evolving::EvolvingPlugin;
use ah_plugins_external::ExternalCliPlugin;
use ah_plugins_git::GitPlugin;
use ah_plugins_graph_memory::GraphMemoryPlugin;
use ah_plugins_mcp::McpPlugin;
use ah_plugins_memory::MemoryPlugin;
use ah_plugins_mock::MockPlugin;
use ah_plugins_oauth::OAuthPlugin;
use ah_plugins_openai::OpenAiPlugin;
use ah_plugins_operator::OperatorPlugin;
use ah_plugins_optimizer::OptimizerPlugin;
use ah_plugins_pregel::PregelPlugin;
use ah_plugins_prompt::PromptPlugin;
use ah_plugins_queue::QueuePlugin;
use ah_plugins_queue::redis_queue::RedisQueuePlugin;
use ah_plugins_rails::{
    ApprovalRailPlugin, PathGuardRailPlugin, ShellGuardRailPlugin, ToolBudgetRailPlugin,
};
use ah_plugins_retrieval::RetrievalPlugin;
use ah_plugins_rl::RlPlugin;
use ah_plugins_rsi::RsiPlugin;
use ah_plugins_rsi::analyzer::AnalyzerPlugin;
use ah_plugins_rsi::single_harness::SingleHarnessPlugin;
use ah_plugins_runner::RunnerPlugin;
use ah_plugins_sandbox::{SandboxPlugin, SandboxRailPlugin};
use ah_plugins_security::SecurityRailPlugin;
use ah_plugins_session_log::SessionLogPlugin;
use ah_plugins_skill::SkillPlugin;
use ah_plugins_store::StorePlugin;
use ah_plugins_store::pg_store::PgStorePlugin;
use ah_plugins_store::redis_store::RedisStorePlugin;
use ah_plugins_subagent::SubagentPlugin;
use ah_plugins_subagents::SubagentsPlugin;
use ah_plugins_symphony::SymphonyPlugin;
use ah_plugins_sysop::SysopPlugin;
use ah_plugins_team_skill::TeamSkillPlugin;
use ah_plugins_teams::{SqliteTeamsPlugin, SwarmflowPlugin, TeamsPlugin};
use ah_plugins_telemetry::TelemetryPlugin;
use ah_plugins_tools::ToolsPlugin;
use ah_plugins_trainer::TrainerPlugin;
use ah_plugins_transport::TransportPlugin;
use ah_plugins_tune::TunePlugin;
use ah_plugins_web::WebPlugin;
use ah_plugins_workflow::WorkflowPlugin;
use ah_plugins_workspace::WorkspacePlugin;

/// 插件目录:名称 → 插件对象。
///
/// - ah-plugins-openai 惰性解析配置:apply 时先查 credentials seam
///   (openai.api_key),再 fallback 到 OPENAI_API_KEY 环境变量;
///   两者都无 key 时挂载显式失败(不静默降级)。
pub fn plugin_catalog(
    workspace_root: &Path,
    session_path: &PathBuf,
    session_dir: &PathBuf,
    memory_dir: &PathBuf,
    retrieval_dir: &PathBuf,
    telemetry_dir: &PathBuf,
) -> Vec<(&'static str, DynPlugin)> {
    let mut catalog: Vec<(&'static str, DynPlugin)> = vec![
        ("ah-plugins-mock", Arc::new(MockPlugin) as DynPlugin),
        (
            // 真实 credentials seam:环境变量 provider,无目录参数。
            "ah-plugins-credentials",
            Arc::new(CredentialsPlugin::default()) as DynPlugin,
        ),
        ("ah-plugins-tools", Arc::new(ToolsPlugin) as DynPlugin),
        (
            "ah-plugins-sysop",
            Arc::new(SysopPlugin::new(workspace_root)) as DynPlugin,
        ),
        (
            "ah-plugins-rails",
            Arc::new(ShellGuardRailPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-rails-path",
            Arc::new(PathGuardRailPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-rails-budget",
            Arc::new(ToolBudgetRailPlugin::new(100)) as DynPlugin,
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
        ("ah-plugins-subagent", Arc::new(SubagentPlugin) as DynPlugin),
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
            "ah-plugins-evolving",
            Arc::new(EvolvingPlugin::new(workspace_root.join("evolving"))) as DynPlugin,
        ),
        (
            "ah-plugins-rsi",
            Arc::new(RsiPlugin::new(workspace_root.join("rsi"))) as DynPlugin,
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
            "ah-plugins-controller",
            Arc::new(ControllerPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-store",
            Arc::new(StorePlugin::new(workspace_root.join("store"))) as DynPlugin,
        ),
        (
            "ah-plugins-store-redis",
            Arc::new(RedisStorePlugin::new("redis://127.0.0.1:6379/")) as DynPlugin,
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
            Arc::new(RedisQueuePlugin::new("redis://127.0.0.1:6379/")) as DynPlugin,
        ),
        (
            "ah-plugins-workspace",
            Arc::new(WorkspacePlugin::new(workspace_root)) as DynPlugin,
        ),
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
        ("ah-plugins-git", Arc::new(GitPlugin) as DynPlugin),
        (
            "ah-plugins-graph-memory",
            Arc::new(GraphMemoryPlugin::new(workspace_root.join("graph-memory"))) as DynPlugin,
        ),
        ("ah-plugins-ci", Arc::new(CiPlugin) as DynPlugin),
        ("ah-plugins-cli", Arc::new(CliPlugin) as DynPlugin),
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
        ("ah-plugins-runner", Arc::new(RunnerPlugin) as DynPlugin),
        (
            "ah-plugins-subagents",
            Arc::new(SubagentsPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-skill",
            Arc::new(SkillPlugin::new(workspace_root.join("skills"))) as DynPlugin,
        ),
        ("ah-plugins-pregel", Arc::new(PregelPlugin) as DynPlugin),
        ("ah-plugins-tune", Arc::new(TunePlugin) as DynPlugin),
        ("ah-plugins-trainer", Arc::new(TrainerPlugin) as DynPlugin),
        ("ah-plugins-oauth", Arc::new(OAuthPlugin) as DynPlugin),
        ("ah-plugins-operator", Arc::new(OperatorPlugin) as DynPlugin),
        (
            "ah-plugins-optimizer",
            Arc::new(OptimizerPlugin) as DynPlugin,
        ),
        (
            "ah-plugins-agentbuilder",
            Arc::new(AgentBuilderPlugin) as DynPlugin,
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
        (
            "ah-plugins-telemetry",
            Arc::new(TelemetryPlugin::new(telemetry_dir)) as DynPlugin,
        ),
        (
            "ah-plugins-agent-loop",
            Arc::new(AgentLoopPlugin::default()) as DynPlugin,
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

/// 按 profile 组装插件的结果:Context + 必须持有的注册 Effects。
pub type BootResult = (Context, Vec<ah_contracts::Effect>);

/// 按 profile 组装插件并返回已挂载的 Context 与注册 Effects。
///
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
    let profile = Profile::load(profile_path)?;
    let catalog = plugin_catalog(
        workspace_root,
        session_path,
        session_dir,
        memory_dir,
        retrieval_dir,
        telemetry_dir,
    );
    let plugins: Vec<DynPlugin> = profile
        .plugin_names()
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
    std::sync::Arc<ah_plugins_agent_loop::AgentLoop>,
    std::sync::Arc<dyn SessionManager>,
);

/// 解析 agent 循环与会话管理器(CLI 需要)。
pub fn agent_and_manager(ctx: &Context) -> Result<AgentManagerPair, Box<dyn std::error::Error>> {
    let agent = ctx
        .service::<ah_plugins_agent_loop::AgentLoop>(&AGENT_LOOP)
        .ok_or("agent-loop service not registered")?;
    let manager = ctx
        .service::<dyn SessionManager>(&SESSION_MANAGER)
        .ok_or("session-manager seam not registered")?;
    Ok((agent, manager))
}
