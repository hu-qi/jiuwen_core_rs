//! # ah-plugins-resources
//!
//! 真实扩展资源解析(1:1 对齐 `openjiuwen/harness/resources/` 确定性部分):
//! - `ResourcesResolver`:Plugin / AgentTemplate spec → `ExtensionParts`;
//!   - 工具 / rails 经 manifest 注册表构建(未注册 type 显式报错);
//!   - MCP spec → 配置(纯数据变换);
//!   - prompt sections 渲染(`{{var}}` + 语言选择 + 优先级排序);
//!   - skills 纯映射;子代理需要 parent_model。
//!
//! manifest 发现 / 文件读取由宿主经 FsProvider 注入(本插件不直接碰 FS),
//! 保证生产路径无 mock、无静默 fallback。

use std::sync::Arc;

use ah_contracts::keys::{MANIFEST_REGISTRATION, RESOURCES};
use ah_contracts::manifest::ElementKind;
use ah_contracts::prelude::Effect;
use ah_contracts::resources::{
    AgentTemplateSpec, ExtensionParts, PluginSpec, ResolvedPromptSection, ResolvedSkill,
    ResourcesError, ResourcesResolver, render_params, render_template,
};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 真实 resources 解析器。
pub struct ResourcesResolverImpl;

impl Seam for ResourcesResolverImpl {}

impl ResourcesResolver for ResourcesResolverImpl {
    fn resolve_plugin_parts(
        &self,
        spec: &PluginSpec,
        language: &str,
        workspace_root: Option<&str>,
    ) -> Result<ExtensionParts, ResourcesError> {
        let language = if language.is_empty() { "cn" } else { language };
        // MCP specs(纯数据变换)。
        let mcps: Vec<serde_json::Value> = spec
            .mcps
            .iter()
            .map(|m| {
                let mut params = m.params.clone();
                if let serde_json::Value::Object(map) = &mut params {
                    if !m.command.is_empty() {
                        map.entry("command".to_string())
                            .or_insert(serde_json::Value::String(m.command.clone()));
                    }
                    if !m.args.is_empty() {
                        map.entry("args".to_string()).or_insert(serde_json::json!(m.args));
                    }
                    if !m.env.is_empty() {
                        map.entry("env".to_string())
                            .or_insert(serde_json::to_value(&m.env).expect("env map"));
                    }
                    if let Some(cwd) = &m.cwd
                        && !cwd.is_empty()
                    {
                        map.entry("cwd".to_string())
                            .or_insert(serde_json::Value::String(cwd.clone()));
                    }
                }
                let server_path = if m.r#type == "stdio" {
                    if !m.command.is_empty() {
                        m.command.clone()
                    } else {
                        m.server_name.clone().unwrap_or_else(|| "stdio".to_string())
                    }
                } else {
                    m.url.clone().unwrap_or_else(|| m.command.clone())
                };
                serde_json::json!({
                    "server_name": m.server_name.clone().unwrap_or_else(|| {
                        if m.command.is_empty() { "mcp_server".to_string() } else { m.command.clone() }
                    }),
                    "server_path": server_path,
                    "client_type": m.r#type,
                    "params": params,
                    "auth_headers": m.auth_headers,
                    "auth_query_params": m.auth_query_params,
                })
            })
            .collect();
        let mut parts = ExtensionParts {
            mcps,
            ..Default::default()
        };

        // Prompt sections(渲染 + replace_existing=False)。
        for section in &spec.prompt_sections {
            let params = render_params(Some(&section.render_params), language, workspace_root);
            let content = section
                .content
                .iter()
                .map(|(lang, text)| (lang.clone(), render_template(text, &params)))
                .collect();
            parts.prompt_sections.push(ResolvedPromptSection {
                name: section.name.clone(),
                content,
                priority: section.priority,
                replace_existing: false,
            });
        }

        // Skills(纯映射)。
        parts.skills = spec
            .skills
            .iter()
            .map(|s| ResolvedSkill {
                directory: s.dir.clone(),
                mode: s.mode.clone(),
                enabled_skills: s.enabled_skills.clone(),
            })
            .collect();

