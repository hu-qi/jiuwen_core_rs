//! # ah-plugins-workflow
//!
//! 真实工作流执行引擎:Start/End/LLM/Tool/Loop 节点 + 条件边,DAG 顺序执行。
//! LLM 节点真实调用 llm seam,Tool 节点真实调用 tools seam(经工具执行管线);
//! 每个节点执行发布 workflow/node 事件,执行轨迹可审计。

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use ah_contracts::event::Event;
use ah_contracts::keys::{LLM, TOOLS, WORKFLOW};
use ah_contracts::llm::{ChatMessage, ChatRole, ModelProvider, ModelRequest};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::tools::ToolRegistry;
use ah_contracts::workflow::{
    EdgeSpec, NodeKind, NodeSpec, WorkflowEngine, WorkflowError, WorkflowOutput, WorkflowSpec,
};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use serde_json::{Value, json};

/// 节点执行事件(emit 模式,供遥测/审计)。
#[derive(Clone, Debug)]
pub struct WorkflowNodeEvent {
    pub workflow_id: String,
    pub node_id: String,
    pub output: Value,
}

impl Event for WorkflowNodeEvent {
    const ID: &'static str = "workflow/node";
}

/// 真实工作流引擎。
pub struct WorkflowEngineImpl {
    llm: Arc<dyn ModelProvider>,
    tools: Arc<dyn ToolRegistry>,
    ctx: Context,
}

impl WorkflowEngineImpl {
    /// 构建引擎(注入 llm + tools)。
    pub fn new(llm: Arc<dyn ModelProvider>, tools: Arc<dyn ToolRegistry>, ctx: Context) -> Self {
        Self { llm, tools, ctx }
    }

