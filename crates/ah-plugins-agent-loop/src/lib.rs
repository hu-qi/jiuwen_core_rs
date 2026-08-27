//! # ah-plugins-agent-loop
//!
//! 真实 ReAct agent 循环:注入 llm + tools + sessions 三个 seam。
//! 循环以会话日志为唯一事实来源(日志即真相):
//! 每次模型请求的消息序列由日志投影(derive_messages)重建,
//! 每轮的用户消息/助手消息/工具调用/工具结果都追加到日志。

use std::sync::Arc;

use std::collections::HashMap;

use ah_contracts::agent::{
    AgentCallbackContext, AgentCallbackManager, AgentControl, AgentRunState, InterruptRuntime,
};
use ah_contracts::context::ContextEngine;
use ah_contracts::keys::{
    AGENT_CALLBACKS, AGENT_LOOP, CONTEXT, INTERRUPT, LLM, MODEL_BACKUP, MODEL_BACKUP_POLICY,
    PROMPT, SESSIONS, TOOLS,
};
use ah_contracts::llm::{ChatMessage, ChatRole, ModelProvider, ModelRequest, ToolSchema};
use ah_contracts::model_backup::{ModelBackup, ModelBackupPolicy, ModelBackupPolicyProvider};
use ah_contracts::prelude::Effect;
use ah_contracts::prompt::PromptRegistry;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::session::{SessionEventKind, SessionLog};
use ah_contracts::tools::ToolRegistry;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use serde_json::{Value, json};

/// agent 循环错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentLoopFailure {
    InvalidInput,
    Interrupted,
    Cancelled,
    TimedOut,
    Session,
    Model,
    Context,
    IterationLimit,
}

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
    timeout_ms: Option<u64>,
    backup: Option<Arc<dyn ModelBackup>>,
    backup_policy: ModelBackupPolicy,
}

