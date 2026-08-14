//! agent-harness 启动入口:读取 profile → 组装插件 → 解析 seam → 执行。

use std::sync::Arc;

use ah_contracts::llm::{ChatMessage, ChatRole, ModelProvider, ModelRequest};
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::DynPlugin;
use ah_hub::profile::Profile;
use ah_plugins_mock::MockPlugin;

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

    let llm_key = ServiceKey::new("llm");
    let provider = ctx
        .service::<dyn ModelProvider>(&llm_key)
        .ok_or("llm seam not registered; check profile")?;

    let response = provider
        .chat(ModelRequest {
            messages: vec![ChatMessage::new(ChatRole::User, "hello from ah-app")],
            ..Default::default()
        })
        .await?;
    println!("[llm] {}: {}", provider.name(), response.content);

    Ok(())
}