    fn node<'a>(&self, spec: &'a WorkflowSpec, id: &str) -> Result<&'a NodeSpec, WorkflowError> {
        spec.nodes
            .iter()
            .find(|n| n.id == id)
            .ok_or_else(|| WorkflowError(format!("node not found: {id}")))
    }

    fn outgoing<'a>(&self, spec: &'a WorkflowSpec, id: &str) -> Vec<&'a EdgeSpec> {
        spec.edges.iter().filter(|e| e.from == id).collect()
    }

    /// 条件边求值:true 才走(关联函数,不依赖 self)。
    fn edge_allows(edge: &EdgeSpec, state: &HashMap<String, Value>) -> bool {
        let Some(condition) = &edge.condition else {
            return true;
        };
        match condition.get("type").and_then(Value::as_str) {
            Some("equals") => {
                let path = condition.get("path").and_then(Value::as_str).unwrap_or("");
                let expected = condition.get("value");
                let actual = lookup_path(state, path);
                actual == expected
            }
            Some("not_equals") => {
                let path = condition.get("path").and_then(Value::as_str).unwrap_or("");
                let expected = condition.get("value");
                let actual = lookup_path(state, path);
                actual != expected
            }
            _ => true,
        }
    }

    /// 执行 LLM 节点:config.prompt + 上一步输出 → 模型。
    async fn run_llm(&self, node: &NodeSpec, input: &Value) -> Result<Value, WorkflowError> {
        let prompt = node
            .config
            .get("prompt")
            .and_then(Value::as_str)
            .unwrap_or("continue");
        let context = input
            .get("output")
            .cloned()
            .unwrap_or_else(|| input.clone());
        let message = format!("{prompt}\nContext: {context}");
        let response = self
            .llm
            .chat(ModelRequest {
                messages: vec![ChatMessage::new(ChatRole::User, message)],
                ..Default::default()
            })
            .await
            .map_err(|e| WorkflowError(format!("llm node failed: {e}")))?;
        Ok(json!({ "content": response.content }))
    }

    /// 执行 Tool 节点:config.tool + args;上一步输出并入 args.output。
    async fn run_tool(&self, node: &NodeSpec, input: &Value) -> Result<Value, WorkflowError> {
        let name = node
            .config
            .get("tool")
            .and_then(Value::as_str)
            .ok_or_else(|| WorkflowError(format!("tool node {} missing tool", node.id)))?;
        let mut args = node
            .config
            .get("args")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if let Value::Object(map) = &mut args {
            map.insert(
                "output".to_string(),
                input.get("output").cloned().unwrap_or_default(),
            );
        }
        let output = self
            .tools
            .invoke(name, args)
            .await
            .map_err(|e| WorkflowError(format!("tool node failed: {e}")))?;
        Ok(json!({ "output": output }))
    }

    /// 执行 Loop 节点:重复执行 target 节点 N 次。
    async fn run_loop(
        &self,
        spec: &WorkflowSpec,
        node: &NodeSpec,
        input: &Value,
    ) -> Result<Value, WorkflowError> {
        let iterations = node
            .config
            .get("iterations")
            .and_then(Value::as_u64)
            .unwrap_or(1) as usize;
        let target = node
            .config
            .get("target")
            .and_then(Value::as_str)
            .ok_or_else(|| WorkflowError(format!("loop node {} missing target", node.id)))?
            .to_string();

        let mut current = input.clone();
        for i in 0..iterations {
            let target_node = self.node(spec, &target)?;
            // 循环目标只允许叶节点(Llm/Tool),避免 async 递归。
            current = match target_node.kind {
                NodeKind::Llm => self.run_llm(target_node, &current).await?,
                NodeKind::Tool => self.run_tool(target_node, &current).await?,
                _ => {
                    return Err(WorkflowError(format!(
                        "loop target must be Llm or Tool, got {:?}",
                        target_node.kind
                    )));
                }
            };
            self.ctx.emit(WorkflowNodeEvent {
                workflow_id: spec.id.clone(),
                node_id: format!("{}({})", node.id, i),
                output: current.clone(),
            });
        }
        Ok(current)
    }

    async fn execute_node(
        &self,
        spec: &WorkflowSpec,
        node: &NodeSpec,
        input: &Value,
    ) -> Result<Value, WorkflowError> {
        let output = match node.kind {
            NodeKind::Start => input.clone(),
            NodeKind::End => input.clone(),
            NodeKind::Llm => self.run_llm(node, input).await?,
            NodeKind::Tool => self.run_tool(node, input).await?,
            NodeKind::Loop => self.run_loop(spec, node, input).await?,
        };
        self.ctx.emit(WorkflowNodeEvent {
            workflow_id: spec.id.clone(),
            node_id: node.id.clone(),
            output: output.clone(),
        });
        Ok(output)
    }

    /// 拓扑排序(检测环)。
    fn topo_order(&self, spec: &WorkflowSpec) -> Result<Vec<String>, WorkflowError> {
        let mut in_degree: HashMap<String, usize> =
            spec.nodes.iter().map(|n| (n.id.clone(), 0)).collect();
        for edge in &spec.edges {
            if let Some(d) = in_degree.get_mut(&edge.to) {
                *d += 1;
            }
        }
        let mut queue: VecDeque<String> = in_degree
            .iter()
            .filter(|(_, d)| **d == 0)
            .map(|(id, _)| id.clone())
            .collect();
        let mut order = Vec::new();
        while let Some(id) = queue.pop_front() {
            order.push(id.clone());
            for edge in self.outgoing(spec, &id) {
                if let Some(d) = in_degree.get_mut(&edge.to) {
                    *d -= 1;
                    if *d == 0 {
                        queue.push_back(edge.to.clone());
                    }
                }
            }
        }
        if order.len() != spec.nodes.len() {
            return Err(WorkflowError("workflow contains a cycle".to_string()));
        }
        Ok(order)
    }
}

impl Seam for WorkflowEngineImpl {}

#[async_trait]
impl WorkflowEngine for WorkflowEngineImpl {
    async fn run(
        &self,
        spec: &WorkflowSpec,
        input: Value,
    ) -> Result<WorkflowOutput, WorkflowError> {
        // 编译期校验:拓扑序(检测环)。
        let order = self.topo_order(spec)?;
        let starts: Vec<&NodeSpec> = spec
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Start)
            .collect();
        if starts.is_empty() {
            return Err(WorkflowError("workflow missing Start node".to_string()));
        }

        let mut state: HashMap<String, Value> = HashMap::new();
        let mut executed: Vec<String> = Vec::new();

        // 从 Start 逐节点推进(按拓扑序)。
        for node_id in order {
            let node = self.node(spec, &node_id)?;
            if node.kind == NodeKind::End {
                continue;
            }
            // 上游是否满足(条件边)?
            let incoming: Vec<&EdgeSpec> = spec.edges.iter().filter(|e| e.to == node_id).collect();
            if !incoming.is_empty() && !incoming.iter().all(|e| Self::edge_allows(e, &state)) {
                continue; // 条件不满足,跳过该分支节点
            }
            let input_for_node = if node.kind == NodeKind::Start {
                input.clone()
            } else {
                // 取第一个可用上游输出作为输入。
                incoming
                    .iter()
                    .find_map(|e| state.get(&e.from).cloned())
                    .unwrap_or_else(|| json!({}))
            };
            let output = self.execute_node(spec, node, &input_for_node).await?;
            state.insert(node_id.clone(), output);
            executed.push(node_id.clone());
        }

