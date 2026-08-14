//! agent-harness 启动入口:读取 profile → 组装插件 → 解析 seam → 执行。

use std::sync::Arc;

use ah_contracts::keys::{LLM, TOOLS};
use ah_contracts::llm::{ChatMessage, ChatRole, ModelProvider, ModelRequest};
use ah_contracts::tools::ToolRegistry;
use ah_hub::context::Context;
use ah_hub::plugin::DynPlugin;
use ah_hub::profile::Profile;
use ah_plugins_mock::MockPlugin;
use serde_json::json;

/// 插件目录:名称 → 插件对象。
///
/// 后续由动态注册 / 进程插件扩展;当前为静态目录。
fn plugin_catalog() -> Vec<(&'static str, DynPlugin)> {
    vec![("ah-plugins-mock", Arc::new(MockPlugin) as DynPlugin)]
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let profile = Profile::load("profiles/dev.toml")?;
    println!(
        "[boot] profile: {} ({})",
        profile.name,
        profile.plugin_names().join(", ")
    );

    let catalog = plugin_catalog();
    let plugins: Vec<DynPlugin> = profile
        .plugin_names()
        .iter()
        .map(|name| {
            catalog
                .iter()
                .find(|(candidate, _)| *candidate == name)
                .map(|(_, plugin)| plugin.clone())
                .ok_or_else(|| format!("unknown plugin: {name}"))
        })
        .collect::<Result<_, _>>()?;

    let ctx = Context::new();
    let _effects = ctx.mount_all(plugins)?;
    println!("[boot] mounted services: {:?}", ctx.service_keys());

    // llm seam
    let provider = ctx
        .service::<dyn ModelProvider>(&LLM)
        .ok_or("llm seam not registered; check profile")?;
    let response = provider
        .chat(ModelRequest {
            messages: vec![ChatMessage::new(ChatRole::User, "hello from ah-app")],
            ..Default::default()
        })
        .await?;
    println!("[llm] {}: {}", provider.name(), response.content);

    // tools seam
    let registry = ctx
        .service::<dyn ToolRegistry>(&TOOLS)
        .ok_or("tools seam not registered; check profile")?;
    let mut names = registry.names();
    names.sort();
    println!("[tools] available: {}", names.join(", "));
    let sum = registry.invoke("add", json!({ "a": 2, "b": 40 })).await?;
    println!("[tools] add(2, 40) = {sum}");

    Ok(())
}
