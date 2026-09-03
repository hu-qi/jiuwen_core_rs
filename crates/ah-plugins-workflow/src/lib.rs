//! # ah-plugins-workflow
//!
//! 真实工作流执行引擎:Start/End/LLM/Tool/Loop 节点 + 条件边,DAG 顺序执行。
//! LLM 节点真实调用 llm seam,Tool 节点真实调用 tools seam(经工具执行管线);
//! 每个节点执行发布 workflow/node 事件,执行轨迹可审计。

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use ah_contracts::event::Event;
use ah_contracts::keys::{LLM, QUEUE, SESSIONS, TOOLS, WEB, WORKFLOW, WORKFLOW_COMPONENTS};
use ah_contracts::llm::{ChatMessage, ChatRole, ModelProvider, ModelRequest};
use ah_contracts::prelude::{
    Effect, MessageQueue, Seam, ServiceKey, SessionEventKind, SessionLog, ToolRegistry,
    WebFetchRequest, WebProvider,
};
use ah_contracts::workflow::{
    CheckpointedOutput, ComponentAbility, EdgeSpec, NodeKind, NodeSpec, WorkflowComponent,
    WorkflowComponentRegistry, WorkflowEngine, WorkflowError, WorkflowOutput, WorkflowSpec,
    WorkflowStreamSink,
};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use futures_util::future::join_all;
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
    sessions: Arc<dyn SessionLog>,
    /// 可选 web seam(Http 节点用;未挂载时 Http 节点显式报错)。
    web: Option<Arc<dyn WebProvider>>,
    /// 可选 queue seam(Questioner 节点用;未挂载时显式报错)。
    queue: Option<Arc<dyn MessageQueue>>,
    /// 用户组件 registry;未注册组件时显式报错,不静默降级。
    components: Option<Arc<dyn WorkflowComponentRegistry>>,
    ctx: Context,
}

impl WorkflowEngineImpl {
    /// 构建引擎(注入 llm + tools + 可选 web)。
    pub fn new(
        llm: Arc<dyn ModelProvider>,
        tools: Arc<dyn ToolRegistry>,
        sessions: Arc<dyn SessionLog>,
        web: Option<Arc<dyn WebProvider>>,
        queue: Option<Arc<dyn MessageQueue>>,
        ctx: Context,
    ) -> Self {
        Self {
            llm,
            tools,
            sessions,
            web,
            queue,
            components: None,
            ctx,
        }
    }

    /// 设置用户组件 registry;组件查找按 node.config.component 名称进行。
    pub fn with_components(mut self, components: Arc<dyn WorkflowComponentRegistry>) -> Self {
        self.components = Some(components);
        self
    }

    /// 从 Context 读取可选组件 registry。
    fn components_from_context(&self) -> Option<Arc<dyn WorkflowComponentRegistry>> {
        self.components.clone().or_else(|| {
            self.ctx
                .service::<dyn WorkflowComponentRegistry>(&WORKFLOW_COMPONENTS)
        })
    }

    /// 挂载 web seam(可选;Http 节点使用)。
    pub fn with_web(mut self, web: Arc<dyn WebProvider>) -> Self {
        self.web = Some(web);
        self
    }

