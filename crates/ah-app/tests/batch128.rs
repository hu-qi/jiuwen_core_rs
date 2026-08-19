//! 第 128 回合新增插件的真实路径集成测试:worktree / kv-cache / lsp /
//! resources / skill-creator / prompt-builder-devtools 挂载后全链路真实执行。

use std::sync::Arc;

use ah_contracts::keys::{
    KVC_HOOKS, LSP, PROMPT_BUILDER_DEVTOOLS, RESOURCES, SKILL_CREATOR, WORKTREE_MEMBER_STATE,
    WORKTREE_NAMING,
};
use ah_contracts::prelude::Effect;
use ah_hub::context::Context;
use ah_hub::plugin::DynPlugin;

fn mount(ctx: &Context, plugins: Vec<DynPlugin>) -> Vec<Effect> {
    ctx.mount_all(plugins).expect("mount")
}

fn base_plugins() -> Vec<DynPlugin> {
    vec![
        Arc::new(ah_plugins_manifest::ManifestPlugin),
        Arc::new(ah_plugins_worktree::WorktreePlugin),
        Arc::new(ah_plugins_kv_cache::KvcCachePlugin),
        Arc::new(ah_plugins_resources::ResourcesPlugin),
        Arc::new(ah_plugins_lsp::LspPlugin),
    ]
}

/// 显式报错的 skill-creator 抓取器(模拟未接线宿主)。
struct StubFetcher;
impl ah_contracts::skill_creator::SkillFetcher for StubFetcher {
    fn fetch(
        &self,
        url: &str,
    ) -> Result<(Vec<u8>, String), ah_contracts::skill_creator::SkillCreatorError> {
        Err(ah_contracts::skill_creator::SkillCreatorError(format!(
            "no fetcher injected (url: {url})"
        )))
    }
}

struct StubGenerator;
impl ah_contracts::skill_creator::SkillGenerator for StubGenerator {
    fn generate_skill_md(
        &self,
        spec: &ah_contracts::skill_creator::SkillGenRequest,
    ) -> Result<String, ah_contracts::skill_creator::SkillCreatorError> {
        Ok(format!("# {}\n\n生成的技能内容", spec.title))
    }
}

struct StubPromptModel;
impl ah_contracts::prompt_builder_devtools::PromptBuilderModel for StubPromptModel {
    fn invoke(
        &self,
        messages: &[ah_contracts::prompt_builder_devtools::ChatMessageView],
    ) -> Result<Option<String>, ah_contracts::prompt_builder_devtools::PromptBuilderError> {
        let content = messages
            .last()
            .map(|m| m.content.clone())
            .unwrap_or_default();
        Ok(Some(format!("<summary>{content}</summary>")))
    }
}

#[test]
fn worktree_seams_mount_and_build_name() {
    let ctx = Context::new();
    let effects = mount(&ctx, base_plugins());
    let naming = ctx
        .service::<dyn ah_contracts::worktree::WorktreeNaming>(&WORKTREE_NAMING)
        .expect("naming");
    let state = ctx
        .service::<dyn ah_contracts::worktree::WorktreeMemberState>(&WORKTREE_MEMBER_STATE)
        .expect("state");
    let name = naming
        .build_teammate_worktree_name("TeamX", "Alice", "s1", "h1")
        .expect("name");
    assert!(name.starts_with("agent-teamx-alice-"));
    let scope = ah_contracts::worktree::WorktreeOwnerScope {
        team_name: "TeamX".into(),
        member_name: "Alice".into(),
        session_id: "s1".into(),
        project_dir: "/p".into(),
        project_hash: "h1".into(),
        managed_root: "/r".into(),
        worktree_name: name.clone(),
    };
    let info = state
        .info_from_options("/w", None, None, Some("s1"), Some("h1"), Some("/r"), &scope)
        .expect("info");
    assert_eq!(info.worktree_name, name);
    drop(effects);
}

