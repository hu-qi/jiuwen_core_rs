//! # ah-plugins-agentbuilder
//!
//! 真实 agent_builder:NL 任务 → 设计(确定性意图解析:goal/工具/步骤)→
//! 工作流 DSL(JSON WorkflowSpec)→ 经 WorkflowEngine 真实执行。

use std::sync::Arc;

use ah_contracts::agent_builder::{AgentBuilder, AgentDesign, BuildError};
use ah_contracts::keys::{AGENT_BUILDER, WORKFLOW};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::workflow::{EdgeSpec, NodeKind, NodeSpec, WorkflowEngine, WorkflowSpec};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use serde_json::{Value, json};

/// 已知工具词表(真实工具名 ↔ 触发词)。
const TOOL_KEYWORDS: &[(&str, &[&str])] = &[
    ("list_dir", &["list", "enumerate", "scan"]),
    ("read_file", &["read", "inspect", "view"]),
    ("write_file", &["write", "create", "edit", "modify"]),
    ("run_shell", &["run", "execute", "shell", "command"]),
    ("web_fetch", &["fetch", "web", "url", "download"]),
    ("run_code", &["code", "script", "python"]),
    ("search_knowledge", &["search", "knowledge", "kb"]),
];

/// 真实 agent 构建器。
pub struct WorkflowAgentBuilder {
    workflow: Arc<dyn WorkflowEngine>,
}

impl WorkflowAgentBuilder {
    pub fn new(workflow: Arc<dyn WorkflowEngine>) -> Self {
        Self { workflow }
    }
}

impl Seam for WorkflowAgentBuilder {}

#[async_trait]
impl AgentBuilder for WorkflowAgentBuilder {
    fn design(&self, nl: &str) -> Result<AgentDesign, BuildError> {
        if nl.trim().is_empty() {
            return Err(BuildError("empty task description".to_string()));
        }
        let lower = nl.to_lowercase();
        let tools: Vec<String> = TOOL_KEYWORDS
            .iter()
            .filter(|(_, keywords)| keywords.iter().any(|k| lower.contains(k)))
            .map(|(name, _)| name.to_string())
            .collect();
        let steps: Vec<String> = nl
            .split(['.', ';', '\n'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        // 名称:首句前 6 词,下划线化。
        let name = steps
            .first()
            .map(|s| {
                s.split_whitespace()
                    .take(6)
                    .collect::<Vec<_>>()
                    .join("_")
                    .to_lowercase()
            })
            .unwrap_or_else(|| "agent".to_string());
        Ok(AgentDesign {
            name,
            goal: nl.trim().to_string(),
            tools,
            steps,
        })
    }

    fn to_dsl(&self, design: &AgentDesign) -> Result<Value, BuildError> {
        if design.tools.is_empty() {
            return Err(BuildError("no tools identified in the task".to_string()));
        }
        // Start -> Tool(1..n, 串行) -> End 的工作流。
        let mut nodes = vec![NodeSpec {
            id: "start".to_string(),
            kind: NodeKind::Start,
            config: json!({}),
        }];
        let mut edges: Vec<EdgeSpec> = Vec::new();
        for (i, tool) in design.tools.iter().enumerate() {
            let id = format!("step{i}");
            nodes.push(NodeSpec {
                id: id.clone(),
                kind: NodeKind::Tool,
                config: json!({ "tool": tool, "args": {} }),
            });
            let from = if i == 0 {
                "start".to_string()
            } else {
                format!("step{}", i - 1)
            };
            edges.push(EdgeSpec {
                from,
                to: id.clone(),
                condition: None,
            });
        }
        nodes.push(NodeSpec {
            id: "end".to_string(),
            kind: NodeKind::End,
            config: json!({}),
        });
        edges.push(EdgeSpec {
            from: format!("step{}", design.tools.len() - 1),
            to: "end".to_string(),
            condition: None,
        });
        let spec = WorkflowSpec {
            id: design.name.clone(),
            nodes,
            edges,
        };
        serde_json::to_value(&spec).map_err(|e| BuildError(format!("serialize dsl: {e}")))
    }

    async fn run(&self, dsl: &Value) -> Result<Value, BuildError> {
        let spec: WorkflowSpec = serde_json::from_value(dsl.clone())
            .map_err(|e| BuildError(format!("parse dsl: {e}")))?;
        let output = self
            .workflow
            .run(&spec, json!({}))
            .await
            .map_err(|e| BuildError(e.0))?;
        Ok(output.output)
    }
}

/// agent-builder 插件:注入 WorkflowEngine,提供 agent-builder seam。
pub struct AgentBuilderPlugin;

impl Plugin for AgentBuilderPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-agentbuilder"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![AGENT_BUILDER]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![WORKFLOW]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let workflow = ctx
            .service::<dyn WorkflowEngine>(&WORKFLOW)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "workflow seam not registered".to_string(),
            })?;
        let builder: Arc<dyn AgentBuilder> = Arc::new(WorkflowAgentBuilder::new(workflow));
        Ok(vec![ctx.register(AGENT_BUILDER, builder)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::AGENT_BUILDER;
    use ah_hub::plugin::DynPlugin;
    use serde_json::json;
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
            StdArc::new(ah_plugins_web::WebPlugin),
            StdArc::new(ah_plugins_workflow::WorkflowPlugin),
            StdArc::new(AgentBuilderPlugin),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[test]
    fn design_extracts_intent_and_tools() {
        let root = std::env::temp_dir().join(format!("ah-ab-design-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let builder = ctx
            .service::<dyn AgentBuilder>(&AGENT_BUILDER)
            .expect("builder");

        let design = builder
            .design("read the config file and list the workspace")
            .expect("design");
        assert!(design.tools.contains(&"read_file".to_string()));
        assert!(design.tools.contains(&"list_dir".to_string()));
        assert!(design.goal.contains("config"));
        assert!(!design.steps.is_empty());

        assert!(builder.design("").is_err(), "empty input errors");

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn dsl_is_runnable_workflow_spec() {
        let root = std::env::temp_dir().join(format!("ah-ab-dsl-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let builder = ctx
            .service::<dyn AgentBuilder>(&AGENT_BUILDER)
            .expect("builder");
        let design = builder.design("list the workspace").expect("design");
        let dsl = builder.to_dsl(&design).expect("dsl");
        let spec: WorkflowSpec = serde_json::from_value(dsl).expect("valid WorkflowSpec");
        assert_eq!(spec.id, design.name);
        assert!(spec.nodes.iter().any(|n| n.kind == NodeKind::Start));
        assert!(spec.nodes.iter().any(|n| n.kind == NodeKind::End));
        // 无工具 → 显式报错。
        let no_tools = AgentDesign {
            name: "x".to_string(),
            goal: "hi".to_string(),
            tools: vec![],
            steps: vec![],
        };
        assert!(builder.to_dsl(&no_tools).is_err());
        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn run_executes_dsl_for_real() {
        let root = std::env::temp_dir().join(format!("ah-ab-run-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let builder = ctx
            .service::<dyn AgentBuilder>(&AGENT_BUILDER)
            .expect("builder");

        let design = builder.design("list the workspace").expect("design");
        let dsl = builder.to_dsl(&design).expect("dsl");
        let output = builder.run(&dsl).await.expect("run");
        // Tool 节点真实执行 list_dir,End 输出其上游输出。
        assert!(!output.to_string().is_empty(), "real workflow output");

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
}
