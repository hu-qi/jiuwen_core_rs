//! agent-harness 启动入口:读取 profile → 组装插件 → 解析 seam → 真实执行。

use std::sync::Arc;

use ah_contracts::fs::FsProvider;
use ah_contracts::keys::{FS, LLM, SHELL, TOOLS};
use ah_contracts::llm::{ChatMessage, ChatRole, ModelProvider, ModelRequest};
use ah_contracts::shell::ShellProvider;
use ah_contracts::tools::ToolRegistry;
use ah_hub::context::Context;
use ah_hub::plugin::DynPlugin;
use ah_hub::profile::Profile;
use ah_plugins_mock::MockPlugin;
use ah_plugins_openai::OpenAiPlugin;
use ah_plugins_sysop::SysopPlugin;
use ah_plugins_tools::ToolsPlugin;
use serde_json::json;

/// 插件目录:名称 → 插件对象。
///
/// - ah-plugins-openai 仅在存在 OPENAI_API_KEY 时可用(真实 provider);
/// - 生产 profile 引用 openai 但无 key 时,解析会显式失败(不静默降级)。
fn plugin_catalog(workspace_root: &std::path::Path) -> Vec<(&'static str, DynPlugin)> {
    let mut catalog: Vec<(&'static str, DynPlugin)> = vec![
        ("ah-plugins-mock", Arc::new(MockPlugin) as DynPlugin),
        ("ah-plugins-tools", Arc::new(ToolsPlugin) as DynPlugin),
        (
            "ah-plugins-sysop",
            Arc::new(SysopPlugin::new(workspace_root)) as DynPlugin,
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

    // 真实临时 workspace:所有 fs/shell 副作用发生在这里。
    let workspace_root = std::env::temp_dir().join(format!("ah-app-{}", std::process::id()));
    println!("[boot] workspace: {}", workspace_root.display());

    let catalog = plugin_catalog(&workspace_root);
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
    println!(
        "[fs] read back: {}",
        String::from_utf8_lossy(&fs.read("notes/plan.md")?)
    );

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

    // tools seam:真实工具调用(write_file -> read_file 往返)。
    let registry = ctx
        .service::<dyn ToolRegistry>(&TOOLS)
        .ok_or("tools seam not registered")?;
    let mut names = registry.names();
    names.sort();
    println!("[tools] available: {}", names.join(", "));
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

    Ok(())
}
