//! agent-harness 启动入口:读取 profile → 组装插件 → 解析 seam → 真实执行。

use std::path::PathBuf;
use std::sync::Arc;

use ah_contracts::fs::FsProvider;
use ah_contracts::keys::{AGENT_LOOP, FS, LLM, SESSIONS, SHELL, TOOLS};
use ah_contracts::llm::{ChatMessage, ChatRole, ModelProvider, ModelRequest};
use ah_contracts::session::SessionLog;
use ah_contracts::shell::ShellProvider;
use ah_contracts::tools::ToolRegistry;
use ah_hub::context::Context;
use ah_hub::plugin::DynPlugin;
use ah_hub::profile::Profile;
use ah_plugins_agent_loop::{AgentLoop, AgentLoopPlugin, AgentStep};
use ah_plugins_mock::MockPlugin;
use ah_plugins_openai::OpenAiPlugin;
use ah_plugins_rails::ShellGuardRailPlugin;
use ah_plugins_session_log::SessionLogPlugin;
use ah_plugins_sysop::SysopPlugin;
use ah_plugins_tools::ToolsPlugin;
use serde_json::json;

/// 插件目录:名称 → 插件对象。
///
/// - ah-plugins-openai 仅在存在 OPENAI_API_KEY 时可用(真实 provider);
/// - 生产 profile 引用 openai 但无 key 时,解析会显式失败(不静默降级)。
fn plugin_catalog(
    workspace_root: &std::path::Path,
    session_path: &PathBuf,
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
            Arc::new(SessionLogPlugin::new(session_path)) as DynPlugin,
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let profile_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "profiles/dev.toml".to_string());
    let profile = Profile::load(&profile_path)?;
    println!(
        "[boot] profile {}: {} ({})",
        profile_path,
        profile.name,
        profile.plugin_names().join(", ")
    );

    // 真实临时 workspace 与会话日志文件。
    let workspace_root = std::env::temp_dir().join(format!("ah-app-{}", std::process::id()));
    let session_path =
        std::env::temp_dir().join(format!("ah-app-session-{}.jsonl", std::process::id()));
    println!("[boot] workspace: {}", workspace_root.display());
    println!("[boot] session log: {}", session_path.display());

    let catalog = plugin_catalog(&workspace_root, &session_path);
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
    let _effects = ctx.mount_all(plugins)?;
    println!("[boot] mounted services: {:?}", ctx.service_keys());

    // llm seam(dev=mock 桩;prod=真实 openai-compatible)
    let provider = ctx
        .service::<dyn ModelProvider>(&LLM)
        .ok_or("llm seam not registered")?;
    let response = provider
        .chat(ModelRequest {
            messages: vec![ChatMessage::new(ChatRole::User, "hello from ah-app")],
            ..Default::default()
        })
        .await?;
    println!("[llm] {}: {}", provider.name(), response.content);

    // fs seam:真实写文件、列目录、读回。
    let fs = ctx
        .service::<dyn FsProvider>(&FS)
        .ok_or("fs seam not registered")?;
    fs.write("notes/plan.md", b"real file written by ah-app")?;
    println!("[fs] wrote notes/plan.md; entries: {:?}", fs.list("notes")?);

    // shell seam:真实执行命令。
    let shell = ctx
        .service::<dyn ShellProvider>(&SHELL)
        .ok_or("shell seam not registered")?;
    let output = shell
        .run(
            "sh",
            &["-c".to_string(), "echo real-process-output".to_string()],
            std::time::Duration::from_secs(10),
        )
        .await?;
    println!(
        "[shell] exit={} stdout={}",
        output.exit_code,
        output.stdout.trim()
    );

    // tools seam:真实工具调用 + 工具执行管线(rails)。
    let registry = ctx
        .service::<dyn ToolRegistry>(&TOOLS)
        .ok_or("tools seam not registered")?;
    let _ = registry
        .invoke(
            "write_file",
            json!({ "path": "via-tool.txt", "content": "written via real tool" }),
        )
        .await?;
    let read = registry
        .invoke("read_file", json!({ "path": "via-tool.txt" }))
        .await?;
    println!(
        "[tools] write_file -> read_file roundtrip: {}",
        read["content"]
    );
    let safe = registry
        .invoke(
            "run_shell",
            json!({ "command": "echo", "args": ["pipeline-ok"] }),
        )
        .await?;
    println!(
        "[tools] run_shell echo pipeline-ok -> exit={}",
        safe["exit_code"]
    );
    match registry
        .invoke("run_shell", json!({ "command": "rm -rf /" }))
        .await
    {
        Ok(_) => println!("[tools] WARNING: dangerous command was NOT blocked!"),
        Err(error) => println!("[tools] run_shell rm -rf / -> blocked: {error}"),
    }

    // agent-loop:真实 ReAct 循环,会话日志驱动。
    let _step_listener = ctx.on::<AgentStep>(|step| {
        println!(
            "[agent] step {}: tool_calls={} done={}",
            step.iteration, step.tool_calls, step.done
        );
    });
    let agent = ctx
        .service::<AgentLoop>(&AGENT_LOOP)
        .ok_or("agent-loop service not registered")?;
    let answer = agent.run("explore the workspace").await?;
    println!("[agent] answer: {answer}");

    // 会话日志:展示日志即真相(事件序列 + 投影消息)。
    let sessions = ctx
        .service::<dyn SessionLog>(&SESSIONS)
        .ok_or("sessions seam not registered")?;
    println!("[session] events:");
    for event in sessions.events() {
        println!(
            "  seq={} kind={:?} payload={}",
            event.seq,
            event.kind,
            serde_json::to_string(&event.payload)?
        );
    }
    let messages = sessions.derive_messages();
    println!("[session] projected messages: {}", messages.len());

    Ok(())
}
