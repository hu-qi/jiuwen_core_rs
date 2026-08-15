//! # ah-plugins-pregel
//!
//! 真实 Pregel 风格超级步图引擎:图节点共享状态(状态通道),每个超级步执行
//! 所有入边条件满足的节点,输出写入 state[node.id];无节点触发 / 达上限 /
//! 命中 halt 配置即停并返回(部分)状态。

use std::collections::HashMap;
use std::sync::Arc;

use ah_contracts::keys::{LLM, PREGEL, TOOLS};
use ah_contracts::llm::{ChatMessage, ChatRole, ModelProvider, ModelRequest};
use ah_contracts::pregel::{
    PregelEdge, PregelEngine, PregelError, PregelGraph, PregelNodeKind, PregelNodeSpec,
    PregelResult,
};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::tools::ToolRegistry;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use serde_json::{Value, json};

/// 条件判定(与 workflow 一致:always / equals / not_equals)。
fn edge_allows(edge: &PregelEdge, state: &Value) -> bool {
    let Some(condition) = &edge.condition else {
        return true;
    };
    match condition
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("always")
    {
        "always" => true,
        "equals" => {
            let path = condition
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let expected = condition.get("value").cloned().unwrap_or(Value::Null);
            state
                .pointer(&format!("/{path}"))
                .map(|v| v == &expected)
                .unwrap_or(false)
        }
        "not_equals" => {
            let path = condition
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let expected = condition.get("value").cloned().unwrap_or(Value::Null);
            state
                .pointer(&format!("/{path}"))
                .map(|v| v != &expected)
                .unwrap_or(true)
        }
        // 状态通道缺失/存在(用于 fire-once 与动态路由)。
        "missing" => {
            let path = condition
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default();
            state.pointer(&format!("/{path}")).is_none()
        }
        "present" => {
            let path = condition
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default();
            state.pointer(&format!("/{path}")).is_some()
        }
        _ => true,
    }
}

/// 真实超级步图引擎。
pub struct PregelEngineImpl {
    llm: Arc<dyn ModelProvider>,
    tools: Arc<dyn ToolRegistry>,
}

impl PregelEngineImpl {
    pub fn new(llm: Arc<dyn ModelProvider>, tools: Arc<dyn ToolRegistry>) -> Self {
        Self { llm, tools }
    }

    async fn execute_node(
        &self,
        node: &PregelNodeSpec,
        state: &Value,
    ) -> Result<Value, PregelError> {
        match node.kind {
            PregelNodeKind::Tool => {
                let name = node
                    .config
                    .get("tool")
                    .and_then(Value::as_str)
                    .ok_or_else(|| PregelError(format!("tool node {} missing tool", node.id)))?;
                let mut args = node
                    .config
                    .get("args")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                if let Value::Object(map) = &mut args {
                    map.insert("state".to_string(), state.clone());
                }
                let output = self
                    .tools
                    .invoke(name, args)
                    .await
                    .map_err(|e| PregelError(format!("tool node failed: {e}")))?;
                Ok(json!({ "output": output }))
            }
            PregelNodeKind::Llm => {
                let prompt = node
                    .config
                    .get("prompt")
                    .and_then(Value::as_str)
                    .unwrap_or("continue");
                let response = self
                    .llm
                    .chat(ModelRequest {
                        messages: vec![ChatMessage::new(
                            ChatRole::User,
                            format!("{prompt}\nState: {state}"),
                        )],
                        ..Default::default()
                    })
                    .await
                    .map_err(|e| PregelError(format!("llm node failed: {e}")))?;
                Ok(json!({ "content": response.content }))
            }
            PregelNodeKind::Noop => Ok(json!({ "noop": true })),
        }
    }
}

impl Seam for PregelEngineImpl {}

#[async_trait]
impl PregelEngine for PregelEngineImpl {
    async fn run(
        &self,
        graph: &PregelGraph,
        initial_state: Value,
    ) -> Result<PregelResult, PregelError> {
        let mut state: Value = initial_state;
        let mut interrupted_at: Option<String> = None;
        let mut fired_any = true;
        let mut supersteps = 0u32;
        // 每节点输出通道缓存(本超级步内共享)。
        while fired_any && supersteps < graph.max_supersteps {
            supersteps += 1;
            fired_any = false;
            let mut updates: HashMap<String, Value> = HashMap::new();
            for node in &graph.nodes {
                // 入边条件(对当前状态)。
                let incoming: Vec<&PregelEdge> =
                    graph.edges.iter().filter(|e| e.to == node.id).collect();
                if !incoming.is_empty() && !incoming.iter().all(|e| edge_allows(e, &state)) {
                    continue;
                }
                let output = self.execute_node(node, &state).await?;
                updates.insert(node.id.clone(), output.clone());
                fired_any = true;
                // 中断点:写入后返回部分状态。
                if node
                    .config
                    .get("halt")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    interrupted_at = Some(node.id.clone());
                    break;
                }
            }
            if let Some(interrupted) = interrupted_at.clone() {
                for (k, v) in updates {
                    if let Value::Object(map) = &mut state {
                        map.insert(k, v);
                    }
                }
                return Ok(PregelResult {
                    state,
                    supersteps,
                    interrupted_at: Some(interrupted),
                });
            }
            for (k, v) in updates {
                if let Value::Object(map) = &mut state {
                    map.insert(k, v);
                }
            }
        }
        Ok(PregelResult {
            state,
            supersteps,
            interrupted_at,
        })
    }
}

