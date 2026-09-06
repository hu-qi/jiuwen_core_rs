//! # ah-plugins-agent-loop
//!
//! 真实 ReAct agent 循环:注入 llm + tools + sessions 三个 seam。
//! 循环以会话日志为唯一事实来源(日志即真相):
//! 每次模型请求的消息序列由日志投影(derive_messages)重建,
//! 每轮的用户消息/助手消息/工具调用/工具结果都追加到日志。

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

static RECOVERY_OWNER_COUNTER: AtomicU64 = AtomicU64::new(0);

use ah_contracts::agent::{
    AgentCallbackContext, AgentCallbackManager, AgentControl, AgentFailure, AgentResult,
    AgentRunState, InterruptRuntime,
};
use ah_contracts::context::ContextEngine;
use ah_contracts::keys::{
    AGENT_CALLBACKS, AGENT_LOOP, CONTEXT, INTERRUPT, LLM, MODEL_BACKUP, MODEL_BACKUP_POLICY,
    MODEL_PROVIDER_CATALOG, PROMPT, RAILS, SESSIONS, TOOLS,
};
use ah_contracts::llm::{
    ChatMessage, ChatRole, ModelProvider, ModelRequest, ModelResponse, ToolCall, ToolSchema,
};
use ah_contracts::model_backup::{ModelBackup, ModelBackupPolicy, ModelBackupPolicyProvider};
use ah_contracts::model_catalog::ModelProviderCatalog;
use ah_contracts::prelude::Effect;
use ah_contracts::prompt::PromptRegistry;
use ah_contracts::rails::{RailAction, RailInput, RailPhase, RailRuntime};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::session::{SessionEventKind, SessionLog};
use ah_contracts::tools::ToolRegistry;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use serde_json::{Value, json};

/// agent 循环失败种类:契约层结构化枚举(消费方直接读字段,不解析错误字符串)。
pub use ah_contracts::agent::AgentFailure as AgentLoopFailure;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentLoopError {
    pub kind: AgentLoopFailure,
    pub message: String,
}