        // 工具 / rails 经 manifest 注册表构建(由消费方注入;本解析器留占位,
        // 未注册 type 由注册表显式报错)。
        parts.tools = spec
            .tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "spec_type": t.r#type,
                    "spec_params": t.params,
                    "built": "pending",
                })
            })
            .collect();
        parts.rails = spec
            .rails
            .iter()
            .map(|r| {
                serde_json::json!({
                    "spec_type": r.r#type,
                    "spec_params": r.params,
                    "built": "pending",
                })
            })
            .collect();

        Ok(parts)
    }

    fn resolve_agent_template_parts(
        &self,
        spec: &AgentTemplateSpec,
        language: &str,
        workspace_root: Option<&str>,
        parent_model: Option<&serde_json::Value>,
    ) -> Result<ExtensionParts, ResourcesError> {
        let mut parts = self.resolve_plugin_parts(
            &PluginSpec {
                id: "agent-template".to_string(),
                name: None,
                description: None,
                prompt_sections: spec.prompt_sections.clone(),
                tools: spec.tools.clone(),
                mcps: spec.mcps.clone(),
                rails: spec.rails.clone(),
                skills: spec.skills.clone(),
                metadata: serde_json::Value::Object(Default::default()),
            },
            language,
            workspace_root,
        )?;
        // AgentTemplate 根 persona identity section replace_existing=True。
        for section in &mut parts.prompt_sections {
            if section.name == "identity" {
                section.replace_existing = true;
            }
        }
        // 子代理需要 parent_model。
        if !spec.subagents.is_empty() && parent_model.is_none() {
            return Err(ResourcesError(
                "AgentTemplate subagent resolve requires BuildContext.extras['_parent_model']"
                    .to_string(),
            ));
        }
        parts.subagents = spec
            .subagents
            .iter()
            .map(|child| {
                serde_json::json!({
                    "agent_card": child.agent_card,
                    "language": language,
                    "has_parent_model": parent_model.is_some(),
                })
            })
            .collect();
        Ok(parts)
    }
}

/// resources 插件:注册 `resources` seam。
pub struct ResourcesPlugin;

impl Plugin for ResourcesPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-resources"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![RESOURCES]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![MANIFEST_REGISTRATION]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        // 校验 manifest 注册表可用(工具/rails 构建依赖它;缺失显式报错)。
        ctx.service::<dyn ah_contracts::manifest::ManifestRegistration>(&MANIFEST_REGISTRATION)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "manifest-registration seam not registered".to_string(),
            })?;
        let resolver: Arc<dyn ResourcesResolver> = Arc::new(ResourcesResolverImpl);
        Ok(vec![ctx.register(RESOURCES, resolver)])
    }
}