    /// Http 节点:真实 HTTP GET(config: {url, timeout_ms?})。
    async fn run_http(&self, node: &NodeSpec, _input: &Value) -> Result<Value, WorkflowError> {
        let url = node
            .config
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| WorkflowError(format!("http node {} missing url", node.id)))?;
        let web = self
            .web
            .clone()
            .ok_or_else(|| WorkflowError("http node: web seam not mounted".to_string()))?;
        let timeout_ms = node.config.get("timeout_ms").and_then(Value::as_u64);
        let result = web
            .fetch(WebFetchRequest {
                url: url.to_string(),
                timeout_ms,
            })
            .map_err(|e| WorkflowError(format!("http node failed: {e}")))?;
        Ok(json!({ "status": result.status, "body": result.body }))
    }

    /// Intent 节点:关键字确定性路由或 LLM 路由(config: {intents, mode, patterns})。
    async fn run_intent(&self, node: &NodeSpec, input: &Value) -> Result<Value, WorkflowError> {
        let intents = node
            .config
            .get("intents")
            .and_then(Value::as_array)
            .ok_or_else(|| WorkflowError(format!("intent node {} missing intents", node.id)))?;
        let mode = node
            .config
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("keyword");
        let text = input
            .get("output")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let input_text = if text.is_empty() {
            input.get("input").and_then(Value::as_str).unwrap_or("")
        } else {
            text
        };

        match mode {
            "keyword" => {
                // 确定性:命中关键词的第一个 intent。
                let patterns = node.config.get("patterns").cloned().unwrap_or(Value::Null);
                let lower = input_text.to_lowercase();
                for intent in intents {
                    let id = intent.get("id").and_then(Value::as_str).unwrap_or_default();
                    let keywords = patterns
                        .get(id)
                        .and_then(Value::as_array)
                        .map(|arr| {
                            arr.iter()
                                .filter_map(Value::as_str)
                                .map(|k| k.to_lowercase())
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    if keywords.iter().any(|k| lower.contains(k)) {
                        return Ok(json!({ "intent": id, "confidence": 1.0 }));
                    }
                }
                Err(WorkflowError(format!(
                    "intent node {}: no keyword matched input",
                    node.id
                )))
            }
            "llm" => {
                // 真实 LLM 路由:要求严格 JSON;不可解析显式报错。
                let candidates = intents
                    .iter()
                    .map(|i| {
                        json!({
                            "id": i.get("id").cloned().unwrap_or_default(),
                            "description": i.get("description").cloned().unwrap_or_default(),
                        })
                    })
                    .collect::<Vec<_>>();
                let prompt = format!(
                    "Classify the user request into exactly one intent. Reply with ONLY \
                     {{\"intent\":\"<id>\"}}.\nIntents: {}\nRequest: {}",
                    serde_json::to_string(&candidates).unwrap_or_default(),
                    input_text
                );
                let response = self
                    .llm
                    .chat(ModelRequest {
                        messages: vec![ChatMessage::new(ChatRole::User, prompt)],
                        ..Default::default()
                    })
                    .await
                    .map_err(|e| WorkflowError(format!("intent llm failed: {e}")))?;
                let content = response.content;
                let start = content.find('{');
                let end = content.rfind('}');
                let parsed = start.and_then(|s| end.filter(|e| *e > s).map(|e| &content[s..=e]));
                let parsed = parsed
                    .and_then(|slice| serde_json::from_str::<Value>(slice).ok())
                    .ok_or_else(|| {
                        WorkflowError(format!(
                            "intent node {}: llm returned unparseable routing: {}",
                            node.id,
                            content.chars().take(120).collect::<String>()
                        ))
                    })?;
                let intent = parsed
                    .get("intent")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let valid = intents
                    .iter()
                    .any(|i| i.get("id").and_then(Value::as_str) == Some(intent));
                if !valid {
                    return Err(WorkflowError(format!(
                        "intent node {}: llm returned unknown intent {intent}",
                        node.id
                    )));
                }
                Ok(json!({ "intent": intent, "confidence": 1.0 }))
            }
            other => Err(WorkflowError(format!(
                "intent node {}: unknown mode {other}",
                node.id
            ))),
        }
    }

    /// Questioner 节点:把问题发布到 queue(workflow:question:{node_id}),
    /// 等待答复 channel(workflow:answer:{node_id})的回答(config: {question, timeout_ms?})。
    async fn run_questioner(&self, node: &NodeSpec, input: &Value) -> Result<Value, WorkflowError> {
        let question = node
            .config
            .get("question")
            .and_then(Value::as_str)
            .unwrap_or("please answer");
        let queue = self
            .queue
            .clone()
            .ok_or_else(|| WorkflowError("questioner node: queue seam not mounted".to_string()))?;
        let q_channel = format!("workflow:question:{}", node.id);
        let a_channel = format!("workflow:answer:{}", node.id);
        let context = input
            .get("output")
            .cloned()
            .unwrap_or_else(|| input.clone());
        queue
            .publish(
                &q_channel,
                json!({ "question": question, "context": context }),
            )
            .map_err(|e| WorkflowError(format!("questioner publish: {e}")))?;
        let timeout_ms = node
            .config
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .unwrap_or(10_000);
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            if let Some(message) = queue
                .consume(&a_channel)
                .map_err(|e| WorkflowError(format!("questioner consume: {e}")))?
            {
                let answer = message
                    .payload
                    .get("answer")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                return Ok(json!({ "answer": answer }));
            }
            if std::time::Instant::now() >= deadline {
                return Err(WorkflowError(format!(
                    "questioner node {}: no answer within {timeout_ms}ms",
                    node.id
                )));
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    fn node<'a>(&self, spec: &'a WorkflowSpec, id: &str) -> Result<&'a NodeSpec, WorkflowError> {
        spec.nodes
            .iter()
            .find(|n| n.id == id)
            .ok_or_else(|| WorkflowError(format!("node not found: {id}")))
    }

    fn component_ability(node: &NodeSpec) -> Result<ComponentAbility, WorkflowError> {
        let raw = node
            .config
            .get("ability")
            .and_then(Value::as_str)
            .unwrap_or("invoke");
        serde_json::from_value(Value::String(raw.to_string())).map_err(|_| {
            WorkflowError(format!(
                "component node {} has unknown ability {raw}",
                node.id
            ))
        })
    }

    fn component(&self, node: &NodeSpec) -> Result<Arc<dyn WorkflowComponent>, WorkflowError> {
        let name = node
            .config
            .get("component")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                WorkflowError(format!("component node {} missing component", node.id))
            })?;
        self.components_from_context()
            .and_then(|registry| registry.get(name))
            .ok_or_else(|| WorkflowError(format!("workflow component not registered: {name}")))
    }

    async fn execute_component(
        &self,
        node: &NodeSpec,
        input: &Value,
    ) -> Result<Value, WorkflowError> {
        let component = self.component(node)?;
        match Self::component_ability(node)? {
            ComponentAbility::Invoke => component.invoke(input.clone()).await,
            ComponentAbility::Stream => {
                Ok(json!({ "chunks": component.stream(input.clone()).await? }))
            }
            ComponentAbility::Collect => component.collect(stream_values(input)).await,
            ComponentAbility::Transform => {
                Ok(json!({ "chunks": component.transform(stream_values(input)).await? }))
            }
        }
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
    /// 经 stream_chat 消费流式 seam(provider 支持 SSE 时真实流式;
    /// 默认实现退化为单块,语义等价于 chat)。
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
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let request = ModelRequest {
            messages: vec![ChatMessage::new(ChatRole::User, message)],
            ..Default::default()
        };
        self.llm
            .stream_chat(request, tx)
            .await
            .map_err(|e| WorkflowError(format!("llm node failed: {e}")))?;
        let mut content = String::new();
        let mut tool_calls: Vec<ah_contracts::llm::ToolCall> = Vec::new();
        while let Some(chunk) = rx.recv().await {
            if !chunk.content_delta.is_empty() {
                content.push_str(&chunk.content_delta);
            }
            for delta in chunk.tool_call_deltas {
                while tool_calls.len() <= delta.index {
                    tool_calls.push(ah_contracts::llm::ToolCall {
                        id: String::new(),
                        name: String::new(),
                        arguments: serde_json::Value::Null,
                    });
                }
                if let Some(id) = delta.id {
                    tool_calls[delta.index].id = id;
                }
                if let Some(name) = delta.name {
                    tool_calls[delta.index].name = name;
                }
                if !delta.arguments.is_empty() {
                    let current = tool_calls[delta.index]
                        .arguments
                        .as_str()
                        .unwrap_or_default();
                    let joined = format!("{current}{}", delta.arguments);
                    tool_calls[delta.index].arguments =
                        serde_json::from_str(&joined).unwrap_or_else(|_| serde_json::json!(joined));
                }
            }
        }
        if !tool_calls.is_empty() {
            return Ok(json!({ "content": content, "tool_calls": tool_calls }));
        }
        Ok(json!({ "content": content }))
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

    /// 执行 SubWorkflow 节点:递归运行内嵌 WorkflowSpec。
    async fn run_subworkflow(
        &self,
        node: &NodeSpec,
        input: &Value,
    ) -> Result<Value, WorkflowError> {
        let sub: WorkflowSpec =
            serde_json::from_value(node.config.get("workflow").cloned().ok_or_else(|| {
                WorkflowError(format!("subworkflow node {} missing workflow", node.id))
            })?)
            .map_err(|e| WorkflowError(format!("invalid subworkflow spec: {e}")))?;
        let output = self.run(&sub, input.clone()).await?;
        Ok(json!({ "subworkflow": sub.id, "output": output.output }))
    }

    /// 执行 Parallel 节点:并发执行多个目标节点(Llm/Tool 叶节点),join 结果。
    async fn run_parallel(
        &self,
        spec: &WorkflowSpec,
        node: &NodeSpec,
        input: &Value,
    ) -> Result<Value, WorkflowError> {
        let targets: Vec<String> = node
            .config
            .get("targets")
            .and_then(Value::as_array)
            .ok_or_else(|| WorkflowError(format!("parallel node {} missing targets", node.id)))?
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        if targets.is_empty() {
            return Err(WorkflowError(format!(
                "parallel node {} has no targets",
                node.id
            )));
        }

        // 并发执行每个目标(真实并发 join)。
        let mut futures = Vec::new();
        for target in &targets {
            let target_node = self.node(spec, target)?.clone();
            let input = input.clone();
            futures.push(async move {
                match target_node.kind {
                    NodeKind::Llm => (
                        target_node.id.clone(),
                        self.run_llm(&target_node, &input).await,
                    ),
                    NodeKind::Tool => (
                        target_node.id.clone(),
                        self.run_tool(&target_node, &input).await,
                    ),
                    NodeKind::Component => (
                        target_node.id.clone(),
                        self.execute_component(&target_node, &input).await,
                    ),
                    _ => (
                        target_node.id.clone(),
                        Err(WorkflowError(format!(
                            "parallel target must be Llm, Tool, or Component, got {:?}",
                            target_node.kind
                        ))),
                    ),
                }
            });
        }
        let results = join_all(futures).await;

        let mut outputs = serde_json::Map::new();
        for (id, result) in results {
            outputs.insert(id, result?);
        }
        Ok(json!({ "outputs": outputs }))
    }

    async fn execute_node(
        &self,
        spec: &WorkflowSpec,
        node: &NodeSpec,
        input: &Value,
    ) -> Result<Value, WorkflowError> {
        let output = if node.config.get("component").is_some() || node.kind == NodeKind::Component {
            self.execute_component(node, input).await?
        } else {
            match node.kind {
                NodeKind::Start => input.clone(),
                NodeKind::End => input.clone(),
                NodeKind::Llm => self.run_llm(node, input).await?,
                NodeKind::Tool => self.run_tool(node, input).await?,
                NodeKind::Loop => self.run_loop(spec, node, input).await?,
                NodeKind::SubWorkflow => self.run_subworkflow(node, input).await?,
                NodeKind::Parallel => self.run_parallel(spec, node, input).await?,
                NodeKind::Http => self.run_http(node, input).await?,
                NodeKind::Intent => self.run_intent(node, input).await?,
                NodeKind::Questioner => self.run_questioner(node, input).await?,
                NodeKind::Component => unreachable!("component nodes are handled above"),
            }
        };
        self.ctx.emit(WorkflowNodeEvent {
            workflow_id: spec.id.clone(),
            node_id: node.id.clone(),
            output: output.clone(),
        });
        let _ = self.sessions.append(
            SessionEventKind::AgentStep,
            json!({ "workflow": spec.id, "node": node.id, "output": output }),
        );
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
    async fn run_llm_stream(
        &self,
        node: &NodeSpec,
        input: &Value,
        workflow_id: &str,
        sink: &Arc<dyn WorkflowStreamSink>,
        next_index: &mut usize,
    ) -> Result<Value, WorkflowError> {
        let prompt = node
            .config
            .get("prompt")
            .and_then(Value::as_str)
            .unwrap_or("continue");
        let context = input
            .get("output")
            .cloned()
            .unwrap_or_else(|| input.clone());
        let request = ModelRequest {
            messages: vec![ChatMessage::new(
                ChatRole::User,
                format!("{prompt}\nContext: {context}"),
            )],
            ..Default::default()
        };
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let llm = self.llm.clone();
        let producer = tokio::spawn(async move { llm.stream_chat(request, tx).await });

        let mut content = String::new();
        let mut tool_calls: Vec<ah_contracts::llm::ToolCall> = Vec::new();
        while let Some(chunk) = rx.recv().await {
            if sink.is_cancelled() {
                producer.abort();
                return Err(WorkflowError("workflow stream cancelled".to_string()));
            }
            if !chunk.content_delta.is_empty()
                || !chunk.reasoning_delta.is_empty()
                || !chunk.tool_call_deltas.is_empty()
            {
                let deltas: Vec<Value> = chunk
                    .tool_call_deltas
                    .iter()
                    .map(|delta| {
                        json!({
                            "index": delta.index,
                            "id": delta.id,
                            "name": delta.name,
                            "arguments": delta.arguments,
                        })
                    })
                    .collect();
                sink.emit(json!({
                    "type": "workflow_delta",

                    "index": *next_index,
                    "payload": {
                        "workflow": workflow_id,
                        "node": node.id,
                        "content_delta": chunk.content_delta,
                        "reasoning_delta": chunk.reasoning_delta,
                        "tool_call_deltas": deltas,
                    }
                }))
                .await?;
                *next_index += 1;
            }
            content.push_str(&chunk.content_delta);
            for delta in chunk.tool_call_deltas {
                while tool_calls.len() <= delta.index {
                    tool_calls.push(ah_contracts::llm::ToolCall {
                        id: String::new(),
                        name: String::new(),
                        arguments: Value::Null,
                    });
                }
                if let Some(id) = delta.id {
                    tool_calls[delta.index].id = id;
                }
                if let Some(name) = delta.name {
                    tool_calls[delta.index].name = name;
                }
                if !delta.arguments.is_empty() {
                    let current = tool_calls[delta.index]
                        .arguments
                        .as_str()
                        .unwrap_or_default();
                    let joined = format!("{current}{}", delta.arguments);
                    tool_calls[delta.index].arguments =
                        serde_json::from_str(&joined).unwrap_or_else(|_| json!(joined));
                }
            }
        }
        let producer_result = producer
            .await
            .map_err(|error| WorkflowError(format!("llm stream task failed: {error}")))?;
        producer_result.map_err(|error| WorkflowError(format!("llm node failed: {error}")))?;
        if !tool_calls.is_empty() {
            Ok(json!({ "content": content, "tool_calls": tool_calls }))
        } else {
            Ok(json!({ "content": content }))
        }
    }
    async fn run_component_stream(
        &self,
        node: &NodeSpec,
        input: &Value,
        workflow_id: &str,
        sink: &Arc<dyn WorkflowStreamSink>,
        stream_index: &mut usize,
    ) -> Result<Value, WorkflowError> {
        let component = self.component(node)?;
        let ability = Self::component_ability(node)?;
        let output = match ability {
            ComponentAbility::Invoke => component.invoke(input.clone()).await?,
            ComponentAbility::Collect => component.collect(stream_values(input)).await?,
            ComponentAbility::Stream => {
                let chunks = component.stream(input.clone()).await?;
                for chunk in &chunks {
                    if sink.is_cancelled() {
                        return Err(WorkflowError("workflow stream cancelled".to_string()));
                    }
                    sink.emit(json!({
                        "type": "workflow_delta",
                        "index": *stream_index,
                        "payload": {"workflow": workflow_id, "node": node.id, "output": chunk}
                    }))
                    .await?;
                    *stream_index += 1;
                }
                json!({ "chunks": chunks })
            }
            ComponentAbility::Transform => {
                let chunks = component.transform(stream_values(input)).await?;
                for chunk in &chunks {
                    if sink.is_cancelled() {
                        return Err(WorkflowError("workflow stream cancelled".to_string()));
                    }
                    sink.emit(json!({
                        "type": "workflow_delta",
                        "index": *stream_index,
                        "payload": {"workflow": workflow_id, "node": node.id, "output": chunk}
                    }))
                    .await?;
                    *stream_index += 1;
                }
                json!({ "chunks": chunks })
            }
        };
        Ok(output)
    }
    async fn stream_checkpointed_workflow(
        &self,
        spec: &WorkflowSpec,
        input: Value,
        checkpoint_path: &std::path::Path,
        sink: Arc<dyn WorkflowStreamSink>,
    ) -> Result<CheckpointedOutput, WorkflowError> {
        let mut checkpoint = HashMap::new();
        if let Ok(text) = std::fs::read_to_string(checkpoint_path) {
            for line in text.lines() {
                if let Ok(entry) = serde_json::from_str::<serde_json::Map<String, Value>>(line)
                    && let (Some(node_id), Some(output)) = (
                        entry.get("node_id").and_then(Value::as_str),
                        entry.get("output"),
                    )
                {
                    checkpoint.insert(node_id.to_string(), output.clone());
                }
            }
        }
        if let Some(parent) = checkpoint_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| WorkflowError(format!("create checkpoint dir: {error}")))?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(checkpoint_path)
            .map_err(|error| WorkflowError(format!("open checkpoint: {error}")))?;
        use std::io::Write;

        let order = self.topo_order(spec)?;
        if !spec.nodes.iter().any(|node| node.kind == NodeKind::Start) {
            return Err(WorkflowError("workflow missing Start node".to_string()));
        }
        let mut state = HashMap::new();
        let mut executed = Vec::new();
        let mut resumed = Vec::new();
        let mut stream_index = 0usize;
        for node_id in order {
            if sink.is_cancelled() {
                return Err(WorkflowError("workflow stream cancelled".to_string()));
            }
            let node = self.node(spec, &node_id)?;
            if node.kind == NodeKind::End {
                continue;
            }
            let incoming: Vec<&EdgeSpec> = spec
                .edges
                .iter()
                .filter(|edge| edge.to == node_id)
                .collect();
            if !incoming.is_empty() && !incoming.iter().all(|edge| Self::edge_allows(edge, &state))
            {
                continue;
            }
            if let Some(output) = checkpoint.get(&node_id) {
                state.insert(node_id.clone(), output.clone());
                resumed.push(node_id.clone());
                sink.emit(json!({
                    "type": "workflow_resume",
                    "index": stream_index,
                    "payload": {"workflow": spec.id, "node": node.id, "output": output}
                }))
                .await?;
                stream_index += 1;
                continue;
            }
            let input_for_node = if node.kind == NodeKind::Start {
                input.clone()
            } else {
                incoming
                    .iter()
                    .find_map(|edge| state.get(&edge.from).cloned())
                    .unwrap_or_else(|| json!({}))
            };
            let output = if node.kind == NodeKind::Llm {
                let output = self
                    .run_llm_stream(
                        node,
                        &input_for_node,
                        spec.id.as_str(),
                        &sink,
                        &mut stream_index,
                    )
                    .await?;
                self.ctx.emit(WorkflowNodeEvent {
                    workflow_id: spec.id.clone(),
                    node_id: node.id.clone(),
                    output: output.clone(),
                });
                let _ = self.sessions.append(
                    SessionEventKind::AgentStep,
                    json!({"workflow": spec.id, "node": node.id, "output": output}),
                );
                output
            } else if node.config.get("component").is_some() || node.kind == NodeKind::Component {
                self.run_component_stream(
                    node,
                    &input_for_node,
                    spec.id.as_str(),
                    &sink,
                    &mut stream_index,
                )
                .await?
            } else {
                self.execute_node(spec, node, &input_for_node).await?
            };
            let line = serde_json::json!({"node_id": node_id, "output": output});
            writeln!(file, "{line}")
                .map_err(|error| WorkflowError(format!("write checkpoint: {error}")))?;
            sink.emit(json!({
                "type": "workflow_node",
                "index": stream_index,
                "payload": {"workflow": spec.id, "node": node.id, "output": output}
            }))
            .await?;
            stream_index += 1;
            state.insert(node_id.clone(), output);
            executed.push(node_id);
        }
        let end_id = spec
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::End)
            .ok_or_else(|| WorkflowError("workflow missing End node".to_string()))?;
        let output = spec
            .edges
            .iter()
            .filter(|edge| edge.to == end_id.id)
            .find_map(|edge| state.get(&edge.from).cloned())
            .unwrap_or_else(|| json!({}));
        sink.emit(json!({"type": "workflow_final", "index": stream_index, "payload": output}))
            .await?;
        Ok(CheckpointedOutput {
            executed,
            resumed,
            output,
        })
    }

    async fn stream_workflow(
        &self,
        spec: &WorkflowSpec,
        input: Value,
        sink: Arc<dyn WorkflowStreamSink>,
    ) -> Result<WorkflowOutput, WorkflowError> {
        let order = self.topo_order(spec)?;
        if !spec.nodes.iter().any(|node| node.kind == NodeKind::Start) {
            return Err(WorkflowError("workflow missing Start node".to_string()));
        }

        let mut state: HashMap<String, Value> = HashMap::new();
        let mut executed = Vec::new();
        let mut stream_index = 0usize;
        for node_id in order {
            if sink.is_cancelled() {
                return Err(WorkflowError("workflow stream cancelled".to_string()));
            }
            let node = self.node(spec, &node_id)?;
            if node.kind == NodeKind::End {
                continue;
            }
            let incoming: Vec<&EdgeSpec> = spec
                .edges
                .iter()
                .filter(|edge| edge.to == node_id)
                .collect();
            if !incoming.is_empty() && !incoming.iter().all(|edge| Self::edge_allows(edge, &state))
            {
                continue;
            }
            let input_for_node = if node.kind == NodeKind::Start {
                input.clone()
            } else {
                incoming
                    .iter()
                    .find_map(|edge| state.get(&edge.from).cloned())
                    .unwrap_or_else(|| json!({}))
            };
            let output = if node.kind == NodeKind::Llm {
                let output = self
                    .run_llm_stream(
                        node,
                        &input_for_node,
                        spec.id.as_str(),
                        &sink,
                        &mut stream_index,
                    )
                    .await?;
                self.ctx.emit(WorkflowNodeEvent {
                    workflow_id: spec.id.clone(),
                    node_id: node.id.clone(),
                    output: output.clone(),
                });
                let _ = self.sessions.append(
                    SessionEventKind::AgentStep,
                    json!({"workflow": spec.id, "node": node.id, "output": output}),
                );
                output
            } else if node.config.get("component").is_some() || node.kind == NodeKind::Component {
                self.run_component_stream(
                    node,
                    &input_for_node,
                    spec.id.as_str(),
                    &sink,
                    &mut stream_index,
                )
                .await?
            } else {
                self.execute_node(spec, node, &input_for_node).await?
            };
            sink.emit(json!({
                "type": "workflow_node",
                "index": stream_index,
                "payload": {
                    "workflow": spec.id,
                    "node": node.id,
                    "output": output,
                }
            }))
            .await?;
            stream_index += 1;
            state.insert(node_id.clone(), output);
            executed.push(node_id);
        }

        let end_id = spec
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::End)
            .ok_or_else(|| WorkflowError("workflow missing End node".to_string()))?;
        let end_input = spec
            .edges
            .iter()
            .filter(|edge| edge.to == end_id.id)
            .find_map(|edge| state.get(&edge.from).cloned())
            .unwrap_or_else(|| json!({}));
        sink.emit(json!({
            "type": "workflow_final",
            "index": stream_index,
            "payload": end_input.clone(),
        }))
        .await?;
        Ok(WorkflowOutput {
            output: end_input,
            executed,
        })
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

    async fn stream(
        &self,
        spec: &WorkflowSpec,
        input: Value,
        sink: Arc<dyn WorkflowStreamSink>,
    ) -> Result<WorkflowOutput, WorkflowError> {
        let result = self.stream_workflow(spec, input, sink.clone()).await;
        let close_result = sink.close().await;
        match (result, close_result) {
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
            (Ok(output), Ok(())) => Ok(output),
        }
    }
    async fn stream_checkpointed(
        &self,
        spec: &WorkflowSpec,
        input: Value,
        checkpoint_path: &std::path::Path,
        sink: Arc<dyn WorkflowStreamSink>,
    ) -> Result<CheckpointedOutput, WorkflowError> {
        let result = self
            .stream_checkpointed_workflow(spec, input, checkpoint_path, sink.clone())
            .await;
        let close_result = sink.close().await;
        match (result, close_result) {
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
            (Ok(output), Ok(())) => Ok(output),
        }
    }

    async fn run_checkpointed(
        &self,
        spec: &WorkflowSpec,
        input: Value,
        checkpoint_path: &std::path::Path,
    ) -> Result<CheckpointedOutput, WorkflowError> {
        // 加载检查点:{node_id: output}。
        let mut checkpoint: HashMap<String, Value> = HashMap::new();
        if let Ok(text) = std::fs::read_to_string(checkpoint_path) {
            for line in text.lines() {
                if let Ok(entry) = serde_json::from_str::<serde_json::Map<String, Value>>(line)
                    && let Some(node_id) = entry.get("node_id").and_then(Value::as_str)
                    && let Some(output) = entry.get("output")
                {
                    checkpoint.insert(node_id.to_string(), output.clone());
                }
            }
        }
        // 追加模式打开(真实续写)。
        if let Some(parent) = checkpoint_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| WorkflowError(format!("create checkpoint dir: {e}")))?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(checkpoint_path)
            .map_err(|e| WorkflowError(format!("open checkpoint: {e}")))?;
        use std::io::Write;

        let order = self.topo_order(spec)?;
        let mut state: HashMap<String, Value> = HashMap::new();
        let mut executed: Vec<String> = Vec::new();
        let mut resumed: Vec<String> = Vec::new();
        for node_id in order {
            let node = self.node(spec, &node_id)?;
            if node.kind == NodeKind::End {
                continue;
            }
            let incoming: Vec<&EdgeSpec> = spec.edges.iter().filter(|e| e.to == node_id).collect();
            if !incoming.is_empty() && !incoming.iter().all(|e| Self::edge_allows(e, &state)) {
                continue;
            }
            // 检查点命中:复用输出,不重执行。
            if let Some(output) = checkpoint.get(&node_id) {
                state.insert(node_id.clone(), output.clone());
                resumed.push(node_id.clone());
                continue;
            }
            let input_for_node = if node.kind == NodeKind::Start {
                input.clone()
            } else {
                incoming
                    .iter()
                    .find_map(|e| state.get(&e.from).cloned())
                    .unwrap_or_else(|| json!({}))
            };
            let output = self.execute_node(spec, node, &input_for_node).await?;
            // 真实落盘(每节点一行)。
            let line = serde_json::json!({ "node_id": node_id, "output": output.clone() });
            writeln!(file, "{line}")
                .map_err(|e| WorkflowError(format!("write checkpoint: {e}")))?;
            state.insert(node_id.clone(), output);
            executed.push(node_id.clone());
        }
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
        Ok(CheckpointedOutput {
            executed,
            resumed,
            output: end_input,
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
/// 将组件输出规范化为 Python stream actor 使用的 chunk 列表。
fn stream_values(value: &Value) -> Vec<Value> {
    for key in ["chunks", "stream", "outputs"] {
        if let Some(values) = value.get(key).and_then(Value::as_array) {
            return values.clone();
        }
    }
    if let Some(values) = value.as_array() {
        return values.clone();
    }
    vec![value.clone()]
}

impl Plugin for WorkflowPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-workflow"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![WORKFLOW]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![LLM, TOOLS, SESSIONS]
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
        let sessions =
            ctx.service::<dyn SessionLog>(&SESSIONS)
                .ok_or_else(|| PluginError::Apply {
                    plugin: self.name(),
                    message: "sessions seam not registered".to_string(),
                })?;
        let mut engine = WorkflowEngineImpl::new(
            llm,
            tools,
            sessions,
            ctx.service::<dyn WebProvider>(&WEB),
            ctx.service::<dyn MessageQueue>(&QUEUE),
            ctx.clone(),
        );
        if let Some(components) = ctx.service::<dyn WorkflowComponentRegistry>(&WORKFLOW_COMPONENTS)
        {
            engine = engine.with_components(components);
        }
        let engine: Arc<dyn WorkflowEngine> = Arc::new(engine);
        Ok(vec![ctx.register(WORKFLOW, engine)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::WORKFLOW;
    use ah_hub::plugin::DynPlugin;
    use std::sync::Arc as StdArc;

    #[derive(Default)]
    struct RecordingSink {
        chunks: std::sync::Mutex<Vec<Value>>,
        closed: std::sync::atomic::AtomicBool,
        cancelled: std::sync::atomic::AtomicBool,
    }

    impl Seam for RecordingSink {}

    #[async_trait]
    impl ah_contracts::workflow::WorkflowStreamSink for RecordingSink {
        async fn emit(&self, chunk: Value) -> Result<(), WorkflowError> {
            self.chunks.lock().unwrap().push(chunk);
            Ok(())
        }

        async fn close(&self) -> Result<(), WorkflowError> {
            self.closed.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
        fn is_cancelled(&self) -> bool {
            self.cancelled.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

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
            StdArc::new(ah_plugins_queue::QueuePlugin::new(root.join("queue"))),
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
    async fn subworkflow_node_runs_nested_spec() {
        let root = std::env::temp_dir().join(format!("ah-wf-sub-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let engine = ctx
            .service::<dyn WorkflowEngine>(&WORKFLOW)
            .expect("workflow");

        let sub = WorkflowSpec {
            id: "sub-inner".to_string(),
            nodes: vec![
                NodeSpec {
                    id: "start".into(),
                    kind: NodeKind::Start,
                    config: json!({}),
                },
                NodeSpec {
                    id: "write".into(),
                    kind: NodeKind::Tool,
                    config: json!({ "tool": "write_file", "args": { "path": "inner.txt", "content": "sub" } }),
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
                    to: "end".into(),
                    condition: None,
                },
            ],
        };

        let spec = WorkflowSpec {
            id: "wf-sub".to_string(),
            nodes: vec![
                NodeSpec {
                    id: "start".into(),
                    kind: NodeKind::Start,
                    config: json!({}),
                },
                NodeSpec {
                    id: "sub".into(),
                    kind: NodeKind::SubWorkflow,
                    config: json!({ "workflow": sub }),
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
                    to: "sub".into(),
                    condition: None,
                },
                EdgeSpec {
                    from: "sub".into(),
                    to: "end".into(),
                    condition: None,
                },
            ],
        };

        let output = engine.run(&spec, json!({})).await.expect("run");
        // 子工作流真实执行:inner.txt 被真实 write_file 创建。
        let fs = ctx
            .service::<dyn ah_contracts::fs::FsProvider>(&ah_contracts::keys::FS)
            .expect("fs");
        assert!(fs.exists("inner.txt"));
        assert!(output.executed.contains(&"sub".to_string()));

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn parallel_node_runs_targets_concurrently() {
        let root = std::env::temp_dir().join(format!("ah-wf-par-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let engine = ctx
            .service::<dyn WorkflowEngine>(&WORKFLOW)
            .expect("workflow");

        let spec = WorkflowSpec {
            id: "wf-par".to_string(),
            nodes: vec![
                NodeSpec {
                    id: "start".into(),
                    kind: NodeKind::Start,
                    config: json!({}),
                },
                NodeSpec {
                    id: "par".into(),
                    kind: NodeKind::Parallel,
                    config: json!({ "targets": ["w1", "w2"] }),
                },
                NodeSpec {
                    id: "w1".into(),
                    kind: NodeKind::Tool,
                    config: json!({ "tool": "write_file", "args": { "path": "a.txt", "content": "a" } }),
                },
                NodeSpec {
                    id: "w2".into(),
                    kind: NodeKind::Tool,
                    config: json!({ "tool": "write_file", "args": { "path": "b.txt", "content": "b" } }),
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
                    to: "par".into(),
                    condition: None,
                },
                EdgeSpec {
                    from: "par".into(),
                    to: "end".into(),
                    condition: None,
                },
            ],
        };

        let output = engine.run(&spec, json!({})).await.expect("run");
        // 两个并行目标都真实执行(真实文件都创建)。
        let fs = ctx
            .service::<dyn ah_contracts::fs::FsProvider>(&ah_contracts::keys::FS)
            .expect("fs");
        assert!(fs.exists("a.txt"));
        assert!(fs.exists("b.txt"));
        // 输出合并了 w1/w2。
        let out = output.output.to_string();
        assert!(out.contains("a.txt"));
        assert!(out.contains("b.txt"));

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn execution_trace_is_written_to_session_log() {
        let root = std::env::temp_dir().join(format!("ah-wf-trace-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let engine = ctx
            .service::<dyn WorkflowEngine>(&WORKFLOW)
            .expect("workflow");
        let sessions = ctx
            .service::<dyn ah_contracts::session::SessionLog>(&ah_contracts::keys::SESSIONS)
            .expect("sessions");

        let spec = WorkflowSpec {
            id: "wf-trace".to_string(),
            nodes: vec![
                NodeSpec {
                    id: "start".into(),
                    kind: NodeKind::Start,
                    config: json!({}),
                },
                NodeSpec {
                    id: "write".into(),
                    kind: NodeKind::Tool,
                    config: json!({ "tool": "write_file", "args": { "path": "t.txt", "content": "t" } }),
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
                    to: "end".into(),
                    condition: None,
                },
            ],
        };

        let _ = engine.run(&spec, json!({})).await.expect("run");
        // 每个已执行节点都写了轨迹事件(AgentStep,含 workflow/node)。
        let events = sessions.events();
        let traces: Vec<_> = events
            .iter()
            .filter(|e| e.kind == ah_contracts::session::SessionEventKind::AgentStep)
            .collect();
        assert_eq!(traces.len(), 2); // start + write
        assert_eq!(traces[0].payload["workflow"], "wf-trace");
        assert_eq!(traces[0].payload["node"], "start");

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

    #[tokio::test]
    async fn http_node_calls_real_local_server() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let url = format!("http://{addr}/");
        let handle = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let body = "wf http ok";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });

        let root = std::env::temp_dir().join(format!("ah-wf-http-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let engine = ctx
            .service::<dyn WorkflowEngine>(&WORKFLOW)
            .expect("engine");
        let spec = WorkflowSpec {
            id: "wf-http".to_string(),
            nodes: vec![
                NodeSpec {
                    id: "start".to_string(),
                    kind: NodeKind::Start,
                    config: json!({}),
                },
                NodeSpec {
                    id: "fetch".to_string(),
                    kind: NodeKind::Http,
                    config: json!({ "url": url, "timeout_ms": 5000 }),
                },
                NodeSpec {
                    id: "end".to_string(),
                    kind: NodeKind::End,
                    config: json!({}),
                },
            ],
            edges: vec![
                EdgeSpec {
                    from: "start".to_string(),
                    to: "fetch".to_string(),
                    condition: None,
                },
                EdgeSpec {
                    from: "fetch".to_string(),
                    to: "end".to_string(),
                    condition: None,
                },
            ],
        };
        let output = engine.run(&spec, json!({})).await.expect("run");
        assert_eq!(output.output["status"], 200);
        assert_eq!(output.output["body"], "wf http ok");

        handle.join().expect("server");
        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn intent_keyword_routes_deterministically() {
        let root = std::env::temp_dir().join(format!("ah-wf-intent-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let engine = ctx
            .service::<dyn WorkflowEngine>(&WORKFLOW)
            .expect("engine");
        let spec = WorkflowSpec {
            id: "wf-intent".to_string(),
            nodes: vec![
                NodeSpec {
                    id: "start".to_string(),
                    kind: NodeKind::Start,
                    config: json!({}),
                },
                NodeSpec {
                    id: "route".to_string(),
                    kind: NodeKind::Intent,
                    config: json!({
                        "mode": "keyword",
                        "intents": [
                            { "id": "research", "description": "research a topic" },
                            { "id": "coding", "description": "write code" },
                        ],
                        "patterns": {
                            "research": ["research", "investigate"],
                            "coding": ["code", "implement", "write"],
                        },
                    }),
                },
                NodeSpec {
                    id: "end".to_string(),
                    kind: NodeKind::End,
                    config: json!({}),
                },
            ],
            edges: vec![
                EdgeSpec {
                    from: "start".to_string(),
                    to: "route".to_string(),
                    condition: None,
                },
                EdgeSpec {
                    from: "route".to_string(),
                    to: "end".to_string(),
                    condition: None,
                },
            ],
        };
        let output = engine
            .run(&spec, json!({ "input": "please implement a feature" }))
            .await
            .expect("run");
        assert_eq!(output.output["intent"], "coding");
        assert_eq!(output.output["confidence"], 1.0);
        // 无匹配 → 显式错误。
        let err = engine
            .run(&spec, json!({ "input": "completely unrelated" }))
            .await
            .expect_err("no keyword");
        assert!(err.0.contains("no keyword matched"));

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn intent_llm_mode_errors_explicitly_with_stub() {
        let root = std::env::temp_dir().join(format!("ah-wf-intentllm-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let engine = ctx
            .service::<dyn WorkflowEngine>(&WORKFLOW)
            .expect("engine");
        let spec = WorkflowSpec {
            id: "wf-intent-llm".to_string(),
            nodes: vec![
                NodeSpec {
                    id: "start".to_string(),
                    kind: NodeKind::Start,
                    config: json!({}),
                },
                NodeSpec {
                    id: "route".to_string(),
                    kind: NodeKind::Intent,
                    config: json!({
                        "mode": "llm",
                        "intents": [{ "id": "a", "description": "intent a" }],
                    }),
                },
                NodeSpec {
                    id: "end".to_string(),
                    kind: NodeKind::End,
                    config: json!({}),
                },
            ],
            edges: vec![
                EdgeSpec {
                    from: "start".to_string(),
                    to: "route".to_string(),
                    condition: None,
                },
                EdgeSpec {
                    from: "route".to_string(),
                    to: "end".to_string(),
                    condition: None,
                },
            ],
        };
        // dev 用 mock 桩不会返回严格 JSON → 显式报错(不静默)。
        let err = engine
            .run(&spec, json!({ "input": "do something" }))
            .await
            .expect_err("mock cannot route");
        assert!(err.0.contains("unparseable") || err.0.contains("unknown intent"));

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn questioner_asks_via_queue_and_consumes_answer() {
        use ah_contracts::queue::MessageQueue;

        let root = std::env::temp_dir().join(format!("ah-wf-q-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let queue = ctx
            .service::<dyn MessageQueue>(&ah_contracts::keys::QUEUE)
            .expect("queue");
        // 预置回答:真实 queue 通道(workflow:answer:ask)。
        queue
            .publish("workflow:answer:ask", json!({ "answer": "42" }))
            .expect("pre-answer");

        let engine = ctx
            .service::<dyn WorkflowEngine>(&WORKFLOW)
            .expect("engine");
        let spec = WorkflowSpec {
            id: "wf-q".to_string(),
            nodes: vec![
                NodeSpec {
                    id: "start".to_string(),
                    kind: NodeKind::Start,
                    config: json!({}),
                },
                NodeSpec {
                    id: "ask".to_string(),
                    kind: NodeKind::Questioner,
                    config: json!({ "question": "what is the meaning?", "timeout_ms": 3000 }),
                },
                NodeSpec {
                    id: "end".to_string(),
                    kind: NodeKind::End,
                    config: json!({}),
                },
            ],
            edges: vec![
                EdgeSpec {
                    from: "start".to_string(),
                    to: "ask".to_string(),
                    condition: None,
                },
                EdgeSpec {
                    from: "ask".to_string(),
                    to: "end".to_string(),
                    condition: None,
                },
            ],
        };
        let output = engine.run(&spec, json!({})).await.expect("run");
        assert_eq!(output.output["answer"], "42");
        // 问题已发布到真实 queue。
        let question = queue.backlog("workflow:question:ask").expect("questions");
        assert_eq!(question.len(), 1);
        assert_eq!(question[0].payload["question"], "what is the meaning?");

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn questioner_times_out_without_answer() {
        let root = std::env::temp_dir().join(format!("ah-wf-qto-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let engine = ctx
            .service::<dyn WorkflowEngine>(&WORKFLOW)
            .expect("engine");
        let spec = WorkflowSpec {
            id: "wf-qto".to_string(),
            nodes: vec![
                NodeSpec {
                    id: "start".to_string(),
                    kind: NodeKind::Start,
                    config: json!({}),
                },
                NodeSpec {
                    id: "ask".to_string(),
                    kind: NodeKind::Questioner,
                    config: json!({ "question": "anyone?", "timeout_ms": 300 }),
                },
                NodeSpec {
                    id: "end".to_string(),
                    kind: NodeKind::End,
                    config: json!({}),
                },
            ],
            edges: vec![
                EdgeSpec {
                    from: "start".to_string(),
                    to: "ask".to_string(),
                    condition: None,
                },
                EdgeSpec {
                    from: "ask".to_string(),
                    to: "end".to_string(),
                    condition: None,
                },
            ],
        };
        let err = engine.run(&spec, json!({})).await.expect_err("no answer");
        assert!(err.0.contains("no answer within"));

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn checkpoint_persists_and_resumes() {
        let root = std::env::temp_dir().join(format!("ah-wf-cp-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let engine = ctx
            .service::<dyn WorkflowEngine>(&WORKFLOW)
            .expect("engine");
        let cp_path = root.join("wf.jsonl");
        let spec = WorkflowSpec {
            id: "wf-cp".to_string(),
            nodes: vec![
                NodeSpec {
                    id: "start".to_string(),
                    kind: NodeKind::Start,
                    config: json!({}),
                },
                NodeSpec {
                    id: "tool".to_string(),
                    kind: NodeKind::Tool,
                    config: json!({ "tool": "list_dir", "args": { "path": "." } }),
                },
                NodeSpec {
                    id: "end".to_string(),
                    kind: NodeKind::End,
                    config: json!({}),
                },
            ],
            edges: vec![
                EdgeSpec {
                    from: "start".to_string(),
                    to: "tool".to_string(),
                    condition: None,
                },
                EdgeSpec {
                    from: "tool".to_string(),
                    to: "end".to_string(),
                    condition: None,
                },
            ],
        };

        // 首次:真实执行并落盘检查点。
        let first = engine
            .run_checkpointed(&spec, json!({}), &cp_path)
            .await
            .expect("first");
        assert!(!first.executed.is_empty(), "nodes executed");
        assert!(first.resumed.is_empty());
        assert!(cp_path.exists(), "checkpoint written");

        // 二次:全部节点从检查点复用,不重执行。
        let second = engine
            .run_checkpointed(&spec, json!({}), &cp_path)
            .await
            .expect("second");
        assert!(second.executed.is_empty(), "no re-execution on resume");
        assert_eq!(
            second.resumed.len(),
            first.executed.len(),
            "all nodes resumed"
        );
        assert_eq!(second.output, first.output, "output preserved");

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
    #[tokio::test]
    async fn stream_emits_node_outputs_and_final_output() {
        let root = std::env::temp_dir().join(format!("ah-wf-stream-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let engine = ctx
            .service::<dyn WorkflowEngine>(&WORKFLOW)
            .expect("workflow");
        let sink = StdArc::new(RecordingSink::default());
        let spec = WorkflowSpec {
            id: "wf-stream".into(),
            nodes: vec![
                NodeSpec {
                    id: "start".into(),
                    kind: NodeKind::Start,
                    config: json!({}),
                },
                NodeSpec {
                    id: "write".into(),
                    kind: NodeKind::Tool,
                    config: json!({"tool": "write_file", "args": {"path": "stream.txt", "content": "ok"}}),
                },
                NodeSpec {
                    id: "llm".into(),
                    kind: NodeKind::Llm,
                    config: json!({"prompt": "summarize"}),
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

        let _output = engine
            .stream(&spec, json!({"input": "go"}), sink.clone())
            .await
            .expect("stream");
        let chunks = sink.chunks.lock().unwrap().clone();
        assert_eq!(chunks.len(), 5);
        assert_eq!(chunks[0]["type"], "workflow_node");
        assert_eq!(chunks[0]["payload"]["node"], "start");
        assert_eq!(chunks[1]["payload"]["node"], "write");
        assert_eq!(
            chunks[1]["payload"]["output"]["output"]["path"],
            "stream.txt"
        );
        assert_eq!(chunks[2]["type"], "workflow_delta");
        assert_eq!(chunks[3]["payload"]["node"], "llm");
        assert_eq!(chunks[4]["type"], "workflow_final");
        assert!(sink.closed.load(std::sync::atomic::Ordering::SeqCst));

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
    #[tokio::test]
    async fn stream_closes_sink_when_cancelled() {
        let root = std::env::temp_dir().join(format!("ah-wf-stream-cancel-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let engine = ctx
            .service::<dyn WorkflowEngine>(&WORKFLOW)
            .expect("workflow");
        let sink = StdArc::new(RecordingSink::default());
        sink.cancelled
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let spec = WorkflowSpec {
            id: "wf-stream-cancel".into(),
            nodes: vec![
                NodeSpec {
                    id: "start".into(),
                    kind: NodeKind::Start,
                    config: json!({}),
                },
                NodeSpec {
                    id: "end".into(),
                    kind: NodeKind::End,
                    config: json!({}),
                },
            ],
            edges: vec![EdgeSpec {
                from: "start".into(),
                to: "end".into(),
                condition: None,
            }],
        };

        let error = engine
            .stream(&spec, json!({}), sink.clone())
            .await
            .expect_err("cancelled stream");
        assert!(error.0.contains("cancelled"));
        assert!(sink.closed.load(std::sync::atomic::Ordering::SeqCst));

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
    #[tokio::test]
    async fn checkpointed_stream_reuses_nodes_and_emits_resume_chunks() {
        let root = std::env::temp_dir().join(format!("ah-wf-stream-cp-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let engine = ctx
            .service::<dyn WorkflowEngine>(&WORKFLOW)
            .expect("workflow");
        let checkpoint = root.join("workflow.jsonl");
        let spec = WorkflowSpec {
            id: "wf-stream-cp".into(),
            nodes: vec![
                NodeSpec {
                    id: "start".into(),
                    kind: NodeKind::Start,
                    config: json!({}),
                },
                NodeSpec {
                    id: "write".into(),
                    kind: NodeKind::Tool,
                    config: json!({"tool": "write_file", "args": {"path": "checkpoint-stream.txt", "content": "ok"}}),
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
                    to: "end".into(),
                    condition: None,
                },
            ],
        };
        let first_sink = StdArc::new(RecordingSink::default());
        let first = engine
            .stream_checkpointed(&spec, json!({}), &checkpoint, first_sink)
            .await
            .expect("first stream");
        assert_eq!(first.executed, vec!["start", "write"]);
        assert!(first.resumed.is_empty());

        let second_sink = StdArc::new(RecordingSink::default());
        let second = engine
            .stream_checkpointed(&spec, json!({}), &checkpoint, second_sink.clone())
            .await
            .expect("resumed stream");
        assert!(second.executed.is_empty());
        assert_eq!(second.resumed, vec!["start", "write"]);
        let chunks = second_sink.chunks.lock().unwrap().clone();
        assert_eq!(chunks[0]["type"], "workflow_resume");
        assert_eq!(chunks[1]["type"], "workflow_resume");
        assert_eq!(chunks[2]["type"], "workflow_final");

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
    struct ComponentRegistry;

    struct StreamTransformCollectComponent;

    impl Seam for ComponentRegistry {}
    impl Seam for StreamTransformCollectComponent {}

    #[async_trait]
    impl ah_contracts::workflow::WorkflowComponent for StreamTransformCollectComponent {
        async fn invoke(&self, input: Value) -> Result<Value, WorkflowError> {
            Ok(input)
        }

        async fn stream(&self, input: Value) -> Result<Vec<Value>, WorkflowError> {
            Ok(vec![json!({"streamed": input}), json!({"streamed": true})])
        }

        async fn collect(&self, inputs: Vec<Value>) -> Result<Value, WorkflowError> {
            Ok(json!({"collected": inputs}))
        }

        async fn transform(&self, inputs: Vec<Value>) -> Result<Vec<Value>, WorkflowError> {
            Ok(inputs
                .into_iter()
                .map(|input| json!({"transformed": input}))
                .collect())
        }
    }

    #[async_trait]
    impl ah_contracts::workflow::WorkflowComponentRegistry for ComponentRegistry {
        fn get(&self, name: &str) -> Option<Arc<dyn WorkflowComponent>> {
            (name == "test-component").then(|| Arc::new(StreamTransformCollectComponent) as _)
        }
    }

    #[tokio::test]
    async fn component_stream_transform_collect_follow_python_ability_contract() {
        let root = std::env::temp_dir().join(format!("ah-wf-components-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let llm = ctx.service::<dyn ModelProvider>(&LLM).expect("llm");
        let tools = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");
        let sessions = ctx.service::<dyn SessionLog>(&SESSIONS).expect("sessions");
        let engine = WorkflowEngineImpl::new(llm, tools, sessions, None, None, ctx)
            .with_components(Arc::new(ComponentRegistry));
        let sink = Arc::new(RecordingSink::default());
        let spec = WorkflowSpec {
            id: "wf-components".into(),
            nodes: vec![
                NodeSpec {
                    id: "start".into(),
                    kind: NodeKind::Start,
                    config: json!({}),
                },
                NodeSpec {
                    id: "stream".into(),
                    kind: NodeKind::Component,
                    config: json!({"component": "test-component", "ability": "stream"}),
                },
                NodeSpec {
                    id: "transform".into(),
                    kind: NodeKind::Component,
                    config: json!({"component": "test-component", "ability": "transform"}),
                },
                NodeSpec {
                    id: "collect".into(),
                    kind: NodeKind::Component,
                    config: json!({"component": "test-component", "ability": "collect"}),
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
                    to: "stream".into(),
                    condition: None,
                },
                EdgeSpec {
                    from: "stream".into(),
                    to: "transform".into(),
                    condition: None,
                },
                EdgeSpec {
                    from: "transform".into(),
                    to: "collect".into(),
                    condition: None,
                },
                EdgeSpec {
                    from: "collect".into(),
                    to: "end".into(),
                    condition: None,
                },
            ],
        };

        let output = engine
            .stream(&spec, json!({"input": 7}), sink.clone())
            .await
            .expect("stream");
        assert_eq!(
            output.output["collected"][0]["transformed"]["streamed"]["input"],
            7
        );
        let chunks = sink.chunks.lock().unwrap();
        assert_eq!(
            chunks
                .iter()
                .filter(|chunk| chunk["type"] == "workflow_delta")
                .count(),
            4
        );
        assert_eq!(
            chunks.last().expect("final chunk")["type"],
            "workflow_final"
        );

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
}
