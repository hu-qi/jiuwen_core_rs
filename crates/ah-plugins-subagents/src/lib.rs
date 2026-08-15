//! # ah-plugins-subagents
//!
//! 类型化子代理(code/research/plan/verify):每种类型注入专属 system 提示,
//! 并经 SubagentSpec.allowed_tools 真实过滤工具白名单(白名单外调用被拒)。

use std::sync::Arc;

use ah_contracts::keys::{SUBAGENT, SUBAGENTS};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::subagent::{SubagentRuntime, SubagentSpec};
use ah_contracts::subagents::{SubagentKind, SubagentProfile, TypedSubagentError, TypedSubagents};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;

fn profile_for(kind: SubagentKind) -> SubagentProfile {
    match kind {
        SubagentKind::Code => SubagentProfile {
            kind,
            system_prompt: "You are a coding subagent. Inspect the workspace, make precise changes, and report exactly what you changed.".to_string(),
            allowed_tools: vec![
                "list_dir".to_string(),
                "read_file".to_string(),
                "write_file".to_string(),
                "run_code".to_string(),
            ],
        },
        SubagentKind::Research => SubagentProfile {
            kind,
            system_prompt: "You are a research subagent. Gather evidence from the workspace and the knowledge base; cite sources in your answer.".to_string(),
            allowed_tools: vec![
                "list_dir".to_string(),
                "read_file".to_string(),
                "search_knowledge".to_string(),
                "web_fetch".to_string(),
            ],
        },
        SubagentKind::Plan => SubagentProfile {
            kind,
            system_prompt: "You are a planning subagent. Produce a step-by-step plan with concrete, verifiable milestones; do not execute changes.".to_string(),
            allowed_tools: vec!["list_dir".to_string(), "read_file".to_string()],
        },
        SubagentKind::Verify => SubagentProfile {
            kind,
            system_prompt: "You are a verification subagent. Check the workspace against the task requirements and report pass/fail with evidence.".to_string(),
            allowed_tools: vec![
                "list_dir".to_string(),
                "read_file".to_string(),
                "run_code".to_string(),
            ],
        },
    }
}

/// 类型化子代理运行时(包装 SubagentRuntime)。
pub struct TypedSubagentsImpl {
    subagent: Arc<dyn SubagentRuntime>,
}

impl TypedSubagentsImpl {
    pub fn new(subagent: Arc<dyn SubagentRuntime>) -> Self {
        Self { subagent }
    }
}

impl Seam for TypedSubagentsImpl {}

#[async_trait]
impl TypedSubagents for TypedSubagentsImpl {
    fn kinds(&self) -> Vec<SubagentKind> {
        vec![
            SubagentKind::Code,
            SubagentKind::Research,
            SubagentKind::Plan,
            SubagentKind::Verify,
        ]
    }

    fn profile(&self, kind: SubagentKind) -> Option<SubagentProfile> {
        Some(profile_for(kind))
    }

    async fn run(
        &self,
        kind: SubagentKind,
        task: &str,
        budget: Option<usize>,
    ) -> Result<ah_contracts::subagent::SubagentResult, TypedSubagentError> {
        let profile = profile_for(kind);
        self.subagent
            .run(SubagentSpec {
                id: format!("typed-{:?}-{}", kind, task.len()),
                task: task.to_string(),
                context: Some(profile.system_prompt),
                budget,
                allowed_tools: Some(profile.allowed_tools),
            })
            .await
            .map_err(|e| TypedSubagentError(e.0))
    }
}

/// subagents 插件:注入 SubagentRuntime,提供类型化子代理。
pub struct SubagentsPlugin;