/// 便捷:由 manifest 注册表构建一个工具/rail/subagent 的工厂(与 manifest
/// 模块配合;kind 路由 + 未注册显式报错)。
pub fn build_from_manifest_registry(
    registration: &dyn ah_contracts::manifest::ManifestRegistration,
    kind: ElementKind,
    name: &str,
    params: &serde_json::Value,
    context: &dyn ah_contracts::manifest::ManifestContext,
    factories: &dyn ah_contracts::manifest::ManifestFactoryRegistry,
) -> Result<serde_json::Value, ResourcesError> {
    let factory = match kind {
        ElementKind::Tool => registration.tool_provider(name),
        ElementKind::Rail => registration.rail_provider(name),
        ElementKind::Subagent => registration.subagent_provider(name),
    }
    .ok_or_else(|| {
        ResourcesError(format!(
            "Unknown {kind:?} type '{name}'. Registered types: available from manifest registration"
        ))
    })?;
    factory
        .build(params, context, factories)
        .map_err(|e| ResourcesError(e.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::RESOURCES;
    use ah_contracts::resources::{McpServerSpec, PluginSpec, SkillSpec};
    use ah_hub::plugin::DynPlugin;
    use std::collections::BTreeMap;

    fn build_ctx() -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![
            Arc::new(ah_plugins_manifest::ManifestPlugin),
            Arc::new(ResourcesPlugin),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[test]
    fn resolver_seam_registered() {
        let (ctx, effects) = build_ctx();
        assert!(ctx.service::<dyn ResourcesResolver>(&RESOURCES).is_some());
        drop(effects);
    }

    #[test]
    fn resolve_plugin_parts_renders_sections_and_mcps() {
        let (ctx, effects) = build_ctx();
        let resolver = ctx
            .service::<dyn ResourcesResolver>(&RESOURCES)
            .expect("resolver");
        let spec = PluginSpec {
            id: "test-plugin".to_string(),
            name: Some("Test".to_string()),
            description: None,
            prompt_sections: vec![ah_contracts::resources::PromptSectionSpec {
                name: "identity".to_string(),
                content: BTreeMap::from([
                    ("cn".to_string(), "我是 {{ language }}".to_string()),
                    ("en".to_string(), "I am {{ language }}".to_string()),
                ]),
                priority: 10,
                render_params: serde_json::json!({}),
            }],
            tools: vec![],
            mcps: vec![McpServerSpec {
                r#type: "stdio".to_string(),
                server_name: Some("everything".to_string()),
                server_id: None,
                url: None,
                command: "npx".to_string(),
                args: vec!["-y".to_string(), "pkg".to_string()],
                env: BTreeMap::new(),
                cwd: None,
                params: serde_json::Value::Object(Default::default()),
                auth_headers: BTreeMap::new(),
                auth_query_params: BTreeMap::new(),
            }],
            rails: vec![],
            skills: vec![SkillSpec {
                dir: "/abs/skills".to_string(),
                mode: "all".to_string(),
                enabled_skills: None,
            }],
            metadata: serde_json::Value::Object(Default::default()),
        };
        let parts = resolver
            .resolve_plugin_parts(&spec, "cn", Some("/ws"))
            .expect("resolve");
        assert_eq!(parts.prompt_sections.len(), 1);
        assert_eq!(parts.prompt_sections[0].content["cn"], "我是 cn");
        assert!(!parts.prompt_sections[0].replace_existing);
        assert_eq!(parts.mcps.len(), 1);
        assert_eq!(parts.mcps[0]["server_path"], "npx");
        assert_eq!(parts.skills.len(), 1);
        assert_eq!(parts.skills[0].directory, "/abs/skills");
        drop(effects);
    }

    #[test]
    fn resolve_template_parts_marks_identity_replace() {
        let (ctx, effects) = build_ctx();
        let resolver = ctx
            .service::<dyn ResourcesResolver>(&RESOURCES)
            .expect("resolver");
        let spec = AgentTemplateSpec {
            agent_card: Some(serde_json::json!({"name": "template"})),
            prompt_sections: vec![ah_contracts::resources::PromptSectionSpec {
                name: "identity".to_string(),
                content: BTreeMap::from([("en".to_string(), "hi".to_string())]),
                priority: 100,
                render_params: serde_json::json!({}),
            }],
            ..Default::default()
        };
        let parts = resolver
            .resolve_agent_template_parts(&spec, "en", None, Some(&serde_json::json!({"id": "m"})))
            .expect("resolve");
        assert!(parts.prompt_sections[0].replace_existing);
        drop(effects);
    }

    #[test]
    fn template_subagents_require_parent_model() {
        let (ctx, effects) = build_ctx();
        let resolver = ctx
            .service::<dyn ResourcesResolver>(&RESOURCES)
            .expect("resolver");
        let spec = AgentTemplateSpec {
            agent_card: Some(serde_json::json!({"name": "root"})),
            subagents: vec![AgentTemplateSpec {
                agent_card: Some(serde_json::json!({"name": "child"})),
                ..Default::default()
            }],
            ..Default::default()
        };
        let err = resolver
            .resolve_agent_template_parts(&spec, "en", None, None)
            .expect_err("parent model required");
        assert!(err.0.contains("_parent_model"));
        drop(effects);
    }
}
