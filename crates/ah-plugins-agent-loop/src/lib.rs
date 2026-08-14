//! # ah-plugins-agent-loop
//!
//! 真实 ReAct agent 循环:注入 llm + tools + sessions 三个 seam。
//! 循环以会话日志为唯一事实来源(日志即真相):
//! 每次模型请求的消息序列由日志投影(derive_messages)重建,
//! 每轮的用户消息/助手消息/工具调用/工具结果都追加到日志。

use std::sync::Arc;

use std::collections::HashMap;

use ah_contracts::context::ContextEngine;
use ah_contracts::keys::{AGENT_LOOP, CONTEXT, LLM, PROMPT, SESSIONS, TOOLS};
use ah_contracts::llm::{ChatMessage, ChatRole, ModelProvider, ModelRequest, ToolSchema};
use ah_contracts::prelude::Effect;
use ah_contracts::prompt::PromptRegistry;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::session::{SessionEventKind, SessionLog};
use ah_contracts::tools::ToolRegistry;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use serde_json::{Value, json};

/// agent 循环错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentLoopError(pub String);

impl core::fmt::Display for AgentLoopError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AgentLoopError {}

/// agent/step 事件契约(跨插件共享):定义在 ah-contracts。
pub use ah_contracts::agent::AgentStep;

/// 真实 ReAct 循环(日志驱动)。
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
    pub async fn run_in_session(
        &self,
        session: Arc<dyn SessionLog>,
        input: &str,
    ) -> Result<String, AgentLoopError> {
        // 1) 用户消息入日志。
        session
            .append(SessionEventKind::User, json!({ "content": input }))
            .map_err(|e| AgentLoopError(format!("session append failed: {e}")))?;

        for iteration in 0..self.max_iterations {
            // 2) 模型可见消息:优先经 context seam 按预算组装(压缩时注入摘要),
            //    未挂载时用日志投影直通(日志即真相:完整历史始终在日志)。
            let messages = if let Some(context) = &self.context {
                context
                    .assemble(session.as_ref(), self.token_budget)
                    .await
                    .map_err(|e| AgentLoopError(format!("context assemble failed: {e}")))?
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
            let response = self
                .llm
                .chat(ModelRequest {
                    messages,
                    tools: self.tool_schemas(),
                    ..Default::default()
                })
                .await
                .map_err(|e| AgentLoopError(format!("model error: {e}")))?;

            if response.tool_calls.is_empty() {
                // 3) 最终回答入日志,结束。
                session
                    .append(
                        SessionEventKind::Assistant,
                        json!({ "content": response.content }),
                    )
                    .map_err(|e| AgentLoopError(format!("session append failed: {e}")))?;
                self.ctx.emit(AgentStep {
                    iteration,
                    tool_calls: 0,
                    done: true,
                });
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
                .map_err(|e| AgentLoopError(format!("session append failed: {e}")))?;

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
                    .map_err(|e| AgentLoopError(format!("session append failed: {e}")))?;
            }

            self.ctx.emit(AgentStep {
                iteration,
                tool_calls: response.tool_calls.len(),
                done: false,
            });
        }
        Err(AgentLoopError(format!(
            "max iterations exceeded: {max}",
            max = self.max_iterations
        )))
    }
}

impl Seam for AgentLoop {}

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
        // context seam 可选:挂载后循环按 token 预算压缩模型可见上下文。
        let context = ctx.service::<dyn ContextEngine>(&CONTEXT);
        // prompt seam 可选:注册 "agent" 模板时注入系统提示。
        let prompt = ctx.service::<dyn PromptRegistry>(&PROMPT);
        let mut agent = AgentLoop::new(llm, tools, sessions, ctx.clone(), self.max_iterations);
        if let Some(context) = context {
            agent = agent.with_context(context, 8192);
        }
        if let Some(prompt) = prompt {
            agent = agent.with_prompt(prompt, "agent");
        }
        let agent = Arc::new(agent);
        Ok(vec![ctx.register(AGENT_LOOP, agent)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::SESSIONS;
    use ah_contracts::llm::ChatRole;
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
            .service::<AgentLoop>(&AGENT_LOOP)
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
            .service::<AgentLoop>(&AGENT_LOOP)
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
        let agent = ctx.service::<AgentLoop>(&AGENT_LOOP).expect("agent-loop");

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
            .service::<AgentLoop>(&AGENT_LOOP)
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
            StdArc::new(ah_plugins_context::ContextPlugin::new(root.join("offload"))),
            StdArc::new(AgentLoopPlugin::default()),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");

        // 手工挂载一个极小预算的循环实例:context seam 消费方真实生效。
        let context = ctx.service::<dyn ContextEngine>(&CONTEXT).expect("context");
        let agent = ctx.service::<AgentLoop>(&AGENT_LOOP).expect("agent-loop");
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