        // End 节点输出 = 其上游输出。
        let end_id = spec
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::End)
            .ok_or_else(|| WorkflowError("workflow missing End node".to_string()))?;
        let end_input = spec
            .edges
            .iter()
            .filter(|e| e.to == end_id.id)
            .find_map(|e| state.get(&e.from).cloned())
            .unwrap_or_else(|| json!({}));

        Ok(WorkflowOutput {
            output: end_input,
            executed,
        })
    }
}
/// 从 state map 按 "a.b" 路径取值。
fn lookup_path<'a>(state: &'a HashMap<String, Value>, path: &str) -> Option<&'a Value> {
    let mut current: Option<&Value> = None;
    for part in path.split('.') {
        current = match current {
            None => state.get(part),
            Some(Value::Object(map)) => map.get(part),
            Some(_) => return None,
        };
    }
    current
}

/// 工作流插件:注入 llm + tools,提供 workflow seam。
pub struct WorkflowPlugin;

impl Plugin for WorkflowPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-workflow"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![WORKFLOW]
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
        let engine: Arc<dyn WorkflowEngine> =
            Arc::new(WorkflowEngineImpl::new(llm, tools, ctx.clone()));
        Ok(vec![ctx.register(WORKFLOW, engine)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::WORKFLOW;
    use ah_hub::plugin::DynPlugin;
    use std::sync::Arc as StdArc;

    fn build_ctx(root: &std::path::Path) -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![
            StdArc::new(ah_plugins_mock::MockPlugin),
            StdArc::new(ah_plugins_tools::ToolsPlugin),
            StdArc::new(ah_plugins_sysop::SysopPlugin::new(root)),
            StdArc::new(WorkflowPlugin),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[tokio::test]
    async fn sequential_tool_then_llm_pipeline() {
        let root = std::env::temp_dir().join(format!("ah-wf-seq-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let engine = ctx
            .service::<dyn WorkflowEngine>(&WORKFLOW)
            .expect("workflow");

        let spec = WorkflowSpec {
            id: "wf-1".to_string(),
            nodes: vec![
                NodeSpec {
                    id: "start".into(),
                    kind: NodeKind::Start,
                    config: json!({}),
                },
                NodeSpec {
                    id: "write".into(),
                    kind: NodeKind::Tool,
                    config: json!({ "tool": "write_file", "args": { "path": "computed.txt", "content": "42" } }),
                },
                NodeSpec {
                    id: "llm".into(),
                    kind: NodeKind::Llm,
                    config: json!({ "prompt": "summarize" }),
                },
                NodeSpec {
                    id: "end".into(),
                    kind: NodeKind::End,
                    config: json!({}),
                },
            ],
            edges: vec![
                EdgeSpec {
                    from: "start".into(),
                    to: "write".into(),
                    condition: None,
                },
                EdgeSpec {
                    from: "write".into(),
                    to: "llm".into(),
                    condition: None,
                },
                EdgeSpec {
                    from: "llm".into(),
                    to: "end".into(),
                    condition: None,
                },
            ],
        };

        let output = engine.run(&spec, json!({ "x": 1 })).await.expect("run");
        // write_file 真实执行(经管线):真实文件被创建。
        let fs = ctx
            .service::<dyn ah_contracts::fs::FsProvider>(&ah_contracts::keys::FS)
            .expect("fs");
        assert!(fs.exists("computed.txt"));
        assert_eq!(fs.read("computed.txt").unwrap(), b"42");
        assert_eq!(
            output.executed,
            vec!["start".to_string(), "write".to_string(), "llm".to_string()]
        );

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn conditional_branch_takes_selected_path() {
        let root = std::env::temp_dir().join(format!("ah-wf-branch-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let engine = ctx
            .service::<dyn WorkflowEngine>(&WORKFLOW)
            .expect("workflow");

        // 预写 value.txt(内容 42);read_file 读到 content==42 走 big 分支,否则 small。
        let fs = ctx
            .service::<dyn ah_contracts::fs::FsProvider>(&ah_contracts::keys::FS)
            .expect("fs");
        fs.write("value.txt", b"42").expect("write");

        let spec = WorkflowSpec {
            id: "wf-branch".to_string(),
            nodes: vec![
                NodeSpec {
                    id: "start".into(),
                    kind: NodeKind::Start,
                    config: json!({}),
                },
                NodeSpec {
                    id: "read".into(),
                    kind: NodeKind::Tool,
                    config: json!({ "tool": "read_file", "args": { "path": "value.txt" } }),
                },
                NodeSpec {
                    id: "big".into(),
                    kind: NodeKind::Tool,
                    config: json!({ "tool": "write_file", "args": { "path": "big.txt", "content": "big" } }),
                },
                NodeSpec {
                    id: "small".into(),
                    kind: NodeKind::Tool,
                    config: json!({ "tool": "write_file", "args": { "path": "small.txt", "content": "small" } }),
                },
                NodeSpec {
                    id: "end".into(),
                    kind: NodeKind::End,
                    config: json!({}),
                },
            ],
            edges: vec![
                EdgeSpec {
                    from: "start".into(),
                    to: "read".into(),
                    condition: None,
                },
                EdgeSpec {
                    from: "read".into(),
                    to: "big".into(),
                    condition: Some(
                        json!({ "type": "equals", "path": "read.output.content", "value": "42" }),
                    ),
                },
                EdgeSpec {
                    from: "read".into(),
                    to: "small".into(),
                    condition: Some(
                        json!({ "type": "not_equals", "path": "read.output.content", "value": "42" }),
                    ),
                },
                EdgeSpec {
                    from: "big".into(),
                    to: "end".into(),
                    condition: None,
                },
                EdgeSpec {
                    from: "small".into(),
                    to: "end".into(),
                    condition: None,
                },
            ],
        };

        let output = engine.run(&spec, json!({})).await.expect("run");
        assert!(output.executed.contains(&"big".to_string()));
        assert!(!output.executed.contains(&"small".to_string()));
        assert!(fs.exists("big.txt"));
        assert!(!fs.exists("small.txt"));

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn loop_repeats_target_node() {
        let root = std::env::temp_dir().join(format!("ah-wf-loop-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let engine = ctx
            .service::<dyn WorkflowEngine>(&WORKFLOW)
            .expect("workflow");

        let spec = WorkflowSpec {
            id: "wf-loop".to_string(),
            nodes: vec![
                NodeSpec {
                    id: "start".into(),
                    kind: NodeKind::Start,
                    config: json!({}),
                },
                NodeSpec {
                    id: "loop".into(),
                    kind: NodeKind::Loop,
                    config: json!({ "iterations": 3, "target": "list_node" }),
                },
                NodeSpec {
                    id: "list_node".into(),
                    kind: NodeKind::Tool,
                    config: json!({ "tool": "list_dir", "args": { "path": "." } }),
                },
                NodeSpec {
                    id: "end".into(),
                    kind: NodeKind::End,
                    config: json!({}),
                },
            ],
            edges: vec![
                EdgeSpec {
                    from: "start".into(),
                    to: "loop".into(),
                    condition: None,
                },
                EdgeSpec {
                    from: "loop".into(),
                    to: "end".into(),
                    condition: None,
                },
            ],
        };

        let events = StdArc::new(std::sync::Mutex::new(Vec::new()));
        let ev = events.clone();
        let _listener = ctx.on::<WorkflowNodeEvent>(move |event| {
            ev.lock().unwrap().push(event.node_id.clone());
        });

        let _output = engine.run(&spec, json!({})).await.expect("run");
        let node_events = events.lock().unwrap().clone();
        let loop_marks: Vec<_> = node_events
            .iter()
            .filter(|id| id.starts_with("loop("))
            .collect();
        assert_eq!(loop_marks.len(), 3);

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn cycle_is_rejected() {
        let root = std::env::temp_dir().join(format!("ah-wf-cycle-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let engine = ctx
            .service::<dyn WorkflowEngine>(&WORKFLOW)
            .expect("workflow");

        let spec = WorkflowSpec {
            id: "wf-cycle".to_string(),
            nodes: vec![
                NodeSpec {
                    id: "a".into(),
                    kind: NodeKind::Start,
                    config: json!({}),
                },
                NodeSpec {
                    id: "b".into(),
                    kind: NodeKind::Tool,
                    config: json!({ "tool": "echo", "args": {} }),
                },
            ],
            edges: vec![
                EdgeSpec {
                    from: "a".into(),
                    to: "b".into(),
                    condition: None,
                },
                EdgeSpec {
                    from: "b".into(),
                    to: "a".into(),
                    condition: None,
                },
            ],
        };

        let error = engine.run(&spec, json!({})).await.expect_err("cycle");
        assert!(error.0.contains("cycle"));

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
}