impl AgentLoopError {
    pub fn new(kind: AgentLoopFailure, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl core::fmt::Display for AgentLoopError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AgentLoopError {}

/// agent/step 事件契约(跨插件共享):定义在 ah-contracts。
pub use ah_contracts::agent::AgentStep;

/// 真实 ReAct 循环(日志驱动)。
#[derive(Clone)]
pub struct AgentLoop {
    llm: Arc<dyn ModelProvider>,
    model_catalog: Option<Arc<dyn ModelProviderCatalog>>,
    tools: Arc<dyn ToolRegistry>,
    sessions: Arc<dyn SessionLog>,
    ctx: Context,
    max_iterations: usize,
    /// 可选上下文引擎(context seam 消费方):组装请求时按预算压缩。
    context: Option<Arc<dyn ContextEngine>>,
    /// 模型可见上下文 token 预算(默认 8192;日志不受影响)。
    token_budget: usize,
    /// 可选 prompt 注册表(prompt seam 消费方):渲染结果注入为请求首条 system 消息。
    prompt: Option<Arc<dyn PromptRegistry>>,
    /// 消费的模板名(默认 "agent")。
    prompt_name: String,
    interrupt: Option<Arc<dyn InterruptRuntime>>,
    callbacks: Option<Arc<dyn AgentCallbackManager>>,
    rails: Option<Arc<dyn RailRuntime>>,
    timeout_ms: Option<u64>,
    request_model: Option<String>,
    request_temperature: Option<f32>,
    system_context: Option<String>,
    backup: Option<Arc<dyn ModelBackup>>,
    backup_policy: ModelBackupPolicy,
}
impl AgentLoop {
    /// 构建循环。
    fn record_step(
        &self,
        session: &Arc<dyn SessionLog>,
        iteration: usize,
        tool_calls: usize,
        done: bool,
    ) {
        let _ = session.append(
            SessionEventKind::AgentStep,
            json!({"iteration": iteration, "tool_calls": tool_calls, "done": done}),
        );
        self.ctx.emit(AgentStep {
            iteration,
            tool_calls,
            done,
        });
    }

    pub fn new(
        llm: Arc<dyn ModelProvider>,
        tools: Arc<dyn ToolRegistry>,
        sessions: Arc<dyn SessionLog>,
        ctx: Context,
        max_iterations: usize,
    ) -> Self {
        Self {
            llm,
            model_catalog: None,
            tools,
            sessions,
            ctx,
            max_iterations,
            context: None,
            token_budget: 8192,
            prompt: None,
            prompt_name: "agent".to_string(),
            interrupt: None,
            callbacks: None,
            rails: None,
            timeout_ms: None,
            request_model: None,
            request_temperature: None,
            system_context: None,
            backup: None,
            backup_policy: ModelBackupPolicy::default(),
        }
    }

    /// 挂载 context seam 消费(可选;未挂载时用日志投影直通)。
    pub fn with_context(mut self, context: Arc<dyn ContextEngine>, token_budget: usize) -> Self {
        self.context = Some(context);
        self.token_budget = token_budget;
        self
    }

    /// 挂载 prompt seam 消费(可选):注册名为 name 的模板存在时,
    /// 渲染结果注入为每次请求首条 system 消息;未注册则无系统提示(文档策略)。
    pub fn with_prompt(mut self, registry: Arc<dyn PromptRegistry>, name: &str) -> Self {
        self.prompt = Some(registry);
        self.prompt_name = name.to_string();
        self
    }

    /// Attach a named provider catalog for request-scoped model switching.
    pub fn with_model_catalog(mut self, catalog: Arc<dyn ModelProviderCatalog>) -> Self {
        self.model_catalog = Some(catalog);
        self
    }

    fn resolve_request_model(&self) -> Result<Arc<dyn ModelProvider>, AgentLoopError> {
        let Some(model_name) = self.request_model.as_deref() else {
            return Ok(self.llm.clone());
        };
        if model_name.trim().is_empty() {
            return Err(AgentLoopError::new(
                AgentLoopFailure::Model,
                "requested model name must not be empty",
            ));
        }
        if model_name == self.llm.name() {
            return Ok(self.llm.clone());
        }
        let Some(catalog) = &self.model_catalog else {
            return Err(AgentLoopError::new(
                AgentLoopFailure::Model,
                format!("model switching requires a provider catalog: {model_name}"),
            ));
        };
        catalog.resolve(model_name).map_err(|error| {
            AgentLoopError::new(
                AgentLoopFailure::Model,
                format!("model switch failed: {error}"),
            )
        })
    }

    /// Attach cooperative controls and an optional per-run deadline.
    /// Configure cooperative controls and an optional deadline for this loop.
    pub fn with_controls(
        mut self,
        interrupt: Option<Arc<dyn InterruptRuntime>>,
        callbacks: Option<Arc<dyn AgentCallbackManager>>,
        timeout_ms: Option<u64>,
    ) -> Self {
        self.interrupt = interrupt;
        self.callbacks = callbacks;
        self.timeout_ms = timeout_ms;
        self
    }

    pub fn with_rails(mut self, rails: Option<Arc<dyn RailRuntime>>) -> Self {
        self.rails = rails;
        self
    }

    async fn notify(
        &self,
        session_id: &str,
        state: AgentRunState,
        iteration: usize,
        payload: Value,
    ) -> Result<(), AgentLoopError> {
        if let Some(callbacks) = &self.callbacks {
            callbacks
                .notify(AgentCallbackContext {
                    session_id: session_id.to_string(),
                    state,
                    iteration,
                    payload,
                })
                .await
                .map_err(|e| {
                    AgentLoopError::new(AgentLoopFailure::Context, format!("callback failed: {e}"))
                })?;
        }
        Ok(())
    }

    async fn notify_checkpoint(
        &self,
        session_id: &str,
        iteration: usize,
        phase: &str,
        payload: Value,
    ) -> Result<(), AgentLoopError> {
        let Some(callbacks) = &self.callbacks else {
            return Ok(());
        };
        let mut payload = payload;
        if let Some(object) = payload.as_object_mut() {
            object.insert("phase".into(), Value::String(phase.into()));
        }
        let control = callbacks
            .notify_checkpoint(AgentCallbackContext {
                session_id: session_id.into(),
                state: AgentRunState::Running,
                iteration,
                payload,
            })
            .await
            .map_err(|error| {
                AgentLoopError::new(
                    AgentLoopFailure::Context,
                    format!("callback failed: {error}"),
                )
            })?;
        match control {
            AgentControl::Continue => Ok(()),
            AgentControl::Interrupt => Err(AgentLoopError::new(
                AgentLoopFailure::Interrupted,
                "agent run interrupted by callback",
            )),
            AgentControl::Cancel => Err(AgentLoopError::new(
                AgentLoopFailure::Cancelled,
                "agent run cancelled by callback",
            )),
        }
    }

    fn control(&self, session_id: &str) -> AgentControl {
        self.interrupt
            .as_ref()
            .map(|runtime| runtime.state(session_id))
            .unwrap_or(AgentControl::Continue)
    }

    /// 从 tools seam 组装模型可见的工具 schema。
    fn tool_schemas(&self) -> Vec<ToolSchema> {
        let mut names = self.tools.names();
        names.sort();
        names
            .iter()
            .filter_map(|name| {
                self.tools.get(name).map(|tool| ToolSchema {
                    name: tool.name().to_string(),
                    description: tool.description().to_string(),
                    parameters: tool.parameters(),
                })
            })
            .collect()
    }

    /// 执行中取消/超时(P1-03):轮询控制状态与截止时间,命中即中止正在运行的
    /// future(模型调用/工具执行)。未挂 interrupt 且无截止时间时直接透传。
    async fn race_control<R>(
        &self,
        session_id: &str,
        deadline: Option<tokio::time::Instant>,
        fut: impl std::future::Future<Output = R>,
    ) -> Result<R, AgentLoopError> {
        if self.interrupt.is_none() && deadline.is_none() {
            return Ok(fut.await);
        }
        tokio::select! {
            biased;
            _ = self.poll_control(session_id, deadline) => {
                if deadline.is_some_and(|dl| tokio::time::Instant::now() >= dl) {
                    Err(AgentLoopError::new(
                        AgentLoopFailure::TimedOut,
                        "agent run timed out",
                    ))
                } else {
                    match self.control(session_id) {
                        AgentControl::Interrupt => Err(AgentLoopError::new(
                            AgentLoopFailure::Interrupted,
                            "agent run interrupted; resume the session to continue",
                        )),
                        AgentControl::Cancel => Err(AgentLoopError::new(
                            AgentLoopFailure::Cancelled,
                            "agent run cancelled",
                        )),
                        AgentControl::Continue => Err(AgentLoopError::new(
                            AgentLoopFailure::Model,
                            "control raced unexpectedly",
                        )),
                    }
                }
            }
            result = fut => Ok(result),
        }
    }

    /// 每 25ms 轮询:命中截止时间或控制状态非 Continue 即返回。
    async fn poll_control(&self, session_id: &str, deadline: Option<tokio::time::Instant>) {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            if deadline.is_some_and(|dl| tokio::time::Instant::now() >= dl) {
                return;
            }
            if let Some(runtime) = &self.interrupt
                && runtime.state(session_id) != AgentControl::Continue
            {
                return;
            }
        }
    }

    /// 在默认会话上运行一轮任务。
    pub async fn run(&self, input: &str) -> AgentResult {
        self.run_in_session(self.sessions.clone(), input).await
    }

    /// 在指定会话上运行一轮任务:日志驱动的 ReAct 循环。
    ///
    /// 会话可为 resume(已有历史)或 fork 出的新会话;历史经日志投影自动保留。
    pub fn with_timeout(mut self, timeout_ms: Option<u64>) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    pub fn with_model_backup(mut self, backup: Option<Arc<dyn ModelBackup>>) -> Self {
        self.backup = backup;
        self
    }
    pub fn with_model_backup_policy(mut self, policy: ModelBackupPolicy) -> Self {
        self.backup_policy = policy;
        self
    }

    pub fn with_request_config(mut self, config: &ah_contracts::agent::AgentRunConfig) -> Self {
        self.timeout_ms = config.timeout_ms;
        self.request_model = config.model.clone();
        self.request_temperature = config.temperature;
        self.system_context = config.system_context.clone();
        self
    }

    /// 统一构造 AgentResult(统计字段由调用方传入)。
    fn result_for(
        session_id: &str,
        state: AgentRunState,
        failure: Option<AgentFailure>,
        error: Option<String>,
        answer: Option<String>,
        iteration: usize,
        tool_calls: usize,
    ) -> AgentResult {
        AgentResult {
            session_id: session_id.to_string(),
            state,
            answer,
            iterations: iteration,
            tool_calls,
            failure,
            error,
        }
    }

    /// 执行中被 race_control 中止(超时/中断/取消)的统一收尾:
    /// 由错误种类推导 state/failure/会话事件,并清空控制状态。
    async fn race_finish(
        &self,
        session: &Arc<dyn SessionLog>,
        session_id: &str,
        iteration: usize,
        error: AgentLoopError,
        tool_calls: usize,
    ) -> AgentResult {
        let (state, failure, event) = match error.kind {
            AgentLoopFailure::TimedOut => (
                AgentRunState::TimedOut,
                AgentFailure::TimedOut,
                SessionEventKind::AgentTimedOut,
            ),
            AgentLoopFailure::Interrupted => (
                AgentRunState::Interrupted,
                AgentFailure::Interrupted,
                SessionEventKind::AgentInterrupted,
            ),
            AgentLoopFailure::Cancelled => (
                AgentRunState::Cancelled,
                AgentFailure::Cancelled,
                SessionEventKind::AgentCanceled,
            ),
            AgentLoopFailure::Session => (
                AgentRunState::Failed,
                AgentFailure::Session,
                SessionEventKind::System,
            ),
            AgentLoopFailure::Context => (
                AgentRunState::Failed,
                AgentFailure::Context,
                SessionEventKind::System,
            ),
            AgentLoopFailure::Model => (
                AgentRunState::Failed,
                AgentFailure::Model,
                SessionEventKind::System,
            ),
            AgentLoopFailure::ToolRecoveryRequired => (
                AgentRunState::Failed,
                AgentFailure::ToolRecoveryRequired,
                SessionEventKind::System,
            ),
            _ => (
                AgentRunState::Failed,
                AgentFailure::Model,
                SessionEventKind::System,
            ),
        };
        let _ = session.append(event, json!({"iteration": iteration}));
        let _ = self.notify(session_id, state, iteration, Value::Null).await;
        if let Some(runtime) = &self.interrupt {
            runtime.clear(session_id);
        }
        if let Some(callbacks) = &self.callbacks {
            callbacks.clear_checkpoint_control(session_id);
        }
        Self::result_for(
            session_id,
            state,
            Some(failure),
            Some(error.message),
            None,
            iteration,
            tool_calls,
        )
    }
    fn pending_tool_calls(session: &dyn SessionLog) -> Result<Vec<ToolCall>, AgentLoopError> {
        let mut pending = Vec::new();
        let mut completed = Vec::new();
        let mut streamed: HashMap<String, Vec<(String, String, String)>> = HashMap::new();
        for event in session.try_events().map_err(|error| {
            AgentLoopError::new(
                AgentLoopFailure::Session,
                format!("read session during recovery failed: {error}"),
            )
        })? {
            match event.kind {
                SessionEventKind::Assistant => {
                    if let Some(calls) = event.payload.get("tool_calls").and_then(Value::as_array) {
                        for call in calls {
                            let call = serde_json::from_value::<ToolCall>(call.clone()).map_err(
                                |error| {
                                    AgentLoopError::new(
                                        AgentLoopFailure::Session,
                                        format!(
                                            "invalid pending tool call during recovery: {error}"
                                        ),
                                    )
                                },
                            )?;
                            if !pending.iter().any(|item: &ToolCall| item.id == call.id) {
                                pending.push(call);
                            }
                        }
                    }
                }
                SessionEventKind::ToolResult => {
                    if let Some(id) = event.payload.get("tool_call_id").and_then(Value::as_str) {
                        completed.push(id.to_string());
                        pending.retain(|call| call.id != id);
                    }
                }
                SessionEventKind::System
                    if event.payload.get("event").and_then(Value::as_str)
                        == Some("assistant_stream_delta") =>
                {
                    let Some(stream_id) = event.payload.get("stream_id").and_then(Value::as_str)
                    else {
                        continue;
                    };
                    let Some(deltas) = event
                        .payload
                        .get("tool_call_deltas")
                        .and_then(Value::as_array)
                    else {
                        continue;
                    };
                    let calls = streamed.entry(stream_id.to_string()).or_default();
                    for delta in deltas {
                        let index = delta.get("index").and_then(Value::as_u64).ok_or_else(|| {
                            AgentLoopError::new(
                                AgentLoopFailure::ToolRecoveryRequired,
                                "stream tool call delta has no index",
                            )
                        })? as usize;
                        while calls.len() <= index {
                            calls.push((String::new(), String::new(), String::new()));
                        }
                        let call = &mut calls[index];
                        if let Some(id) = delta.get("id").and_then(Value::as_str) {
                            call.0 = id.to_string();
                        }
                        if let Some(name) = delta.get("name").and_then(Value::as_str) {
                            call.1 = name.to_string();
                        }
                        if let Some(arguments) = delta.get("arguments").and_then(Value::as_str) {
                            call.2.push_str(arguments);
                        }
                    }
                }
                _ => {}
            }
        }
        for calls in streamed.into_values() {
            for (id, name, arguments) in calls {
                if id.trim().is_empty() && name.trim().is_empty() {
                    continue;
                }
                if id.trim().is_empty() || name.trim().is_empty() {
                    return Err(AgentLoopError::new(
                        AgentLoopFailure::ToolRecoveryRequired,
                        "stream tool call identity is incomplete; manual recovery required",
                    ));
                }
                let arguments = serde_json::from_str(&arguments).map_err(|error| {
                    AgentLoopError::new(
                        AgentLoopFailure::ToolRecoveryRequired,
                        format!("stream tool call arguments are incomplete: {error}"),
                    )
                })?;
                if !completed.iter().any(|item| item == &id)
                    && !pending.iter().any(|item: &ToolCall| item.id == id)
                {
                    pending.push(ToolCall {
                        id,
                        name,
                        arguments,
                    });
                }
            }
        }
        Ok(pending)
    }

    /// Complete tool calls persisted before a process crash.
    async fn recover_pending_tool_calls(
        &self,
        session: &Arc<dyn SessionLog>,
        session_id: &str,
        deadline: Option<tokio::time::Instant>,
    ) -> Result<usize, AgentLoopError> {
        let pending = Self::pending_tool_calls(session.as_ref())?;
        let owner = format!(
            "{}:{}",
            std::process::id(),
            RECOVERY_OWNER_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let mut recovered = 0;
        for call in pending {
            let claimed = session
                .claim_tool_call(&call.id, &owner, 300_000)
                .map_err(|error| {
                    AgentLoopError::new(
                        AgentLoopFailure::Session,
                        format!("claim pending tool call failed: {error}"),
                    )
                })?;
            if !claimed {
                return Err(AgentLoopError::new(
                    AgentLoopFailure::ToolRecoveryRequired,
                    format!(
                        "pending tool call {} is already claimed or completed; retry after the other recovery worker finishes",
                        call.id
                    ),
                ));
            }
            let tool = self.tools.get(&call.name).ok_or_else(|| {
                AgentLoopError::new(
                    AgentLoopFailure::ToolRecoveryRequired,
                    format!(
                        "tool execution outcome unknown after crash: tool {} is unavailable; manual retry required",
                        call.name
                    ),
                )
            })?;
            if !tool.idempotent() {
                let message = format!(
                    "tool execution outcome unknown after crash: {} is non-idempotent; manual retry required",
                    call.name
                );
                session
                    .append(
                        SessionEventKind::ToolResult,
                        json!({
                            "tool_call_id": call.id,
                            "status": "unknown",
                            "output": message,
                        }),
                    )
                    .map_err(|error| {
                        AgentLoopError::new(
                            AgentLoopFailure::Session,
                            format!("persist unknown tool result failed: {error}"),
                        )
                    })?;
                return Err(AgentLoopError::new(
                    AgentLoopFailure::ToolRecoveryRequired,
                    message,
                ));
            }

            let invoke = self
                .tools
                .invoke_with_id(&call.name, &call.id, call.arguments.clone());
            let (status, output) = match self.race_control(session_id, deadline, invoke).await? {
                Ok(value) => ("completed", value.to_string()),
                Err(error) => ("error", format!("tool error: {error}")),
            };
            session
                .append(
                    SessionEventKind::ToolResult,
                    json!({
                        "tool_call_id": call.id,
                        "status": status,
                        "output": output,
                    }),
                )
                .map_err(|error| {
                    AgentLoopError::new(
                        AgentLoopFailure::Session,
                        format!("persist recovered tool result failed: {error}"),
                    )
                })?;
            recovered += 1;
        }
        Ok(recovered)
    }
    // The stream path carries request/session/attempt state together so every
    // delta can be persisted and raced against the same deadline.
    #[allow(clippy::too_many_arguments)]
    async fn stream_response(
        &self,
        llm: Arc<dyn ModelProvider>,
        request: ModelRequest,
        session: &Arc<dyn SessionLog>,
        session_id: &str,
        iteration: usize,
        attempt: u32,
        deadline: Option<tokio::time::Instant>,
    ) -> Result<ModelResponse, AgentLoopError> {
        struct PartialCall {
            id: String,
            name: String,
            arguments: String,
        }

        let (sender, mut receiver) = tokio::sync::mpsc::channel(64);
        let backup = self.backup.clone();
        let policy = self.backup_policy;
        let mut producer = tokio::spawn(async move {
            if let Some(backup) = backup {
                backup
                    .stream_chat_with_policy(llm.as_ref(), request, sender, policy)
                    .await
                    .map_err(|error| format!("model backup failed: {error}"))
            } else {
                llm.stream_chat(request, sender)
                    .await
                    .map_err(|error| format!("model error: {error}"))
            }
        });

        let mut producer_result: Option<Result<(), String>> = None;
        let mut receiver_closed = false;
        let mut content = String::new();
        let mut reasoning = String::new();
        let rails = self.rails.clone();
        let mut calls: Vec<PartialCall> = Vec::new();
        let stream_id = format!(
            "{}:{}:{}",
            session_id,
            iteration,
            RECOVERY_OWNER_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let mut consume_chunk = |chunk: ah_contracts::llm::ModelChunk| {
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
                session
                    .append(
                        SessionEventKind::System,
                        json!({
                            "event": "assistant_stream_delta",
                            "stream_id": stream_id,
                            "content_delta": chunk.content_delta.clone(),
                            "reasoning_delta": chunk.reasoning_delta.clone(),
                            "tool_call_deltas": deltas,
                        }),
                    )
                    .map_err(|error| {
                        AgentLoopError::new(
                            AgentLoopFailure::Session,
                            format!("persist stream delta failed: {error}"),
                        )
                    })?;
            }
            content.push_str(&chunk.content_delta);
            if let Some(rails) = rails.as_ref() {
                for (field, delta) in [
                    ("content", chunk.content_delta.as_str()),
                    ("reasoning_content", chunk.reasoning_delta.as_str()),
                ] {
                    if delta.is_empty() {
                        continue;
                    }
                    let mut rail_input = RailInput::new(session_id, RailPhase::ModelChunk);
                    rail_input.iteration = iteration;
                    rail_input.attempt = attempt;
                    rail_input.stream_field = Some(field.to_string());
                    rail_input.stream_delta = Some(delta.to_string());
                    let decision = rails.evaluate(rail_input);
                    if matches!(
                        decision.action,
                        RailAction::Retry | RailAction::Deny | RailAction::Stop
                    ) {
                        return Err(AgentLoopError::new(
                            AgentLoopFailure::Model,
                            decision
                                .reason
                                .unwrap_or_else(|| "model stream rail rejected output".to_string()),
                        ));
                    }
                }
            }
            reasoning.push_str(&chunk.reasoning_delta);
            for delta in chunk.tool_call_deltas {
                while calls.len() <= delta.index {
                    calls.push(PartialCall {
                        id: String::new(),
                        name: String::new(),
                        arguments: String::new(),
                    });
                }
                let call = &mut calls[delta.index];
                if let Some(id) = delta.id {
                    call.id = id;
                }
                if let Some(name) = delta.name {
                    call.name = name;
                }
                call.arguments.push_str(&delta.arguments);
            }
            Ok::<(), AgentLoopError>(())
        };

        let mut control_future = Box::pin(self.poll_control(session_id, deadline));
        loop {
            if let Some(result) = producer_result.as_ref() {
                if let Err(error) = result {
                    return Err(AgentLoopError::new(AgentLoopFailure::Model, error.clone()));
                }
                match receiver.recv().await {
                    Some(chunk) => {
                        if let Err(error) = consume_chunk(chunk) {
                            producer.abort();
                            return Err(error);
                        }
                    }
                    None => break,
                }
                continue;
            }
            tokio::select! {
                biased;
                _ = &mut control_future => {
                    producer.abort();
                    if deadline.is_some_and(|dl| tokio::time::Instant::now() >= dl) {
                        return Err(AgentLoopError::new(AgentLoopFailure::TimedOut, "agent run timed out"));
                    }
                    return match self.control(session_id) {
                        AgentControl::Interrupt => Err(AgentLoopError::new(
                            AgentLoopFailure::Interrupted,
                            "agent run interrupted; resume the session to continue",
                        )),
                        AgentControl::Cancel => Err(AgentLoopError::new(
                            AgentLoopFailure::Cancelled,
                            "agent run cancelled",
                        )),
                        AgentControl::Continue => Err(AgentLoopError::new(
                            AgentLoopFailure::Model,
                            "control raced unexpectedly",
                        )),
                    };
                }
                result = &mut producer => {
                    producer_result = Some(match result {
                        Ok(result) => result,
                        Err(error) => Err(format!("stream task failed: {error}")),
                    });
                }
                chunk = receiver.recv(), if !receiver_closed => {
                    match chunk {
                        Some(chunk) => {
                            if let Err(error) = consume_chunk(chunk) {
                                producer.abort();
                                return Err(error);
                            }
                        },
                        None => receiver_closed = true,
                    }
                }
            }
        }

        if let Some(Err(error)) = producer_result {
            return Err(AgentLoopError::new(AgentLoopFailure::Model, error));
        }
        let tool_calls = calls
            .into_iter()
            .map(|call| {
                if call.id.trim().is_empty() || call.name.trim().is_empty() {
                    return Err(AgentLoopError::new(
                        AgentLoopFailure::Model,
                        "stream ended with incomplete tool call identity",
                    ));
                }
                let arguments = serde_json::from_str(&call.arguments).map_err(|error| {
                    AgentLoopError::new(
                        AgentLoopFailure::Model,
                        format!("stream ended with incomplete tool call arguments: {error}"),
                    )
                })?;
                Ok(ToolCall {
                    id: call.id,
                    name: call.name,
                    arguments,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ModelResponse {
            content,
            tool_calls,
            reasoning_content: (!reasoning.is_empty()).then_some(reasoning),
        })
    }

    /// 在指定会话上运行一轮任务:日志驱动的 ReAct 循环,返回结构化 AgentResult。
    ///
    /// - 会话可为 resume(已有历史)或 fork 出的新会话;历史经日志投影自动保留。
    /// - 终止原因结构化(P1-02):state + failure 字段,消费方无需解析错误字符串。
    /// - 执行中取消/超时(P1-03):模型调用与工具执行期间轮询控制状态与截止时间,
    ///   命中即中止在途 future(跨 provider:backup 链上的任意 provider 同被中止)。
    pub async fn run_in_session(&self, session: Arc<dyn SessionLog>, input: &str) -> AgentResult {
        let session_id = session.id().to_string();
        let deadline = self
            .timeout_ms
            .map(|ms| tokio::time::Instant::now() + std::time::Duration::from_millis(ms));
        let mut total_tool_calls: usize = 0;

        if input.trim().is_empty() {
            return Self::result_for(
                &session_id,
                AgentRunState::Failed,
                Some(AgentFailure::InvalidInput),
                Some("input must not be empty".to_string()),
                None,
                0,
                total_tool_calls,
            );
        }
        if let Some(rails) = &self.rails {
            rails.reset(&session_id);
        }
        if self
            .notify(
                &session_id,
                AgentRunState::Running,
                0,
                json!({"input": input}),
            )
            .await
            .is_err()
        {
            return Self::result_for(
                &session_id,
                AgentRunState::Failed,
                Some(AgentFailure::Context),
                Some("callback notify failed".to_string()),
                None,
                0,
                total_tool_calls,
            );
        }
        // 恢复顺序必须先补齐历史 tool call,再追加本次用户消息,否则模型会看到
        // assistant tool call 后紧跟 user 的非法对话序列。
        match self
            .recover_pending_tool_calls(&session, &session_id, deadline)
            .await
        {
            Ok(count) => total_tool_calls += count,
            Err(error) => {
                return self
                    .race_finish(&session, &session_id, 0, error, total_tool_calls)
                    .await;
            }
        }
        // 1) 用户消息入日志。
        if session
            .append(SessionEventKind::User, json!({ "content": input }))
            .is_err()
        {
            return Self::result_for(
                &session_id,
                AgentRunState::Failed,
                Some(AgentFailure::Session),
                Some("session append failed".to_string()),
                None,
                0,
                total_tool_calls,
            );
        }

        for iteration in 0..self.max_iterations {
            // 快速路径:轮次边界检查(与执行中 race 互补)。
            if deadline.is_some_and(|dl| tokio::time::Instant::now() >= dl) {
                return self
                    .race_finish(
                        &session,
                        &session_id,
                        iteration,
                        AgentLoopError::new(AgentLoopFailure::TimedOut, "agent run timed out"),
                        total_tool_calls,
                    )
                    .await;
            }
            match self.control(&session_id) {
                AgentControl::Interrupt => {
                    return self
                        .race_finish(
                            &session,
                            &session_id,
                            iteration,
                            AgentLoopError::new(
                                AgentLoopFailure::Interrupted,
                                "agent run interrupted; resume the session to continue",
                            ),
                            total_tool_calls,
                        )
                        .await;
                }
                AgentControl::Cancel => {
                    return self
                        .race_finish(
                            &session,
                            &session_id,
                            iteration,
                            AgentLoopError::new(AgentLoopFailure::Cancelled, "agent run cancelled"),
                            total_tool_calls,
                        )
                        .await;
                }
                AgentControl::Continue => {}
            }
            if let Some(rails) = &self.rails {
                let mut rail_input = RailInput::new(&session_id, RailPhase::BeforeIteration);
                rail_input.iteration = iteration;
                let decision = rails.evaluate(rail_input);
                if matches!(decision.action, RailAction::Stop | RailAction::Deny) {
                    return Self::result_for(
                        &session_id,
                        AgentRunState::Failed,
                        Some(AgentFailure::IterationLimit),
                        decision.reason,
                        None,
                        iteration,
                        total_tool_calls,
                    );
                }
            }
            // 2) 模型可见消息:优先经 context seam 按预算组装(压缩时注入摘要),
            //    未挂载时用日志投影直通(日志即真相:完整历史始终在日志)。
            let mut messages = if let Some(context) = &self.context {
                match context.assemble(session.as_ref(), self.token_budget).await {
                    Ok(assembled) => assembled.messages,
                    Err(e) => {
                        return Self::result_for(
                            &session_id,
                            AgentRunState::Failed,
                            Some(AgentFailure::Context),
                            Some(format!("context assemble failed: {e}")),
                            None,
                            iteration,
                            total_tool_calls,
                        );
                    }
                }
            } else {
                session.derive_messages()
            };
            if let Some(context) = &self.system_context {
                messages.insert(0, ChatMessage::new(ChatRole::System, context.clone()));
            }
            // prompt seam 消费:注册的模板渲染为 system 消息(请求首条)。
            if let Some(registry) = &self.prompt
                && registry.get(&self.prompt_name).is_some()
                && let Ok(rendered) = registry.render(&self.prompt_name, &HashMap::new())
            {
                messages.insert(0, ChatMessage::new(ChatRole::System, rendered.content));
            }
            if let Some(rails) = &self.rails
                && let Some(planning_prompt) = rails.config().planning_prompt
            {
                messages.insert(0, ChatMessage::new(ChatRole::System, planning_prompt));
            }
            let request = ModelRequest {
                messages,
                tools: self.tool_schemas(),
                model: self.request_model.clone(),
                temperature: self.request_temperature,
            };
            let active_llm = match self.resolve_request_model() {
                Ok(provider) => provider,
                Err(error) => {
                    return self
                        .race_finish(&session, &session_id, iteration, error, total_tool_calls)
                        .await;
                }
            };
            let mut model_attempt = 0_u32;
            let response = loop {
                if let Some(rails) = &self.rails {
                    let mut rail_input = RailInput::new(&session_id, RailPhase::BeforeModelCall);
                    rail_input.iteration = iteration;
                    rail_input.attempt = model_attempt;
                    let decision = rails.evaluate(rail_input);
                    if decision.action == RailAction::Retry {
                        if let Some(delay) = decision.retry_after_ms {
                            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        }
                        continue;
                    }
                    if matches!(decision.action, RailAction::Stop | RailAction::Deny) {
                        let error = AgentLoopError::new(
                            AgentLoopFailure::Model,
                            decision
                                .reason
                                .unwrap_or_else(|| "model rail rejected call".to_string()),
                        );
                        return self
                            .race_finish(&session, &session_id, iteration, error, total_tool_calls)
                            .await;
                    }
                }
                match self
                    .stream_response(
                        active_llm.clone(),
                        request.clone(),
                        &session,
                        &session_id,
                        iteration,
                        model_attempt,
                        deadline,
                    )
                    .await
                {
                    Ok(response) => {
                        match self
                            .notify_checkpoint(
                                &session_id,
                                iteration,
                                "after_model_call",
                                serde_json::json!({
                                    "model": active_llm.name(),
                                    "content": response.content.clone(),
                                    "tool_calls": response.tool_calls.len(),
                                }),
                            )
                            .await
                        {
                            Ok(()) => break response,
                            Err(error) => {
                                return self
                                    .race_finish(
                                        &session,
                                        &session_id,
                                        iteration,
                                        error,
                                        total_tool_calls,
                                    )
                                    .await;
                            }
                        }
                    }
                    Err(error)
                        if matches!(
                            error.kind,
                            AgentLoopFailure::TimedOut
                                | AgentLoopFailure::Interrupted
                                | AgentLoopFailure::Cancelled
                        ) =>
                    {
                        return self
                            .race_finish(&session, &session_id, iteration, error, total_tool_calls)
                            .await;
                    }
                    Err(error) => {
                        let decision = if let Some(rails) = &self.rails {
                            let mut rail_input = RailInput::new(&session_id, RailPhase::ModelError);
                            rail_input.iteration = iteration;
                            rail_input.attempt = model_attempt;
                            rail_input.error = Some(error.message.clone());
                            rails.evaluate(rail_input)
                        } else {
                            ah_contracts::rails::RailDecision::deny(error.message.clone())
                        };
                        if decision.action == RailAction::Retry {
                            model_attempt += 1;
                            if let Some(delay) = decision.retry_after_ms {
                                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                            }
                            continue;
                        }
                        let failure = match error.kind {
                            AgentLoopFailure::Session => AgentFailure::Session,
                            AgentLoopFailure::Context => AgentFailure::Context,
                            AgentLoopFailure::ToolRecoveryRequired => {
                                AgentFailure::ToolRecoveryRequired
                            }
                            _ => AgentFailure::Model,
                        };
                        let message = error.message;
                        let _ = session.append(
                            SessionEventKind::System,
                            json!({
                                "event": "agent_error",
                                "failure": format!("{failure:?}"),
                                "message": message,
                            }),
                        );
                        return Self::result_for(
                            &session_id,
                            AgentRunState::Failed,
                            Some(failure),
                            Some(message),
                            None,
                            iteration,
                            total_tool_calls,
                        );
                    }
                }
            };

            if response.tool_calls.is_empty() {
                let completion_decision = if let Some(rails) = &self.rails {
                    let mut rail_input = RailInput::new(&session_id, RailPhase::ModelOutput);
                    rail_input.iteration = iteration;
                    rail_input.output = Some(response.content.clone());
                    rails.evaluate(rail_input)
                } else {
                    ah_contracts::rails::RailDecision::continue_()
                };
                let mut assistant_payload = json!({ "content": response.content });
                if let Some(reasoning) = response.reasoning_content.clone() {
                    assistant_payload["reasoning_content"] = Value::String(reasoning);
                }
                if session
                    .append(SessionEventKind::Assistant, assistant_payload)
                    .is_err()
                {
                    return Self::result_for(
                        &session_id,
                        AgentRunState::Failed,
                        Some(AgentFailure::Session),
                        Some("session append failed".to_string()),
                        None,
                        iteration,
                        total_tool_calls,
                    );
                }
                if completion_decision.action == RailAction::Deny {
                    return Self::result_for(
                        &session_id,
                        AgentRunState::Failed,
                        Some(AgentFailure::IterationLimit),
                        completion_decision.reason,
                        None,
                        iteration,
                        total_tool_calls,
                    );
                }
                if self
                    .rails
                    .as_ref()
                    .is_some_and(|rails| rails.config().completion_promise.is_some())
                    && completion_decision.action != RailAction::Stop
                {
                    self.record_step(&session, iteration, 0, false);
                    continue;
                }
                self.record_step(&session, iteration, 0, true);
                let _ = self
                    .notify(
                        &session_id,
                        AgentRunState::Completed,
                        iteration,
                        json!({"answer": response.content}),
                    )
                    .await;
                if let Some(runtime) = &self.interrupt {
                    runtime.clear(&session_id);
                }
                if let Some(callbacks) = &self.callbacks {
                    callbacks.clear_checkpoint_control(&session_id);
                }
                return Self::result_for(
                    &session_id,
                    AgentRunState::Completed,
                    None,
                    None,
                    Some(response.content),
                    iteration,
                    total_tool_calls,
                );
            }

            // 5) 助手工具调用入日志,同时保存 thinking-mode 所需的 reasoning_content。
            let calls: Vec<Value> = response
                .tool_calls
                .iter()
                .map(|call| {
                    json!({
                        "id": call.id,
                        "name": call.name,
                        "arguments": call.arguments,
                    })
                })
                .collect();
            let mut assistant_payload = json!({ "tool_calls": calls });
            if let Some(reasoning) = response.reasoning_content.clone() {
                assistant_payload["reasoning_content"] = Value::String(reasoning);
            }
            if session
                .append(SessionEventKind::Assistant, assistant_payload)
                .is_err()
            {
                return Self::result_for(
                    &session_id,
                    AgentRunState::Failed,
                    Some(AgentFailure::Session),
                    Some("session append failed".to_string()),
                    None,
                    iteration,
                    total_tool_calls,
                );
            }

            // 6) 真实执行工具。工具失败/被拒**不中断循环**,错误作为工具结果回喂模型
            //    (模型据此换方案),符合真实 agent 语义;执行中取消/超时则中止循环。
            for call in &response.tool_calls {
                if let Err(error) = self
                    .notify_checkpoint(
                        &session_id,
                        iteration,
                        "before_tool_call",
                        serde_json::json!({
                            "tool_name": call.name,
                            "tool_call_id": call.id,
                        }),
                    )
                    .await
                {
                    return self
                        .race_finish(&session, &session_id, iteration, error, total_tool_calls)
                        .await;
                }
                let tool_idempotent = self
                    .tools
                    .get(&call.name)
                    .is_some_and(|tool| tool.idempotent());
                let mut tool_attempt = 0_u32;
                let (status, output) = loop {
                    let invoke =
                        self.tools
                            .invoke_with_id(&call.name, &call.id, call.arguments.clone());
                    match self.race_control(&session_id, deadline, invoke).await {
                        Ok(Ok(value)) => {
                            total_tool_calls += 1;
                            break ("completed", value.to_string());
                        }
                        Ok(Err(error)) => {
                            total_tool_calls += 1;
                            let message = format!("tool error: {error}");
                            let decision = if let Some(rails) = &self.rails {
                                let mut rail_input =
                                    RailInput::new(&session_id, RailPhase::ToolError);
                                rail_input.iteration = iteration;
                                rail_input.attempt = tool_attempt;
                                rail_input.error = Some(message.clone());
                                rail_input.tool_name = Some(call.name.clone());
                                rail_input.tool_idempotent = tool_idempotent;
                                rails.evaluate(rail_input)
                            } else {
                                ah_contracts::rails::RailDecision::deny(message.clone())
                            };
                            if decision.action == RailAction::Retry {
                                tool_attempt += 1;
                                if let Some(delay) = decision.retry_after_ms {
                                    tokio::time::sleep(std::time::Duration::from_millis(delay))
                                        .await;
                                }
                                continue;
                            }
                            break ("error", message);
                        }
                        Err(error) => {
                            return self
                                .race_finish(
                                    &session,
                                    &session_id,
                                    iteration,
                                    error,
                                    total_tool_calls,
                                )
                                .await;
                        }
                    }
                };
                if session
                    .append(
                        SessionEventKind::ToolResult,
                        json!({
                            "tool_call_id": call.id,
                            "status": status,
                            "output": output,
                        }),
                    )
                    .is_err()
                {
                    return Self::result_for(
                        &session_id,
                        AgentRunState::Failed,
                        Some(AgentFailure::Session),
                        Some("session append failed".to_string()),
                        None,
                        iteration,
                        total_tool_calls,
                    );
                }
            }

            self.record_step(&session, iteration, response.tool_calls.len(), false);
        }
        let _ = self
            .notify(
                &session_id,
                AgentRunState::Failed,
                self.max_iterations,
                Value::Null,
            )
            .await;
        if let Some(callbacks) = &self.callbacks {
            callbacks.clear_checkpoint_control(&session_id);
        }
        Self::result_for(
            &session_id,
            AgentRunState::Failed,
            Some(AgentFailure::IterationLimit),
            Some(format!(
                "max iterations exceeded: {max}",
                max = self.max_iterations
            )),
            None,
            self.max_iterations,
            total_tool_calls,
        )
    }
}

impl Seam for AgentLoop {}

#[async_trait]
impl ah_contracts::agent::AgentLoopRuntime for AgentLoop {
    fn card(&self) -> ah_contracts::agent::AgentCard {
        ah_contracts::agent::AgentCard {
            id: "agent-loop".into(),
            name: "ReAct Agent Loop".into(),
            description: "A session-backed agent loop with tool execution and recovery controls."
                .into(),
            capabilities: vec![
                "chat".into(),
                "tool-use".into(),
                "interrupt".into(),
                "cancel".into(),
                "timeout".into(),
                "session-recovery".into(),
            ],
        }
    }

    async fn run(&self, input: &str) -> ah_contracts::agent::AgentResult {
        AgentLoop::run(self, input).await
    }

    async fn run_in_session(
        &self,
        session: Arc<dyn SessionLog>,
        input: &str,
    ) -> ah_contracts::agent::AgentResult {
        AgentLoop::run_in_session(self, session, input).await
    }

    async fn run_in_session_with_timeout(
        &self,
        session: Arc<dyn SessionLog>,
        input: &str,
        timeout_ms: Option<u64>,
    ) -> ah_contracts::agent::AgentResult {
        AgentLoop::with_timeout(self.clone(), timeout_ms)
            .run_in_session(session, input)
            .await
    }
    async fn run_in_session_with_config(
        &self,
        session: Arc<dyn SessionLog>,
        input: &str,
        config: ah_contracts::agent::AgentRunConfig,
    ) -> ah_contracts::agent::AgentResult {
        AgentLoop::with_request_config(self.clone(), &config)
            .run_in_session(session, input)
            .await
    }
}

/// agent 循环插件:注入 llm + tools + sessions,提供 agent-loop 服务。
pub struct AgentLoopPlugin {
    max_iterations: usize,
    timeout_ms: Option<u64>,
}

impl AgentLoopPlugin {
    /// 创建循环插件。
    pub fn new(max_iterations: usize) -> Self {
        Self {
            max_iterations,
            timeout_ms: Some(120_000),
        }
    }

    /// 覆盖插件运行级截止时间;None 表示不设置截止时间。
    pub fn with_timeout(mut self, timeout_ms: Option<u64>) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }
}

impl Default for AgentLoopPlugin {
    fn default() -> Self {
        Self::new(8)
    }
}

impl Plugin for AgentLoopPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-agent-loop"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![AGENT_LOOP]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![
            LLM,
            TOOLS,
            SESSIONS,
            INTERRUPT,
            AGENT_CALLBACKS,
            MODEL_BACKUP,
        ]
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
        // context seam 可选:挂载后循环按 token 预算压缩模型可见上下文。
        let context = ctx.service::<dyn ContextEngine>(&CONTEXT);
        let interrupt = ctx.service::<dyn InterruptRuntime>(&INTERRUPT);
        let callbacks = ctx.service::<dyn AgentCallbackManager>(&AGENT_CALLBACKS);
        let rails = ctx.service::<dyn RailRuntime>(&RAILS);
        let backup = ctx.service::<dyn ModelBackup>(&MODEL_BACKUP);
        let backup_policy = ctx
            .service::<dyn ModelBackupPolicyProvider>(&MODEL_BACKUP_POLICY)
            .map(|provider| provider.policy())
            .unwrap_or_default();
        // prompt seam 可选:注册 "agent" 模板时注入系统提示。
        let prompt = ctx.service::<dyn PromptRegistry>(&PROMPT);
        let model_catalog = ctx.service::<dyn ModelProviderCatalog>(&MODEL_PROVIDER_CATALOG);
        let mut agent = AgentLoop::new(llm, tools, sessions, ctx.clone(), self.max_iterations);
        if let Some(context) = context {
            agent = agent.with_context(context, 8192);
        }
        if let Some(prompt) = prompt {
            agent = agent.with_prompt(prompt, "agent");
        }
        if let Some(model_catalog) = model_catalog {
            agent = agent.with_model_catalog(model_catalog);
        }
        agent = agent
            .with_controls(interrupt, callbacks, None)
            .with_timeout(self.timeout_ms)
            .with_model_backup(backup)
            .with_rails(rails)
            .with_model_backup_policy(backup_policy);
        let agent: Arc<dyn ah_contracts::agent::AgentLoopRuntime> = Arc::new(agent);
        Ok(vec![ctx.register(AGENT_LOOP, agent)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::SESSIONS;
    use ah_contracts::llm::{ChatRole, ModelChunk, ModelError, ModelResponse, ToolCallDelta};
    use ah_contracts::session::SessionLog;
    use ah_hub::plugin::DynPlugin;
    use async_trait::async_trait;
    use std::sync::Arc as StdArc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// 组装 dev 式组合:mock llm(桩)+ 真实工具 + 真实 sysop + 会话日志 + 循环。
    /// 会话目录由 session_path 名称派生,保证每个测试独立、可重复。
    fn build_ctx(root: &std::path::Path, session_path: &std::path::Path) -> (Context, Vec<Effect>) {
        let session_dir = session_path.parent().unwrap().join(format!(
            "{}-dir",
            session_path.file_name().unwrap().to_string_lossy()
        ));
        let _ = std::fs::remove_dir_all(&session_dir);
        build_ctx_with_dir(root, &session_dir)
    }

    /// 同上,但指定会话目录(供多会话测试)。
    fn build_ctx_with_dir(
        root: &std::path::Path,
        session_dir: &std::path::Path,
    ) -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let default_path = session_dir.join("default.jsonl");
        let plugins: Vec<DynPlugin> = vec![
            StdArc::new(ah_plugins_mock::MockPlugin),
            StdArc::new(ah_plugins_agent_control::AgentControlPlugin),
            StdArc::new(ah_plugins_model_backup::ModelBackupPlugin::new(Vec::new())),
            StdArc::new(ah_plugins_tools::ToolsPlugin),
            StdArc::new(ah_plugins_sysop::SysopPlugin::new(root)),
            StdArc::new(ah_plugins_session_log::SessionLogPlugin::new(
                &default_path,
                session_dir,
            )),
            StdArc::new(AgentLoopPlugin::default()),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[tokio::test]
    async fn react_loop_drives_real_tools_and_logs_everything() {
        let root = std::env::temp_dir().join(format!("ah-loop-{}", std::process::id()));
        let session_path =
            std::env::temp_dir().join(format!("ah-loop-session-{}", std::process::id()));
        let _ = std::fs::remove_file(&session_path);
        let (ctx, effects) = build_ctx(&root, &session_path);

        // 真实写入一个文件,让循环调用的 list_dir 能列出它。
        let fs = ctx
            .service::<dyn ah_contracts::fs::FsProvider>(&ah_contracts::keys::FS)
            .expect("fs");
        fs.write("probe.txt", b"x").expect("write");

        let agent = ctx
            .service::<dyn ah_contracts::agent::AgentLoopRuntime>(&AGENT_LOOP)
            .expect("agent-loop service");
        let result = agent.run("explore the workspace").await;
        assert_eq!(result.state, AgentRunState::Completed);
        let answer = result.answer.unwrap();

        assert!(answer.contains("mock final answer"));
        assert!(answer.contains("probe.txt"));
        assert!(result.iterations >= 1);
        assert!(result.tool_calls >= 1, "list_dir 已真实执行");

        // 会话日志:从文件重开,事件完整、消息可投影。
        let sessions = ctx.service::<dyn SessionLog>(&SESSIONS).expect("sessions");
        let events = sessions.events();
        let kinds: Vec<_> = events.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                ah_contracts::session::SessionEventKind::User,
                ah_contracts::session::SessionEventKind::System,
                ah_contracts::session::SessionEventKind::Assistant,
                ah_contracts::session::SessionEventKind::ToolResult,
                ah_contracts::session::SessionEventKind::AgentStep,
                ah_contracts::session::SessionEventKind::System,
                ah_contracts::session::SessionEventKind::Assistant,
                ah_contracts::session::SessionEventKind::AgentStep,
            ]
        );
        let messages = sessions.derive_messages();
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[2].role, ChatRole::Tool);

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&session_path);
    }

    #[tokio::test]
    async fn tool_error_is_fed_back_to_model_and_loop_completes() {
        use ah_contracts::tools::{Tool, ToolError};
        use serde_json::Value;

        struct FailingListDir;

        #[async_trait::async_trait]
        impl Tool for FailingListDir {
            fn name(&self) -> &'static str {
                "list_dir"
            }

            fn description(&self) -> &'static str {
                "always fails"
            }

            async fn invoke(&self, _arguments: Value) -> Result<Value, ToolError> {
                Err(ToolError("boom: directory unreadable".to_string()))
            }
        }