/// pregel 插件:注入 llm + tools,提供 pregel seam。
pub struct PregelPlugin;

impl Plugin for PregelPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-pregel"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![PREGEL]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![LLM, TOOLS]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let llm = ctx
            .service::<dyn ModelProvider>(&LLM)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "llm seam not registered".to_string(),
            })?;
        let tools = ctx
            .service::<dyn ToolRegistry>(&TOOLS)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "tools seam not registered".to_string(),
            })?;
        let engine: Arc<dyn PregelEngine> = Arc::new(PregelEngineImpl::new(llm, tools));
        Ok(vec![ctx.register(PREGEL, engine)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::PREGEL;
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
            StdArc::new(PregelPlugin),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[tokio::test]
    async fn supersteps_fire_until_no_trigger() {
        let root = std::env::temp_dir().join(format!("ah-pregel-run-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let engine = ctx.service::<dyn PregelEngine>(&PREGEL).expect("pregel");
        let graph = PregelGraph {
            id: "g1".to_string(),
            nodes: vec![
                PregelNodeSpec {
                    id: "scan".to_string(),
                    kind: PregelNodeKind::Tool,
                    config: json!({ "tool": "list_dir", "args": { "path": "." } }),
                },
                PregelNodeSpec {
                    id: "done".to_string(),
                    kind: PregelNodeKind::Noop,
                    config: json!({}),
                },
            ],
            edges: vec![
                // fire-once 语义:scan 仅在通道缺失时触发;done 在 scan 出现后触发一次。
                PregelEdge {
                    from: "scan".to_string(),
                    to: "scan".to_string(),
                    condition: Some(json!({ "type": "missing", "path": "scan" })),
                },
                PregelEdge {
                    from: "scan".to_string(),
                    to: "done".to_string(),
                    condition: Some(json!({ "type": "present", "path": "scan" })),
                },
                PregelEdge {
                    from: "done".to_string(),
                    to: "done".to_string(),
                    condition: Some(json!({ "type": "missing", "path": "done" })),
                },
            ],
            max_supersteps: 5,
        };
        let result = engine.run(&graph, json!({})).await.expect("run");
        assert!(
            result.state.get("scan").is_some(),
            "tool output in state channel"
        );
        assert!(
            result.state.get("done").is_some(),
            "noop output in state channel"
        );
        assert!(result.supersteps >= 1);
        assert!(result.interrupted_at.is_none(), "no halt configured");

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn halt_interrupts_with_partial_state() {
        let root = std::env::temp_dir().join(format!("ah-pregel-halt-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let engine = ctx.service::<dyn PregelEngine>(&PREGEL).expect("pregel");
        let graph = PregelGraph {
            id: "g2".to_string(),
            nodes: vec![
                PregelNodeSpec {
                    id: "scan".to_string(),
                    kind: PregelNodeKind::Tool,
                    config: json!({ "tool": "list_dir", "args": { "path": "." } }),
                },
                PregelNodeSpec {
                    id: "checkpoint".to_string(),
                    kind: PregelNodeKind::Noop,
                    config: json!({ "halt": true }),
                },
            ],
            edges: vec![PregelEdge {
                from: "scan".to_string(),
                to: "checkpoint".to_string(),
                condition: None,
            }],
            max_supersteps: 5,
        };
        let result = engine.run(&graph, json!({})).await.expect("run");
        assert_eq!(result.interrupted_at.as_deref(), Some("checkpoint"));
        assert!(
            result.state.get("scan").is_some(),
            "partial state preserved"
        );

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn max_supersteps_caps_runaway_graph() {
        let root = std::env::temp_dir().join(format!("ah-pregel-cap-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let engine = ctx.service::<dyn PregelEngine>(&PREGEL).expect("pregel");
        // 自触发环:always 边让自己每步都触发。
        let graph = PregelGraph {
            id: "g3".to_string(),
            nodes: vec![PregelNodeSpec {
                id: "loop".to_string(),
                kind: PregelNodeKind::Noop,
                config: json!({}),
            }],
            edges: vec![PregelEdge {
                from: "loop".to_string(),
                to: "loop".to_string(),
                condition: None,
            }],
            max_supersteps: 3,
        };
        let result = engine.run(&graph, json!({})).await.expect("run");
        assert_eq!(result.supersteps, 3, "capped by max_supersteps");

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
}
