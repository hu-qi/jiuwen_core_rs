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
use ah_plugins_memory::MemoryPlugin;
use ah_plugins_mock::MockPlugin;
use ah_plugins_openai::OpenAiPlugin;
use ah_plugins_rails::ShellGuardRailPlugin;
use ah_plugins_retrieval::RetrievalPlugin;
use ah_plugins_session_log::SessionLogPlugin;
use ah_plugins_sysop::SysopPlugin;
use ah_plugins_tools::ToolsPlugin;
use ah_plugins_workflow::WorkflowPlugin;

/// 插件目录:名称 → 插件对象。
///
/// - ah-plugins-openai 仅在存在 OPENAI_API_KEY 时可用(真实 provider);
/// - 生产 profile 引用 openai 但无 key 时,解析会显式失败(不静默降级)。
pub fn plugin_catalog(
    workspace_root: &Path,
    session_path: &PathBuf,
    session_dir: &PathBuf,
    memory_dir: &PathBuf,
    retrieval_dir: &PathBuf,
) -> Vec<(&'static str, DynPlugin)> {
    let mut catalog: Vec<(&'static str, DynPlugin)> = vec![
        ("ah-plugins-mock", Arc::new(MockPlugin) as DynPlugin),
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
            "ah-plugins-agent-loop",
            Arc::new(AgentLoopPlugin::default()) as DynPlugin,
        ),
    ];
    if let Some(plugin) = OpenAiPlugin::from_env() {
        catalog.push(("ah-plugins-openai", Arc::new(plugin) as DynPlugin));
    }
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
) -> Result<BootResult, Box<dyn std::error::Error>> {
    let profile = Profile::load(profile_path)?;
    let catalog = plugin_catalog(
        workspace_root,
        session_path,
        session_dir,
        memory_dir,
        retrieval_dir,
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