        let root = std::env::temp_dir().join(format!("ah-loop-fail-{}", std::process::id()));
        let session_path =
            std::env::temp_dir().join(format!("ah-loop-fail-session-{}", std::process::id()));
        let _ = std::fs::remove_file(&session_path);
        let (ctx, effects) = build_ctx(&root, &session_path);

        // 用失败工具覆盖 list_dir:mock 模型会调它,应得到错误回喂而非循环崩溃。
        let registry = ctx
            .service::<dyn ah_contracts::tools::ToolRegistry>(&ah_contracts::keys::TOOLS)
            .expect("tools");
        let _override = registry.register(StdArc::new(FailingListDir));

        let agent = ctx
            .service::<dyn ah_contracts::agent::AgentLoopRuntime>(&AGENT_LOOP)
            .expect("agent-loop service");
        let result = agent.run("go").await;
        assert_eq!(result.state, AgentRunState::Completed);
        let answer = result.answer.unwrap_or_default();

        // 错误被回喂:最终回答引用了工具错误文本。
        assert!(answer.contains("mock final answer"));
        assert!(answer.contains("boom: directory unreadable"));

        // 日志中 ToolResult 记录的是错误文本(而非崩溃)。
        let sessions = ctx.service::<dyn SessionLog>(&SESSIONS).expect("sessions");
        let events = sessions.events();
        let tool_results: Vec<_> = events
            .iter()
            .filter(|e| e.kind == ah_contracts::session::SessionEventKind::ToolResult)
            .collect();
        assert_eq!(tool_results.len(), 1);
        assert_eq!(tool_results[0].payload["status"], "error");
        assert!(
            tool_results[0].payload["output"]
                .as_str()
                .unwrap_or_default()
                .contains("boom")
        );

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&session_path);
    }

    #[tokio::test]
    async fn idempotent_tool_error_is_retried_and_loop_completes() {
        use ah_contracts::tools::{Tool, ToolError};

        struct FlakyTool {
            calls: StdArc<AtomicUsize>,
        }

        #[async_trait]
        impl Tool for FlakyTool {
            fn name(&self) -> &'static str {
                "flaky_tool"
            }

            fn description(&self) -> &'static str {
                "fails once, then succeeds"
            }

            fn idempotent(&self) -> bool {
                true
            }

            async fn invoke(&self, _arguments: Value) -> Result<Value, ToolError> {
                if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err(ToolError("connection reset by peer".into()))
                } else {
                    Ok(json!({"ok": true}))
                }
            }
        }

        struct ToolRetryProvider {
            calls: AtomicUsize,
        }

        impl Seam for ToolRetryProvider {}

        #[async_trait]
        impl ModelProvider for ToolRetryProvider {
            fn name(&self) -> &'static str {
                "tool-retry-test"
            }

            async fn chat(&self, _request: ModelRequest) -> Result<ModelResponse, ModelError> {
                if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    Ok(ModelResponse {
                        tool_calls: vec![ToolCall {
                            id: "flaky-call".into(),
                            name: "flaky_tool".into(),
                            arguments: json!({}),
                        }],
                        ..Default::default()
                    })
                } else {
                    Ok(ModelResponse {
                        content: "tool recovered".into(),
                        ..Default::default()
                    })
                }
            }
        }

        let root = std::env::temp_dir().join(format!("ah-loop-tool-retry-{}", std::process::id()));
        let session_dir = root.join("sessions");
        let _ = std::fs::remove_dir_all(&root);
        let ctx = Context::new();
        let effects = ctx
            .mount_all(vec![
                StdArc::new(ah_plugins_tools::ToolsPlugin),
                StdArc::new(ah_plugins_session_log::SessionLogPlugin::new(
                    session_dir.join("default.jsonl"),
                    &session_dir,
                )),
            ])
            .expect("mount tool retry context");
        let calls = StdArc::new(AtomicUsize::new(0));
        let tools = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");
        let tool_effect = tools.register(StdArc::new(FlakyTool {
            calls: calls.clone(),
        }));
        let session = ctx.service::<dyn SessionLog>(&SESSIONS).expect("session");
        let rails: StdArc<dyn RailRuntime> = StdArc::new(
            ah_plugins_rails::LocalRailRuntime::new(ah_contracts::rails::RailConfig {
                max_tool_retries: 1,
                retry_backoff_ms: vec![0],
                ..Default::default()
            })
            .expect("rails"),
        );
        let agent = AgentLoop::new(
            StdArc::new(ToolRetryProvider {
                calls: AtomicUsize::new(0),
            }),
            tools,
            session.clone(),
            ctx,
            2,
        )
        .with_rails(Some(rails));
        let result = agent
            .run_in_session(session.clone(), "retry flaky tool")
            .await;
        assert_eq!(result.state, AgentRunState::Completed, "{result:?}");
        assert_eq!(result.answer.as_deref(), Some("tool recovered"));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(result.tool_calls, 2);
        let tool_results: Vec<_> = session
            .events()
            .into_iter()
            .filter(|event| event.kind == SessionEventKind::ToolResult)
            .collect();
        assert_eq!(tool_results.len(), 1);
        assert_eq!(tool_results[0].payload["status"], "completed");
        drop(tool_effect);
        drop(effects);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn reconstructs_complete_stream_delta_after_restart() {
        let path = std::env::temp_dir().join(format!(
            "ah-loop-stream-recovery-{}.jsonl",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let log = ah_plugins_session_log::JsonlSessionLog::open(&path, Context::new())
            .expect("open stream session");
        for arguments in ["{\"path\":\"", ".\"}"] {
            log.append(
                SessionEventKind::System,
                json!({
                    "event": "assistant_stream_delta",
                    "stream_id": "stream-1",
                    "content_delta": "",
                    "tool_call_deltas": [{
                        "index": 0,
                        "id": "stream-call",
                        "name": "list_dir",
                        "arguments": arguments
                    }]
                }),
            )
            .expect("persist stream delta");
        }
        let pending = AgentLoop::pending_tool_calls(&log).expect("reconstruct stream call");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "stream-call");
        assert_eq!(pending[0].arguments["path"], ".");
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn fork_and_resume_preserves_history() {
        use ah_contracts::session::SessionManager;

        let root = std::env::temp_dir().join(format!("ah-loop-resume-{}", std::process::id()));
        let session_dir =
            std::env::temp_dir().join(format!("ah-loop-resume-sessions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&session_dir);
        let (ctx, effects) = build_ctx_with_dir(&root, &session_dir);

        let manager = ctx
            .service::<dyn SessionManager>(&ah_contracts::keys::SESSION_MANAGER)
            .expect("manager");
        let agent = ctx
            .service::<dyn ah_contracts::agent::AgentLoopRuntime>(&AGENT_LOOP)
            .expect("agent-loop");

        // 第一轮:task-a
        let a = manager.create("task-a").expect("create");
        let result = agent.run_in_session(a.clone(), "first task").await;
        assert_eq!(result.state, AgentRunState::Completed);
        let answer = result.answer.unwrap();
        assert!(answer.contains("mock final answer"));
        let a_events = a.events().len();
        assert!(a_events >= 2);

        // fork 到 task-b:历史复制,续跑追加
        let b = manager.fork("task-a", "task-b").expect("fork");
        assert_eq!(b.events().len(), a_events);
        let _ = agent.run_in_session(b.clone(), "follow up").await;
        assert!(b.events().len() > a_events, "resume 追加了事件");
        assert_eq!(a.events().len(), a_events, "原会话不受影响");

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&session_dir);
    }
    #[tokio::test]
    async fn recovers_pending_tool_call_after_restart() {
        let root = std::env::temp_dir().join(format!("ah-loop-crash-root-{}", std::process::id()));
        let session_dir =
            std::env::temp_dir().join(format!("ah-loop-crash-sessions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&session_dir);
        std::fs::create_dir_all(&session_dir).expect("session dir");
        let path = session_dir.join("default.jsonl");

        // 模拟进程在工具返回前崩溃:助手调用已经持久化,ToolResult 尚未落盘。
        let crashed = ah_plugins_session_log::JsonlSessionLog::open(&path, Context::new())
            .expect("open crashed session");
        crashed
            .append(
                SessionEventKind::User,
                serde_json::json!({"content": "explore the workspace"}),
            )
            .expect("persist user");
        crashed
            .append(
                SessionEventKind::Assistant,
                serde_json::json!({
                    "tool_calls": [{
                        "id": "crashed-call",
                        "name": "list_dir",
                        "arguments": {"path": "."}
                    }]
                }),
            )
            .expect("persist pending tool call");
        drop(crashed);

        // 新进程重新挂载同一会话,应先补齐 pending ToolResult,再请求模型。
        let (ctx, effects) = build_ctx_with_dir(&root, &session_dir);
        let agent = ctx
            .service::<dyn ah_contracts::agent::AgentLoopRuntime>(&AGENT_LOOP)
            .expect("agent loop");
        let result = agent.run("resume after crash").await;
        assert_eq!(result.state, AgentRunState::Completed);
        assert_eq!(result.tool_calls, 1, "pending tool call executes once");

        let resumed = ah_plugins_session_log::JsonlSessionLog::open(&path, Context::new())
            .expect("reopen resumed session");
        let events = resumed.events();
        let tool_results: Vec<_> = events
            .iter()
            .filter(|event| event.kind == SessionEventKind::ToolResult)
            .collect();
        assert_eq!(tool_results.len(), 1);
        assert_eq!(tool_results[0].payload["tool_call_id"], "crashed-call");
        assert!(
            resumed
                .derive_messages()
                .iter()
                .any(|message| message.role == ChatRole::Tool)
        );

        drop(effects);
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(session_dir);
    }
    #[tokio::test]
    async fn refuses_automatic_retry_for_non_idempotent_tool() {
        use ah_contracts::tools::{Tool, ToolError};

        struct SideEffectTool {
            calls: StdArc<AtomicUsize>,
        }

        #[async_trait]
        impl Tool for SideEffectTool {
            fn name(&self) -> &'static str {
                "side_effect"
            }

            fn description(&self) -> &'static str {
                "non-idempotent test side effect"
            }

            async fn invoke(&self, _arguments: Value) -> Result<Value, ToolError> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                Ok(json!({"executed": true}))
            }
        }

        let root =
            std::env::temp_dir().join(format!("ah-loop-no-retry-root-{}", std::process::id()));
        let session_dir =
            std::env::temp_dir().join(format!("ah-loop-no-retry-sessions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&session_dir);
        std::fs::create_dir_all(&session_dir).expect("session dir");
        let path = session_dir.join("default.jsonl");
        let crashed = ah_plugins_session_log::JsonlSessionLog::open(&path, Context::new())
            .expect("open crashed session");
        crashed
            .append(
                SessionEventKind::User,
                json!({"content": "run the side effect"}),
            )
            .expect("persist user");
        crashed
            .append(
                SessionEventKind::Assistant,
                json!({
                    "tool_calls": [{
                        "id": "unknown-outcome",
                        "name": "side_effect",
                        "arguments": {}
                    }]
                }),
            )
            .expect("persist pending tool call");
        drop(crashed);

        let (ctx, effects) = build_ctx_with_dir(&root, &session_dir);
        let calls = StdArc::new(AtomicUsize::new(0));
        let registry = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");
        let tool_effect = registry.register(StdArc::new(SideEffectTool {
            calls: calls.clone(),
        }));
        let agent = ctx
            .service::<dyn ah_contracts::agent::AgentLoopRuntime>(&AGENT_LOOP)
            .expect("agent loop");
        let result = agent.run("resume safely").await;

        assert_eq!(result.state, AgentRunState::Failed);
        assert_eq!(result.failure, Some(AgentFailure::ToolRecoveryRequired));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "unknown outcome is not retried"
        );
        let second = agent.run("continue after review").await;
        assert_eq!(second.state, AgentRunState::Completed);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "unknown outcome stays deduplicated"
        );
        let resumed = ah_plugins_session_log::JsonlSessionLog::open(&path, Context::new())
            .expect("reopen session");
        let result_event = resumed
            .events()
            .into_iter()
            .find(|event| event.kind == SessionEventKind::ToolResult)
            .expect("recovery result");
        assert_eq!(result_event.payload["status"], "unknown");
        assert!(
            result_event.payload["output"]
                .as_str()
                .unwrap_or_default()
                .contains("manual retry")
        );

        drop(tool_effect);
        drop(effects);
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(session_dir);
    }

    #[tokio::test]
    async fn persists_partial_stream_tool_call_before_timeout() {
        struct PartialStreamProvider;

        impl Seam for PartialStreamProvider {}

        #[async_trait]
        impl ModelProvider for PartialStreamProvider {
            fn name(&self) -> &'static str {
                "partial-stream-test"
            }

            async fn chat(&self, _request: ModelRequest) -> Result<ModelResponse, ModelError> {
                Err(ModelError("chat path must not be used".into()))
            }

            async fn stream_chat(
                &self,
                _request: ModelRequest,
                sink: tokio::sync::mpsc::Sender<ModelChunk>,
            ) -> Result<(), ModelError> {
                sink.send(ModelChunk {
                    content_delta: String::new(),
                    reasoning_delta: String::new(),
                    tool_call_deltas: vec![ToolCallDelta {
                        index: 0,
                        id: Some("partial-call".into()),
                        name: Some("write_file".into()),
                        arguments: "{\"path\":\"notes/".into(),
                    }],
                    done: false,
                })
                .await
                .map_err(|_| ModelError("stream receiver closed".into()))?;
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                Ok(())
            }
        }

        let root = std::env::temp_dir().join(format!("ah-loop-stream-root-{}", std::process::id()));
        let session_dir =
            std::env::temp_dir().join(format!("ah-loop-stream-sessions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&session_dir);
        let (ctx, effects) = build_ctx_with_dir(&root, &session_dir);
        let session = ctx.service::<dyn SessionLog>(&SESSIONS).expect("session");
        let tools = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");
        let agent = AgentLoop::new(
            StdArc::new(PartialStreamProvider),
            tools,
            session.clone(),
            ctx,
            2,
        )
        .with_timeout(Some(100));
        let result = agent
            .run_in_session(session.clone(), "start streaming")
            .await;
        assert_eq!(result.state, AgentRunState::TimedOut);
        assert!(session.events().iter().any(|event| {
            event.kind == SessionEventKind::System
                && event.payload["event"] == "assistant_stream_delta"
        }));
        assert!(!session.events().iter().any(|event| {
            event.kind == SessionEventKind::Assistant && event.payload.get("tool_calls").is_some()
        }));
        drop(effects);
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(session_dir);
    }

    #[tokio::test]
    async fn repeated_model_stream_is_retried_by_rails() {
        struct RepeatingProvider {
            calls: AtomicUsize,
        }

        impl Seam for RepeatingProvider {}

        #[async_trait]
        impl ModelProvider for RepeatingProvider {
            fn name(&self) -> &'static str {
                "repeating-stream"
            }

            async fn chat(&self, _request: ModelRequest) -> Result<ModelResponse, ModelError> {
                Ok(ModelResponse {
                    content: "fallback".into(),
                    ..Default::default()
                })
            }

            async fn stream_chat(
                &self,
                _request: ModelRequest,
                sink: tokio::sync::mpsc::Sender<ModelChunk>,
            ) -> Result<(), ModelError> {
                let content = if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    "xy".repeat(80)
                } else {
                    "final answer".to_string()
                };
                sink.send(ModelChunk {
                    content_delta: content,
                    ..Default::default()
                })
                .await
                .map_err(|_| ModelError("stream receiver closed".into()))?;
                Ok(())
            }
        }

        let root = std::env::temp_dir().join(format!("ah-loop-llm-retry-{}", std::process::id()));
        let session_dir = root.join("sessions");
        let _ = std::fs::remove_dir_all(&root);
        let ctx = Context::new();
        let effects = ctx
            .mount_all(vec![
                StdArc::new(ah_plugins_tools::ToolsPlugin),
                StdArc::new(ah_plugins_sysop::SysopPlugin::new(&root)),
                StdArc::new(ah_plugins_session_log::SessionLogPlugin::new(
                    session_dir.join("default.jsonl"),
                    &session_dir,
                )),
            ])
            .expect("mount stream retry context");
        let session = ctx.service::<dyn SessionLog>(&SESSIONS).expect("session");
        let tools = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");
        let rails: StdArc<dyn RailRuntime> = StdArc::new(
            ah_plugins_rails::LocalRailRuntime::new(ah_contracts::rails::RailConfig {
                max_model_retries: 1,
                retry_backoff_ms: vec![0],
                ..Default::default()
            })
            .expect("rails"),
        );
        let agent = AgentLoop::new(
            StdArc::new(RepeatingProvider {
                calls: AtomicUsize::new(0),
            }),
            tools,
            session.clone(),
            ctx,
            2,
        )
        .with_rails(Some(rails));
        let result = agent.run_in_session(session, "retry repeated stream").await;
        assert_eq!(result.state, AgentRunState::Completed, "{result:?}");
        assert_eq!(result.answer.as_deref(), Some("final answer"));
        drop(effects);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn request_model_switch_resolves_catalog_provider() {
        struct NamedProvider(&'static str);

        impl Seam for NamedProvider {}

        #[async_trait]
        impl ModelProvider for NamedProvider {
            fn name(&self) -> &'static str {
                self.0
            }

            async fn chat(&self, _request: ModelRequest) -> Result<ModelResponse, ModelError> {
                Ok(ModelResponse {
                    content: self.0.into(),
                    ..Default::default()
                })
            }
        }

        let root =
            std::env::temp_dir().join(format!("ah-loop-model-switch-{}", std::process::id()));
        let session_dir = root.join("sessions");
        let _ = std::fs::remove_dir_all(&root);
        let ctx = Context::new();
        let effects = ctx
            .mount_all(vec![
                StdArc::new(ah_plugins_tools::ToolsPlugin),
                StdArc::new(ah_plugins_session_log::SessionLogPlugin::new(
                    session_dir.join("default.jsonl"),
                    &session_dir,
                )),
            ])
            .expect("mount model switch context");
        let sessions = ctx.service::<dyn SessionLog>(&SESSIONS).expect("session");
        let tools = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");
        let default = StdArc::new(NamedProvider("default"));
        let alternate = StdArc::new(NamedProvider("alternate"));
        let catalog = ah_plugins_model_backup::StaticModelProviderCatalog::new(vec![
            default.clone(),
            alternate,
        ])
        .expect("catalog");
        let agent = AgentLoop::new(default, tools, sessions.clone(), ctx, 1)
            .with_model_catalog(StdArc::new(catalog))
            .with_request_config(&ah_contracts::agent::AgentRunConfig {
                model: Some("alternate".into()),
                ..Default::default()
            });
        let result = agent.run_in_session(sessions, "switch model").await;
        assert_eq!(result.state, AgentRunState::Completed, "{result:?}");
        assert_eq!(result.answer.as_deref(), Some("alternate"));
        drop(effects);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn cancellation_callback_stops_after_model_checkpoint() {
        let root =
            std::env::temp_dir().join(format!("ah-loop-callback-cancel-{}", std::process::id()));
        let session_dir = root.join("sessions");
        let _ = std::fs::remove_dir_all(&root);
        let ctx = Context::new();
        let effects = ctx
            .mount_all(vec![
                StdArc::new(ah_plugins_tools::ToolsPlugin),
                StdArc::new(ah_plugins_session_log::SessionLogPlugin::new(
                    session_dir.join("default.jsonl"),
                    &session_dir,
                )),
            ])
            .expect("mount callback context");
        let sessions = ctx.service::<dyn SessionLog>(&SESSIONS).expect("session");
        let tools = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");
        let callbacks = StdArc::new(ah_plugins_agent_control::LocalAgentCallbackManager::default());
        callbacks
            .request_control("default", AgentControl::Cancel)
            .expect("request callback cancel");
        let agent = AgentLoop::new(
            StdArc::new(ah_plugins_mock::MockModelProvider::default()),
            tools,
            sessions.clone(),
            ctx,
            1,
        )
        .with_controls(None, Some(callbacks.clone()), None);
        let result = agent
            .run_in_session(sessions, "cancel at callback checkpoint")
            .await;
        assert_eq!(result.state, AgentRunState::Cancelled, "{result:?}");
        let records = callbacks.callbacks();
        assert!(records.iter().any(|record| {
            record.payload["phase"] == "after_model_call" && record.state == AgentRunState::Running
        }));
        assert_eq!(
            records.last().map(|record| record.state),
            Some(AgentRunState::Cancelled)
        );
        drop(effects);
        let _ = std::fs::remove_dir_all(root);
    }
    #[tokio::test]
    async fn interruption_and_cancellation_are_logged() {
        let root = std::env::temp_dir().join(format!("ah-loop-control-{}", std::process::id()));
        let session_dir = root.join("sessions");
        let default_path = session_dir.join("default.jsonl");
        let ctx = Context::new();
        let effects = ctx
            .mount_all(vec![
                StdArc::new(ah_plugins_mock::MockPlugin),
                StdArc::new(ah_plugins_agent_control::AgentControlPlugin),
                StdArc::new(ah_plugins_tools::ToolsPlugin),
                StdArc::new(ah_plugins_sysop::SysopPlugin::new(&root)),
                StdArc::new(ah_plugins_session_log::SessionLogPlugin::new(
                    &default_path,
                    &session_dir,
                )),
            ])
            .expect("mount controls");
        let interrupt = ctx
            .service::<dyn ah_contracts::agent::InterruptRuntime>(&ah_contracts::keys::INTERRUPT)
            .expect("interrupt");
        let session = ctx.service::<dyn SessionLog>(&SESSIONS).expect("session");
        interrupt
            .request(session.id(), AgentControl::Cancel)
            .await
            .expect("request cancel");
        let agent = AgentLoop::new(
            ctx.service::<dyn ModelProvider>(&LLM).unwrap(),
            ctx.service::<dyn ToolRegistry>(&TOOLS).unwrap(),
            session.clone(),
            ctx.clone(),
            2,
        )
        .with_controls(Some(interrupt.clone()), None, None);
        let cancelled = agent.run_in_session(session.clone(), "cancel me").await;
        assert_eq!(cancelled.state, AgentRunState::Cancelled);
        assert_eq!(cancelled.failure, Some(AgentFailure::Cancelled));
        assert!(
            session
                .events()
                .iter()
                .any(|e| e.kind == SessionEventKind::AgentCanceled)
        );
        interrupt.clear(session.id());
        interrupt
            .request(session.id(), AgentControl::Interrupt)
            .await
            .unwrap();
        let interrupted = agent.run_in_session(session.clone(), "interrupt me").await;
        assert_eq!(interrupted.state, AgentRunState::Interrupted);
        assert_eq!(interrupted.failure, Some(AgentFailure::Interrupted));
        assert!(
            session
                .events()
                .iter()
                .any(|e| e.kind == SessionEventKind::AgentInterrupted)
        );
        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn empty_input_is_rejected_before_logging() {
        let root = std::env::temp_dir().join(format!("ah-loop-empty-{}", std::process::id()));
        let session_path = root.join("default.jsonl");
        let (ctx, effects) = build_ctx(&root, &session_path);
        let agent = ctx
            .service::<dyn ah_contracts::agent::AgentLoopRuntime>(&AGENT_LOOP)
            .expect("agent-loop");
        let result = agent.run("  ").await;
        assert_eq!(result.state, AgentRunState::Failed);
        assert_eq!(result.failure, Some(AgentFailure::InvalidInput));
        let sessions = ctx.service::<dyn SessionLog>(&SESSIONS).expect("sessions");
        assert!(sessions.events().is_empty());
        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&session_path);
    }

    #[tokio::test]
    async fn loop_emits_agent_step_events() {
        let root = std::env::temp_dir().join(format!("ah-loop-events-{}", std::process::id()));
        let session_path =
            std::env::temp_dir().join(format!("ah-loop-events-session-{}", std::process::id()));
        let _ = std::fs::remove_file(&session_path);
        let (ctx, effects) = build_ctx(&root, &session_path);

        let steps = StdArc::new(AtomicUsize::new(0));
        let counter = steps.clone();
        let _listener = ctx.on::<AgentStep>(move |_step| {
            counter.fetch_add(1, Ordering::SeqCst);
        });

        let agent = ctx
            .service::<dyn ah_contracts::agent::AgentLoopRuntime>(&AGENT_LOOP)
            .expect("agent-loop service");
        let _ = agent.run("go").await;

        assert!(steps.load(Ordering::SeqCst) >= 2);

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&session_path);
    }

    #[tokio::test]
    async fn context_consumer_compresses_with_tight_budget_but_logs_are_truth() {
        use ah_contracts::context::ContextEngine;
        use ah_contracts::keys::CONTEXT;

        let root = std::env::temp_dir().join(format!("ah-loop-ctx-{}", std::process::id()));
        let session_dir = root.join("sessions");
        let default_path = session_dir.join("default.jsonl");
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![
            StdArc::new(ah_plugins_mock::MockPlugin),
            StdArc::new(ah_plugins_tools::ToolsPlugin),
            StdArc::new(ah_plugins_sysop::SysopPlugin::new(&root)),
            StdArc::new(ah_plugins_session_log::SessionLogPlugin::new(
                &default_path,
                &session_dir,
            )),
            StdArc::new(ah_plugins_agent_control::AgentControlPlugin),
            StdArc::new(ah_plugins_model_backup::ModelBackupPlugin::new(Vec::new())),
            StdArc::new(ah_plugins_context::ContextPlugin::new(root.join("offload"))),
            StdArc::new(AgentLoopPlugin::default()),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");

        // 手工挂载一个极小预算的循环实例:context seam 消费方真实生效。
        let context = ctx.service::<dyn ContextEngine>(&CONTEXT).expect("context");
        let agent = ctx
            .service::<dyn ah_contracts::agent::AgentLoopRuntime>(&AGENT_LOOP)
            .expect("agent-loop");
        // 验证插件已把 context 挂到循环(通过行为验证:用 with_context 重建)。
        let sessions = ctx.service::<dyn SessionLog>(&SESSIONS).expect("sessions");
        let llm = ctx.service::<dyn ModelProvider>(&LLM).expect("llm");
        let tools = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");
        let _ = agent; // 插件路径循环已生效;此处构造小预算实例做消费验证。
        let tight = AgentLoop::new(llm, tools, sessions, ctx.clone(), 3).with_context(context, 10);
        let result = tight.run("explore the workspace").await;
        assert_eq!(result.state, AgentRunState::Completed);
        let answer = result.answer.unwrap();
        assert!(answer.contains("mock final answer"));
        // 日志即真相:完整历史仍在日志(压缩只影响模型可见窗口)。
        let sessions2 = ctx.service::<dyn SessionLog>(&SESSIONS).expect("sessions");
        assert!(
            sessions2.events().len() >= 3,
            "history preserved in the log"
        );

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 测试专用录制 provider:记录每次模型请求后委托给真实 mock(仅测试用桩)。
    struct RecordingProvider {
        inner: StdArc<dyn ModelProvider>,
        requests: StdArc<std::sync::Mutex<Vec<ModelRequest>>>,
    }

    impl Seam for RecordingProvider {}

    #[async_trait]
    impl ModelProvider for RecordingProvider {
        fn name(&self) -> &'static str {
            "recording"
        }

        async fn chat(
            &self,
            request: ModelRequest,
        ) -> Result<ah_contracts::llm::ModelResponse, ah_contracts::llm::ModelError> {
            self.requests.lock().unwrap().push(request.clone());
            self.inner.chat(request).await
        }
    }

    struct DirectFailingModel;
    impl Seam for DirectFailingModel {}
    #[async_trait]
    impl ModelProvider for DirectFailingModel {
        fn name(&self) -> &'static str {
            "primary"
        }
        async fn chat(
            &self,
            _: ModelRequest,
        ) -> Result<ModelResponse, ah_contracts::llm::ModelError> {
            Err(ah_contracts::llm::ModelError("primary unavailable".into()))
        }
    }

    struct DirectBackupRuntime;
    impl Seam for DirectBackupRuntime {}
    #[async_trait]
    impl ModelBackup for DirectBackupRuntime {
        async fn chat(
            &self,
            primary: &dyn ModelProvider,
            request: ModelRequest,
        ) -> Result<ModelResponse, ah_contracts::model_backup::ModelBackupError> {
            primary
                .chat(request.clone())
                .await
                .map_err(|e| ah_contracts::model_backup::ModelBackupError(e.0))
                .or_else(|_| {
                    Ok(ModelResponse {
                        content: "backup answer".into(),
                        tool_calls: vec![],
                        reasoning_content: None,
                    })
                })
        }
        fn models(&self) -> Vec<String> {
            vec!["backup".into()]
        }
    }

    struct DirectFailingBackupRuntime;
    impl Seam for DirectFailingBackupRuntime {}
    #[async_trait]
    impl ModelBackup for DirectFailingBackupRuntime {
        async fn chat(
            &self,
            _: &dyn ModelProvider,
            _: ModelRequest,
        ) -> Result<ModelResponse, ah_contracts::model_backup::ModelBackupError> {
            Err(ah_contracts::model_backup::ModelBackupError(
                "all models unavailable".into(),
            ))
        }
        fn models(&self) -> Vec<String> {
            vec!["backup".into()]
        }
    }

    #[tokio::test]
    async fn backup_failure_is_persisted_as_system_event() {
        let ctx = Context::new();
        let session_dir =
            std::env::temp_dir().join(format!("ah-loop-backup-error-{}", std::process::id()));
        let session_path = session_dir.join("default.jsonl");
        let _ = std::fs::remove_dir_all(&session_dir);
        let session = StdArc::new(
            ah_plugins_session_log::JsonlSessionLog::open(&session_path, ctx.clone()).unwrap(),
        );
        let agent = AgentLoop::new(
            StdArc::new(DirectFailingModel),
            StdArc::new(ah_plugins_tools::LocalToolRegistry::new(ctx.clone())),
            session.clone(),
            ctx,
            1,
        )
        .with_model_backup(Some(StdArc::new(DirectFailingBackupRuntime)));
        let result = agent.run("fail").await;
        assert_eq!(result.state, AgentRunState::Failed);
        assert_eq!(result.failure, Some(AgentFailure::Model));
        let event = session
            .events()
            .into_iter()
            .find(|event| event.kind == SessionEventKind::System)
            .expect("error event");
        assert_eq!(event.payload["event"], "agent_error");
        assert!(
            event.payload["message"]
                .as_str()
                .unwrap()
                .contains("model backup failed")
        );
    }

    #[tokio::test]
    async fn model_failure_is_persisted_as_system_event() {
        let ctx = Context::new();
        let session_dir =
            std::env::temp_dir().join(format!("ah-loop-error-{}", std::process::id()));
        let session_path = session_dir.join("default.jsonl");
        let _ = std::fs::remove_dir_all(&session_dir);
        let session = StdArc::new(
            ah_plugins_session_log::JsonlSessionLog::open(&session_path, ctx.clone()).unwrap(),
        );
        let agent = AgentLoop::new(
            StdArc::new(DirectFailingModel),
            StdArc::new(ah_plugins_tools::LocalToolRegistry::new(ctx.clone())),
            session.clone(),
            ctx,
            1,
        );
        let result = agent.run("fail").await;
        assert_eq!(result.state, AgentRunState::Failed);
        assert_eq!(result.failure, Some(AgentFailure::Model));
        let event = session
            .events()
            .into_iter()
            .find(|event| event.kind == SessionEventKind::System)
            .expect("error event");
        assert_eq!(event.payload["event"], "agent_error");
        assert_eq!(event.payload["failure"], "Model");
    }

    #[test]
    fn runtime_card_describes_recovery_capabilities() {
        let ctx = Context::new();
        let session_dir = std::env::temp_dir().join(format!("ah-loop-card-{}", std::process::id()));
        let session_path = session_dir.join("default.jsonl");
        let _ = std::fs::remove_dir_all(&session_dir);
        let session = StdArc::new(
            ah_plugins_session_log::JsonlSessionLog::open(&session_path, ctx.clone()).unwrap(),
        );
        let runtime = AgentLoop::new(
            StdArc::new(DirectFailingModel),
            StdArc::new(ah_plugins_tools::LocalToolRegistry::new(ctx.clone())),
            session,
            ctx,
            1,
        );
        let card = <AgentLoop as ah_contracts::agent::AgentLoopRuntime>::card(&runtime);
        assert_eq!(card.id, "agent-loop");
        assert!(card.capabilities.iter().any(|item| item == "tool-use"));
        assert!(
            card.capabilities
                .iter()
                .any(|item| item == "session-recovery")
        );
        let encoded = serde_json::to_string(&card).expect("card serializes");
        assert!(encoded.contains("session-recovery"));
    }

    #[tokio::test]
    async fn agent_loop_consumes_model_backup_after_primary_failure() {
        let ctx = Context::new();
        let session_dir =
            std::env::temp_dir().join(format!("ah-loop-backup-{}", std::process::id()));
        let session_path = session_dir.join("default.jsonl");
        let session = StdArc::new(
            ah_plugins_session_log::JsonlSessionLog::open(&session_path, ctx.clone()).unwrap(),
        );
        let agent = AgentLoop::new(
            StdArc::new(DirectFailingModel),
            StdArc::new(ah_plugins_tools::LocalToolRegistry::new(ctx.clone())),
            session,
            ctx,
            1,
        )
        .with_model_backup(Some(StdArc::new(DirectBackupRuntime)));
        let result = agent.run("recover").await;
        assert_eq!(result.state, AgentRunState::Completed);
        assert_eq!(result.answer.as_deref(), Some("backup answer"));
    }

    #[tokio::test]
    async fn prompt_consumer_injects_rendered_system_message() {
        use ah_contracts::prompt::{PromptRegistry, PromptTemplate};

        let root = std::env::temp_dir().join(format!("ah-loop-prompt-{}", std::process::id()));
        let session_dir = root.join("sessions");
        let default_path = session_dir.join("default.jsonl");
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![
            StdArc::new(ah_plugins_mock::MockPlugin),
            StdArc::new(ah_plugins_tools::ToolsPlugin),
            StdArc::new(ah_plugins_sysop::SysopPlugin::new(&root)),
            StdArc::new(ah_plugins_session_log::SessionLogPlugin::new(
                &default_path,
                &session_dir,
            )),
            StdArc::new(ah_plugins_prompt::PromptPlugin::new(root.join("prompts"))),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");

        // 注册 agent 模板(prompt seam 提供方)。
        let prompts = ctx
            .service::<dyn PromptRegistry>(&ah_contracts::keys::PROMPT)
            .expect("prompt");
        prompts
            .register(PromptTemplate {
                name: "agent".to_string(),
                version: 0,
                template: "You are the workspace agent. Be concise.".to_string(),
                description: "agent system prompt".to_string(),
            })
            .expect("register");

        // 手工构造循环:录制 provider + prompt 消费方。
        let sessions = ctx.service::<dyn SessionLog>(&SESSIONS).expect("sessions");
        let mock = ctx.service::<dyn ModelProvider>(&LLM).expect("llm");
        let tools = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");
        let requests = StdArc::new(std::sync::Mutex::new(Vec::new()));
        let recording = StdArc::new(RecordingProvider {
            inner: mock,
            requests: requests.clone(),
        });
        let agent = AgentLoop::new(
            recording as StdArc<dyn ModelProvider>,
            tools,
            sessions,
            ctx.clone(),
            3,
        )
        .with_prompt(prompts, "agent");
        let result = agent.run("explore the workspace").await;
        assert_eq!(result.state, AgentRunState::Completed);
        let answer = result.answer.unwrap();
        assert!(answer.contains("mock final answer"));

        // 首次请求的首条消息是渲染出的 system 提示。
        let first = requests
            .lock()
            .unwrap()
            .first()
            .expect("at least one request")
            .clone();
        assert_eq!(first.messages[0].role, ChatRole::System);
        assert_eq!(
            first.messages[0].content,
            "You are the workspace agent. Be concise."
        );

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    // ---------- P1-02/03:结构化 AgentResult 与执行中取消/超时 ----------

    /// 精确统计:两次工具调用后作答 → iterations=2、tool_calls=2、Completed。
    #[tokio::test]
    async fn agent_result_reports_exact_iterations_and_tool_calls() {
        use ah_contracts::llm::ToolCall;
        use ah_contracts::tools::{Tool, ToolError};
        use serde_json::Value;

        struct NoopTool;
        #[async_trait]
        impl Tool for NoopTool {
            fn name(&self) -> &'static str {
                "noop"
            }
            fn description(&self) -> &'static str {
                "no-op tool"
            }
            fn parameters(&self) -> Value {
                json!({ "type": "object", "properties": {} })
            }
            async fn invoke(&self, _: Value) -> Result<Value, ToolError> {
                Ok(json!({ "ok": true }))
            }
        }

        struct TwoToolModel {
            calls: AtomicUsize,
        }
        impl Seam for TwoToolModel {}
        #[async_trait]
        impl ModelProvider for TwoToolModel {
            fn name(&self) -> &'static str {
                "two-tool"
            }
            async fn chat(
                &self,
                _: ModelRequest,
            ) -> Result<ModelResponse, ah_contracts::llm::ModelError> {
                let n = self.calls.fetch_add(1, Ordering::SeqCst);
                if n < 2 {
                    Ok(ModelResponse {
                        content: String::new(),
                        tool_calls: vec![ToolCall {
                            id: format!("c{n}"),
                            name: "noop".to_string(),
                            arguments: json!({}),
                        }],
                        reasoning_content: None,
                    })
                } else {
                    Ok(ModelResponse {
                        content: "final answer".to_string(),
                        tool_calls: vec![],
                        reasoning_content: None,
                    })
                }
            }
        }

        let ctx = Context::new();
        let session_dir =
            std::env::temp_dir().join(format!("ah-loop-stats-{}", std::process::id()));
        let session_path = session_dir.join("default.jsonl");
        let _ = std::fs::remove_dir_all(&session_dir);
        let registry = StdArc::new(ah_plugins_tools::LocalToolRegistry::new(ctx.clone()));
        let _tool_effect = registry.register(StdArc::new(NoopTool));
        let session = StdArc::new(
            ah_plugins_session_log::JsonlSessionLog::open(&session_path, ctx.clone()).unwrap(),
        );
        let agent = AgentLoop::new(
            StdArc::new(TwoToolModel {
                calls: AtomicUsize::new(0),
            }),
            registry,
            session,
            ctx,
            5,
        );
        let result = agent.run("stats").await;
        assert_eq!(result.state, AgentRunState::Completed);
        assert_eq!(result.answer.as_deref(), Some("final answer"));
        assert_eq!(result.iterations, 2, "两轮工具轮 + 一轮作答");
        assert_eq!(result.tool_calls, 2, "两个工具调用均已统计");
        let _ = std::fs::remove_dir_all(&session_dir);
    }

    /// 超时中止**在途**模型调用(而非仅轮次边界):300ms 超时下 10s 睡眠模型
    /// 必须很快以 TimedOut 返回。
    #[tokio::test]
    async fn timeout_aborts_in_flight_model_call() {
        struct SleepingModel;
        impl Seam for SleepingModel {}
        #[async_trait]
        impl ModelProvider for SleepingModel {
            fn name(&self) -> &'static str {
                "sleeping"
            }
            async fn chat(
                &self,
                _: ModelRequest,
            ) -> Result<ModelResponse, ah_contracts::llm::ModelError> {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                Ok(ModelResponse {
                    content: "slow done".to_string(),
                    tool_calls: vec![],
                    reasoning_content: None,
                })
            }
        }

        let ctx = Context::new();
        let session_dir = std::env::temp_dir().join(format!("ah-loop-slow-{}", std::process::id()));
        let session_path = session_dir.join("default.jsonl");
        let _ = std::fs::remove_dir_all(&session_dir);
        let session = StdArc::new(
            ah_plugins_session_log::JsonlSessionLog::open(&session_path, ctx.clone()).unwrap(),
        );
        let agent = AgentLoop::new(
            StdArc::new(SleepingModel),
            StdArc::new(ah_plugins_tools::LocalToolRegistry::new(ctx.clone())),
            session,
            ctx,
            3,
        )
        .with_timeout(Some(300));
        let start = std::time::Instant::now();
        let result = agent.run("slow").await;
        let elapsed_ms = start.elapsed().as_millis();
        assert_eq!(result.state, AgentRunState::TimedOut);
        assert_eq!(result.failure, Some(AgentFailure::TimedOut));
        assert!(
            elapsed_ms < 2000,
            "必须中止在途模型调用,实际耗时 {elapsed_ms}ms"
        );
        let _ = std::fs::remove_dir_all(&session_dir);
    }

    /// 中断中止**在途**模型调用:运行中请求 Interrupt,10s 睡眠模型应快速中止。
    #[tokio::test]
    async fn interrupt_aborts_in_flight_model_call() {
        use ah_contracts::keys::INTERRUPT;

        struct SleepingModel;
        impl Seam for SleepingModel {}
        #[async_trait]
        impl ModelProvider for SleepingModel {
            fn name(&self) -> &'static str {
                "sleeping"
            }
            async fn chat(
                &self,
                _: ModelRequest,
            ) -> Result<ModelResponse, ah_contracts::llm::ModelError> {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                Ok(ModelResponse {
                    content: "slow done".to_string(),
                    tool_calls: vec![],
                    reasoning_content: None,
                })
            }
        }

        let ctx = Context::new();
        let session_dir =
            std::env::temp_dir().join(format!("ah-loop-int-mid-{}", std::process::id()));
        let default_path = session_dir.join("default.jsonl");
        let effects = ctx
            .mount_all(vec![
                StdArc::new(ah_plugins_agent_control::AgentControlPlugin),
                StdArc::new(ah_plugins_session_log::SessionLogPlugin::new(
                    &default_path,
                    &session_dir,
                )),
            ])
            .expect("mount");
        let interrupt = ctx
            .service::<dyn InterruptRuntime>(&INTERRUPT)
            .expect("interrupt");
        let session = ctx.service::<dyn SessionLog>(&SESSIONS).expect("session");
        let sid = session.id().to_string();
        let i2 = interrupt.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            let _ = i2.request(&sid, AgentControl::Interrupt).await;
        });
        let agent = AgentLoop::new(
            StdArc::new(SleepingModel),
            StdArc::new(ah_plugins_tools::LocalToolRegistry::new(ctx.clone())),
            session.clone(),
            ctx,
            3,
        )
        .with_controls(Some(interrupt.clone()), None, None);
        let start = std::time::Instant::now();
        let result = agent.run("slow").await;
        let elapsed_ms = start.elapsed().as_millis();
        assert_eq!(result.state, AgentRunState::Interrupted);
        assert_eq!(result.failure, Some(AgentFailure::Interrupted));
        assert!(
            elapsed_ms < 2000,
            "必须中止在途模型调用,实际耗时 {elapsed_ms}ms"
        );
        drop(effects);
        let _ = std::fs::remove_dir_all(&session_dir);
    }

    /// 中断中止**在途**工具调用:模型先请求一个睡眠工具,运行中 Interrupt。
    #[tokio::test]
    async fn interrupt_aborts_in_flight_tool_call() {
        use ah_contracts::keys::INTERRUPT;
        use ah_contracts::llm::ToolCall;
        use ah_contracts::tools::{Tool, ToolError};
        use serde_json::Value;

        struct SleepyTool;
        #[async_trait]
        impl Tool for SleepyTool {
            fn name(&self) -> &'static str {
                "sleepy"
            }
            fn description(&self) -> &'static str {
                "sleeps forever"
            }
            fn parameters(&self) -> Value {
                json!({ "type": "object", "properties": {} })
            }
            async fn invoke(&self, _: Value) -> Result<Value, ToolError> {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                Ok(json!({}))
            }
        }

        struct ToolThenAnswerModel {
            calls: AtomicUsize,
        }
        impl Seam for ToolThenAnswerModel {}
        #[async_trait]
        impl ModelProvider for ToolThenAnswerModel {
            fn name(&self) -> &'static str {
                "tool-then-answer"
            }
            async fn chat(
                &self,
                _: ModelRequest,
            ) -> Result<ModelResponse, ah_contracts::llm::ModelError> {
                if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    Ok(ModelResponse {
                        content: String::new(),
                        tool_calls: vec![ToolCall {
                            id: "c1".to_string(),
                            name: "sleepy".to_string(),
                            arguments: json!({}),
                        }],
                        reasoning_content: None,
                    })
                } else {
                    Ok(ModelResponse {
                        content: "done".to_string(),
                        tool_calls: vec![],
                        reasoning_content: None,
                    })
                }
            }
        }

        let ctx = Context::new();
        let session_dir =
            std::env::temp_dir().join(format!("ah-loop-tool-int-{}", std::process::id()));
        let default_path = session_dir.join("default.jsonl");
        let effects = ctx
            .mount_all(vec![
                StdArc::new(ah_plugins_agent_control::AgentControlPlugin),
                StdArc::new(ah_plugins_session_log::SessionLogPlugin::new(
                    &default_path,
                    &session_dir,
                )),
            ])
            .expect("mount");
        let interrupt = ctx
            .service::<dyn InterruptRuntime>(&INTERRUPT)
            .expect("interrupt");
        let session = ctx.service::<dyn SessionLog>(&SESSIONS).expect("session");
        let registry = StdArc::new(ah_plugins_tools::LocalToolRegistry::new(ctx.clone()));
        let _tool_effect = registry.register(StdArc::new(SleepyTool));
        let sid = session.id().to_string();
        let i2 = interrupt.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            let _ = i2.request(&sid, AgentControl::Interrupt).await;
        });
        let agent = AgentLoop::new(
            StdArc::new(ToolThenAnswerModel {
                calls: AtomicUsize::new(0),
            }),
            registry,
            session.clone(),
            ctx,
            3,
        )
        .with_controls(Some(interrupt.clone()), None, None);
        let start = std::time::Instant::now();
        let result = agent.run("slow tool").await;
        let elapsed_ms = start.elapsed().as_millis();
        assert_eq!(result.state, AgentRunState::Interrupted);
        assert_eq!(result.failure, Some(AgentFailure::Interrupted));
        assert!(
            elapsed_ms < 2000,
            "必须中止在途工具调用,实际耗时 {elapsed_ms}ms"
        );
        drop(effects);
        let _ = std::fs::remove_dir_all(&session_dir);
    }
    #[test]
    fn default_plugin_has_bounded_run_timeout() {
        assert_eq!(AgentLoopPlugin::default().timeout_ms, Some(120_000));
    }
}