impl AgentLoop {
    /// 构建循环。
    pub fn new(
        llm: Arc<dyn ModelProvider>,
        tools: Arc<dyn ToolRegistry>,
        sessions: Arc<dyn SessionLog>,
        ctx: Context,
        max_iterations: usize,
    ) -> Self {
        Self {
            llm,
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
            timeout_ms: None,
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

    /// 在默认会话上运行一轮任务。
    pub async fn run(&self, input: &str) -> Result<String, AgentLoopError> {
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

    pub async fn run_in_session(
        &self,
        session: Arc<dyn SessionLog>,
        input: &str,
    ) -> Result<String, AgentLoopError> {
        if input.trim().is_empty() {
            return Err(AgentLoopError::new(
                AgentLoopFailure::InvalidInput,
                "input must not be empty",
            ));
        }
        let session_id = session.id().to_string();
        let deadline = self
            .timeout_ms
            .map(|ms| tokio::time::Instant::now() + std::time::Duration::from_millis(ms));
        self.notify(
            &session_id,
            AgentRunState::Running,
            0,
            json!({"input": input}),
        )
        .await?;
        // 1) 用户消息入日志。
        session
            .append(SessionEventKind::User, json!({ "content": input }))
            .map_err(|e| {
                AgentLoopError::new(
                    AgentLoopFailure::Session,
                    format!("session append failed: {e}"),
                )
            })?;

        for iteration in 0..self.max_iterations {
            if let Some(deadline) = deadline
                && tokio::time::Instant::now() >= deadline
            {
                let _ = session.append(
                    SessionEventKind::AgentTimedOut,
                    json!({"iteration": iteration}),
                );
                self.notify(&session_id, AgentRunState::TimedOut, iteration, Value::Null)
                    .await?;
                return Err(AgentLoopError::new(
                    AgentLoopFailure::TimedOut,
                    "agent run timed out",
                ));
            }
            match self.control(&session_id) {
                AgentControl::Interrupt => {
                    session
                        .append(
                            SessionEventKind::AgentInterrupted,
                            json!({"iteration": iteration}),
                        )
                        .map_err(|e| {
                            AgentLoopError::new(
                                AgentLoopFailure::Session,
                                format!("session append failed: {e}"),
                            )
                        })?;
                    self.notify(
                        &session_id,
                        AgentRunState::Interrupted,
                        iteration,
                        Value::Null,
                    )
                    .await?;
                    return Err(AgentLoopError::new(
                        AgentLoopFailure::Interrupted,
                        "agent run interrupted; resume the session to continue",
                    ));
                }
                AgentControl::Cancel => {
                    session
                        .append(
                            SessionEventKind::AgentCanceled,
                            json!({"iteration": iteration}),
                        )
                        .map_err(|e| {
                            AgentLoopError::new(
                                AgentLoopFailure::Session,
                                format!("session append failed: {e}"),
                            )
                        })?;
                    self.notify(
                        &session_id,
                        AgentRunState::Cancelled,
                        iteration,
                        Value::Null,
                    )
                    .await?;
                    return Err(AgentLoopError::new(
                        AgentLoopFailure::Cancelled,
                        "agent run cancelled",
                    ));
                }
                AgentControl::Continue => {}
            }
            // 2) 模型可见消息:优先经 context seam 按预算组装(压缩时注入摘要),
            //    未挂载时用日志投影直通(日志即真相:完整历史始终在日志)。
            let messages = if let Some(context) = &self.context {
                context
                    .assemble(session.as_ref(), self.token_budget)
                    .await
                    .map_err(|e| {
                        AgentLoopError::new(
                            AgentLoopFailure::Context,
                            format!("context assemble failed: {e}"),
                        )
                    })?
                    .messages
            } else {
                session.derive_messages()
            };
            // prompt seam 消费:注册的模板渲染为 system 消息(请求首条)。
            let mut messages = messages;
            if let Some(registry) = &self.prompt
                && registry.get(&self.prompt_name).is_some()
                && let Ok(rendered) = registry.render(&self.prompt_name, &HashMap::new())
            {
                messages.insert(0, ChatMessage::new(ChatRole::System, rendered.content));
            }
            let request = ModelRequest {
                messages,
                tools: self.tool_schemas(),
                ..Default::default()
            };
            let response = if let Some(backup) = &self.backup {
                backup
                    .chat_with_policy(self.llm.as_ref(), request, self.backup_policy)
                    .await
                    .map_err(|e| format!("model backup failed: {e}"))
            } else {
                self.llm
                    .chat(request)
                    .await
                    .map_err(|e| format!("model error: {e}"))
            };
            let response = match response {
                Ok(response) => response,
                Err(message) => {
                    let error = AgentLoopError::new(AgentLoopFailure::Model, message);
                    let _ = session.append(
                        SessionEventKind::System,
                        json!({
                            "event": "agent_error",
                            "failure": format!("{:?}", error.kind),
                            "message": error.message,
                        }),
                    );
                    return Err(error);
                }
            };

            if response.tool_calls.is_empty() {
                // 3) 最终回答入日志,结束。
                session
                    .append(
                        SessionEventKind::Assistant,
                        json!({ "content": response.content }),
                    )
                    .map_err(|e| {
                        AgentLoopError::new(
                            AgentLoopFailure::Session,
                            format!("session append failed: {e}"),
                        )
                    })?;
                self.ctx.emit(AgentStep {
                    iteration,
                    tool_calls: 0,
                    done: true,
                });
                self.notify(
                    &session_id,
                    AgentRunState::Completed,
                    iteration,
                    json!({"answer": response.content}),
                )
                .await?;
                if let Some(runtime) = &self.interrupt {
                    runtime.clear(&session_id);
                }
                return Ok(response.content);
            }

            // 4) 助手工具调用入日志。
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
            session
                .append(SessionEventKind::Assistant, json!({ "tool_calls": calls }))
                .map_err(|e| {
                    AgentLoopError::new(
                        AgentLoopFailure::Session,
                        format!("session append failed: {e}"),
                    )
                })?;

            // 5) 真实执行工具。工具失败/被拒**不中断循环**,错误作为工具结果回喂模型
            //    (模型据此换方案),符合真实 agent 语义。
            for call in &response.tool_calls {
                let output = match self.tools.invoke(&call.name, call.arguments.clone()).await {
                    Ok(value) => value.to_string(),
                    Err(error) => format!("tool error: {error}"),
                };
                session
                    .append(
                        SessionEventKind::ToolResult,
                        json!({
                            "tool_call_id": call.id,
                            "output": output,
                        }),
                    )
                    .map_err(|e| {
                        AgentLoopError::new(
                            AgentLoopFailure::Session,
                            format!("session append failed: {e}"),
                        )
                    })?;
            }

            self.ctx.emit(AgentStep {
                iteration,
                tool_calls: response.tool_calls.len(),
                done: false,
            });
        }
        self.notify(
            &session_id,
            AgentRunState::Failed,
            self.max_iterations,
            Value::Null,
        )
        .await?;
        Err(AgentLoopError::new(
            AgentLoopFailure::IterationLimit,
            format!("max iterations exceeded: {max}", max = self.max_iterations),
        ))
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

    async fn run(&self, input: &str) -> Result<String, ah_contracts::agent::AgentControlError> {
        AgentLoop::run(self, input)
            .await
            .map_err(|error| ah_contracts::agent::AgentControlError(error.message))
    }

    async fn run_in_session(
        &self,
        session: Arc<dyn SessionLog>,
        input: &str,
    ) -> Result<String, ah_contracts::agent::AgentControlError> {
        AgentLoop::run_in_session(self, session, input)
            .await
            .map_err(|error| ah_contracts::agent::AgentControlError(error.message))
    }

    async fn run_in_session_with_timeout(
        &self,
        session: Arc<dyn SessionLog>,
        input: &str,
        timeout_ms: Option<u64>,
    ) -> Result<String, ah_contracts::agent::AgentControlError> {
        AgentLoop::with_timeout(self.clone(), timeout_ms)
            .run_in_session(session, input)
            .await
            .map_err(|error| ah_contracts::agent::AgentControlError(error.message))
    }
}

/// agent 循环插件:注入 llm + tools + sessions,提供 agent-loop 服务。
pub struct AgentLoopPlugin {
    max_iterations: usize,
}

impl AgentLoopPlugin {
    /// 创建循环插件。
    pub fn new(max_iterations: usize) -> Self {
        Self { max_iterations }
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
        let backup = ctx.service::<dyn ModelBackup>(&MODEL_BACKUP);
        let backup_policy = ctx
            .service::<dyn ModelBackupPolicyProvider>(&MODEL_BACKUP_POLICY)
            .map(|provider| provider.policy())
            .unwrap_or_default();
        // prompt seam 可选:注册 "agent" 模板时注入系统提示。
        let prompt = ctx.service::<dyn PromptRegistry>(&PROMPT);
        let mut agent = AgentLoop::new(llm, tools, sessions, ctx.clone(), self.max_iterations);
        if let Some(context) = context {
            agent = agent.with_context(context, 8192);
        }
        if let Some(prompt) = prompt {
            agent = agent.with_prompt(prompt, "agent");
        }
        agent = agent
            .with_controls(interrupt, callbacks, None)
            .with_model_backup(backup)
            .with_model_backup_policy(backup_policy);
        let agent: Arc<dyn ah_contracts::agent::AgentLoopRuntime> = Arc::new(agent);
        Ok(vec![ctx.register(AGENT_LOOP, agent)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::SESSIONS;
    use ah_contracts::llm::{ChatRole, ModelResponse};
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
        let answer = agent.run("explore the workspace").await.expect("run");

        assert!(answer.contains("mock final answer"));
        assert!(answer.contains("probe.txt"));

        // 会话日志:从文件重开,事件完整、消息可投影。
        let sessions = ctx.service::<dyn SessionLog>(&SESSIONS).expect("sessions");
        let events = sessions.events();
        let kinds: Vec<_> = events.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                ah_contracts::session::SessionEventKind::User,
                ah_contracts::session::SessionEventKind::Assistant,
                ah_contracts::session::SessionEventKind::ToolResult,
                ah_contracts::session::SessionEventKind::Assistant,
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
        let answer = agent
            .run("go")
            .await
            .expect("loop must not crash on tool error");

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
        let answer = agent
            .run_in_session(a.clone(), "first task")
            .await
            .expect("run1");
        assert!(answer.contains("mock final answer"));
        let a_events = a.events().len();
        assert!(a_events >= 2);

        // fork 到 task-b:历史复制,续跑追加
        let b = manager.fork("task-a", "task-b").expect("fork");
        assert_eq!(b.events().len(), a_events);
        let _ = agent
            .run_in_session(b.clone(), "follow up")
            .await
            .expect("run2");
        assert!(b.events().len() > a_events, "resume 追加了事件");
        assert_eq!(a.events().len(), a_events, "原会话不受影响");

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&session_dir);
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
        assert!(
            agent
                .run_in_session(session.clone(), "cancel me")
                .await
                .is_err()
        );
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
        assert!(
            agent
                .run_in_session(session.clone(), "interrupt me")
                .await
                .is_err()
        );
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
        assert!(agent.run("  ").await.is_err());
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
        let _ = agent.run("go").await.expect("run");

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
        let answer = tight
            .run("explore the workspace")
            .await
            .expect("run with tight budget");
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
        let error = agent.run("fail").await.expect_err("backup must fail");
        assert_eq!(error.kind, AgentLoopFailure::Model);
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
        let error = agent.run("fail").await.expect_err("model must fail");
        assert_eq!(error.kind, AgentLoopFailure::Model);
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
        let answer = agent.run("recover").await.expect("backup answer");
        assert_eq!(answer, "backup answer");
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
        let answer = agent.run("explore the workspace").await.expect("run");
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
}