impl Plugin for SubagentsPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-subagents"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![SUBAGENTS]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![SUBAGENT]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let subagent = ctx
            .service::<dyn SubagentRuntime>(&SUBAGENT)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "subagent seam not registered".to_string(),
            })?;
        let typed: Arc<dyn TypedSubagents> = Arc::new(TypedSubagentsImpl::new(subagent));
        Ok(vec![ctx.register(SUBAGENTS, typed)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::{SESSION_MANAGER, SUBAGENTS};
    use ah_contracts::session::SessionManager;
    use ah_hub::plugin::DynPlugin;
    use std::sync::Arc as StdArc;

    fn build_ctx(root: &std::path::Path) -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let session_dir = root.join("sessions");
        let default_path = session_dir.join("default.jsonl");
        let plugins: Vec<DynPlugin> = vec![
            StdArc::new(ah_plugins_mock::MockPlugin),
            StdArc::new(ah_plugins_tools::ToolsPlugin),
            StdArc::new(ah_plugins_sysop::SysopPlugin::new(root)),
            StdArc::new(ah_plugins_session_log::SessionLogPlugin::new(
                &default_path,
                &session_dir,
            )),
            StdArc::new(ah_plugins_subagent::SubagentPlugin),
            StdArc::new(SubagentsPlugin),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[tokio::test]
    async fn typed_subagent_injects_kind_prompt_and_runs() {
        let root = std::env::temp_dir().join(format!("ah-ts-run-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let typed = ctx
            .service::<dyn TypedSubagents>(&SUBAGENTS)
            .expect("subagents");

        let profile = typed.profile(SubagentKind::Code).expect("profile");
        assert!(profile.system_prompt.contains("coding subagent"));
        assert!(profile.allowed_tools.contains(&"write_file".to_string()));

        let result = typed
            .run(SubagentKind::Code, "refactor the workspace", Some(6))
            .await
            .expect("run");
        assert!(result.answer.contains("mock final answer"));

        // 类型提示真实注入:子代理会话含 system 消息。
        let manager = ctx
            .service::<dyn SessionManager>(&SESSION_MANAGER)
            .expect("manager");
        let session = manager
            .open(&format!("typed-Code-{}", "refactor the workspace".len()))
            .expect("open");
        let messages = session.derive_messages();
        assert!(
            messages.iter().any(|m| {
                m.role == ah_contracts::llm::ChatRole::System
                    && m.content.contains("coding subagent")
            }),
            "kind system prompt injected into the subagent session"
        );

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn allowlist_blocks_disallowed_tool_calls() {
        let root = std::env::temp_dir().join(format!("ah-ts-allow-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let typed = ctx
            .service::<dyn TypedSubagents>(&SUBAGENTS)
            .expect("subagents");
        // Plan 白名单只含 list_dir/read_file;mock 模型会请求 list_dir → 放行。
        let result = typed
            .run(SubagentKind::Plan, "plan the next milestone", Some(4))
            .await
            .expect("run");
        assert!(result.answer.contains("mock final answer"));

        // 直接构造一个不允许任何工具的 spec:越权调用必须被拒(日志可见 "tool not allowed")。
        let subagent = ctx
            .service::<dyn SubagentRuntime>(&ah_contracts::keys::SUBAGENT)
            .expect("subagent");
        let res = subagent
            .run(SubagentSpec {
                id: format!("no-tools-{}", std::process::id()),
                task: "list the workspace".to_string(),
                context: None,
                budget: Some(3),
                allowed_tools: Some(vec![]),
            })
            .await
            .expect("run");
        assert!(res.answer.contains("mock final answer"));
        // mock 请求的 list_dir 未被白名单放行 → 会话日志含 "tool not allowed"。
        let manager = ctx
            .service::<dyn SessionManager>(&SESSION_MANAGER)
            .expect("manager");
        let session_id = format!("no-tools-{}", std::process::id());
        let session = manager.open(&session_id).expect("open");
        let events = session.events();
        assert!(
            events.iter().any(|e| {
                e.payload
                    .get("output")
                    .and_then(serde_json::Value::as_str)
                    .map(|s| s.contains("tool not allowed"))
                    .unwrap_or(false)
            }),
            "disallowed tool call recorded as rejected"
        );

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
}