#[test]
fn kv_cache_hooks_mount_and_skip_when_disabled() {
    let ctx = Context::new();
    let effects = mount(&ctx, base_plugins());
    let hooks = ctx
        .service::<dyn ah_contracts::kv_cache::KvcHooks>(&KVC_HOOKS)
        .expect("hooks");
    assert!(hooks.is_sticky_subagent_type("browser_agent"));
    assert_eq!(hooks.resolve_sub_session_id("t", "p", None), "p_sub_t");
    // 无模型时动作直接返回(不 panic)。
    hooks.prefetch_sticky_subagent(None, true, "browser_agent", "sub", "parent");
    hooks.finish_subagent(None, true, "browser_agent", "sub", "parent", true);
    hooks.evict_subagent(None, true, "sub", "parent");
    drop(effects);
}

#[test]
fn resources_mount_and_resolve_plugin_parts() {
    let ctx = Context::new();
    let effects = mount(&ctx, base_plugins());
    let resolver = ctx
        .service::<dyn ah_contracts::resources::ResourcesResolver>(&RESOURCES)
        .expect("resources");
    let spec = ah_contracts::resources::PluginSpec {
        id: "itest".to_string(),
        name: None,
        description: None,
        prompt_sections: vec![ah_contracts::resources::PromptSectionSpec {
            name: "identity".to_string(),
            content: std::collections::BTreeMap::from([(
                "en".to_string(),
                "I am {{ language }}".to_string(),
            )]),
            priority: 10,
            render_params: serde_json::json!({}),
        }],
        tools: vec![],
        mcps: vec![],
        rails: vec![],
        skills: vec![],
        metadata: serde_json::Value::Object(Default::default()),
    };
    let parts = resolver
        .resolve_plugin_parts(&spec, "en", Some("/ws"))
        .expect("resolve");
    assert_eq!(parts.prompt_sections[0].content["en"], "I am en");
    drop(effects);
}

#[test]
fn lsp_mount_and_diagnostics() {
    let ctx = Context::new();
    let effects = mount(&ctx, base_plugins());
    let lsp = ctx
        .service::<dyn ah_contracts::lsp::LspService>(&LSP)
        .expect("lsp");
    lsp.initialize(&ah_contracts::lsp::InitializeOptions::default())
        .expect("init");
    let status = lsp.status();
    assert_eq!(status.len(), 5);
    assert!(
        status
            .iter()
            .all(|s| s.state == ah_contracts::lsp::LspServerState::Stopped)
    );
    drop(effects);
}

#[test]
fn skill_creator_mount_and_explicit_error_on_stub_fetcher() {
    let ctx = Context::new();
    let mut plugins = base_plugins();
    plugins.push(Arc::new(ah_plugins_skill_creator::SkillCreatorPlugin::new(
        Arc::new(StubFetcher),
        Arc::new(StubGenerator),
    )) as DynPlugin);
    let effects = mount(&ctx, plugins);
    let creator = ctx
        .service::<dyn ah_contracts::skill_creator::SkillCreator>(&SKILL_CREATOR)
        .expect("skill-creator");
    // 抓取器未接线 → 显式错误(不静默)。
    let err = creator
        .create_skill("https://example.com/x", "测试", "zh-CN")
        .expect_err("explicit fetch error");
    assert!(err.0.contains("no fetcher injected"));
    drop(effects);
}

#[test]
fn prompt_builder_devtools_mount_and_validate() {
    let ctx = Context::new();
    let mut plugins = base_plugins();
    plugins.push(Arc::new(
        ah_plugins_prompt_builder_devtools::PromptBuilderDevtoolsPlugin::new(Arc::new(
            StubPromptModel,
        )),
    ) as DynPlugin);
    let effects = mount(&ctx, plugins);
    let builder = ctx
        .service::<dyn ah_contracts::prompt_builder_devtools::PromptBuilder>(
            &PROMPT_BUILDER_DEVTOOLS,
        )
        .expect("prompt-builder-devtools");
    // 空 cases → 显式错误。
    let err = builder
        .build_bad_case("p", &[], "zh-CN")
        .expect_err("empty cases");
    assert!(err.0.contains("cases cannot be empty"));
    drop(effects);
}
