//! # ah-plugins-subagent
//!
//! 真实子代理:在隔离会话中,用预算受限的 agent 循环执行子任务。
//! 上下文经 system 消息注入(日志投影支持);提供 delegate_task 工具,
//! 让模型能把子任务委派给新会话。

use std::future::Future;
use std::sync::Arc;

use ah_contracts::agent::{AgentControl, InterruptRuntime};
use ah_contracts::context::ContextEngine;
use ah_contracts::keys::{CONTEXT, INTERRUPT, LLM, SESSION_MANAGER, SESSIONS, SUBAGENT, TOOLS};
use ah_contracts::llm::{ModelProvider, ModelRequest, ToolSchema};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::session::{SessionEventKind, SessionLog, SessionManager};
use ah_contracts::subagent::{SubagentError, SubagentResult, SubagentRuntime, SubagentSpec};
use ah_contracts::tools::{Tool, ToolError, ToolRegistry};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use serde_json::{Value, json};

/// 真实子代理运行时:隔离会话 + 预算受限循环。
pub struct LocalSubagentRuntime {
    llm: Arc<dyn ModelProvider>,
    tools: Arc<dyn ToolRegistry>,
    manager: Arc<dyn SessionManager>,
    /// 可选上下文引擎(context seam 消费方):组装请求时按预算压缩。
    context: Option<Arc<dyn ContextEngine>>,
    token_budget: usize,
    interrupt: Option<Arc<dyn InterruptRuntime>>,
}

impl LocalSubagentRuntime {
    /// Create a runtime with explicit model, tools, session manager and controls.
    pub fn new(
        llm: Arc<dyn ModelProvider>,
        tools: Arc<dyn ToolRegistry>,
        manager: Arc<dyn SessionManager>,
        interrupt: Option<Arc<dyn InterruptRuntime>>,
    ) -> Self {
        Self {
            llm,
            tools,
            manager,
            context: None,
            token_budget: 8192,
            interrupt,
        }
    }
    /// 挂载 context seam 消费(可选)。
    pub fn with_context(mut self, context: Arc<dyn ContextEngine>, token_budget: usize) -> Self {
        self.context = Some(context);
        self.token_budget = token_budget;
        self
    }

    fn tool_schemas(&self, allowed: Option<&Vec<String>>) -> Vec<ToolSchema> {
        let mut names = self.tools.names();
        names.sort();
        names
            .iter()
            .filter(|name| allowed.map(|allow| allow.contains(name)).unwrap_or(true))
            .filter_map(|name| {
                self.tools.get(name).map(|tool| ToolSchema {
                    name: tool.name().to_string(),
                    description: tool.description().to_string(),
                    parameters: tool.parameters(),
                })
            })
            .collect()
    }

    async fn race_control<R>(
        &self,
        session_id: &str,
        future: impl Future<Output = R>,
    ) -> Result<R, SubagentError> {
        let Some(interrupt) = &self.interrupt else {
            return Ok(future.await);
        };
        if interrupt.state(session_id) != AgentControl::Continue {
            return Err(SubagentError(format!(
                "subagent {}",
                control_message(interrupt.state(session_id))
            )));
        }
        tokio::select! {
            result = future => Ok(result),
            _ = async {
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                    if interrupt.state(session_id) != AgentControl::Continue { break; }
                }
            } => Err(SubagentError(format!("subagent {}", control_message(interrupt.state(session_id))))),
        }
    }
    fn record_control_event(&self, session: &dyn SessionLog, session_id: &str) {
        let Some(interrupt) = &self.interrupt else {
            return;
        };
        let control = interrupt.state(session_id);
        if control == AgentControl::Continue {
            return;
        }
        let _ = session.append(
            match control {
                AgentControl::Interrupt => SessionEventKind::AgentInterrupted,
                AgentControl::Cancel => SessionEventKind::AgentCanceled,
                AgentControl::Continue => SessionEventKind::AgentStep,
            },
            json!({ "error": control_message(control) }),
        );
    }
}

fn control_message(control: AgentControl) -> &'static str {
    match control {
        AgentControl::Interrupt => "interrupted; resume the session to continue",
        AgentControl::Cancel => "cancelled",
        AgentControl::Continue => "control raced unexpectedly",
    }
}
fn mobile_serial_from_context(context: Option<&str>) -> &str {
    context
        .and_then(|value| value.split_once("Android device serial: "))
        .map(|(_, value)| value.split('.').next().unwrap_or(value).trim())
        .filter(|value| !value.is_empty())
        .unwrap_or("emulator-5554")
}

fn scoped_mobile_arguments(name: &str, arguments: &Value, serial: &str) -> Value {
    if !matches!(
        name,
        "tap_coordinate"
            | "double_tap_coordinate"
            | "long_press_coordinate"
            | "drag_coordinate"
            | "type_text"
            | "scroll"
            | "press_back"
            | "press_home"
            | "press_enter"
            | "wait_gui_load"
            | "screenshot"
            | "device_health"
    ) {
        return arguments.clone();
    }
    let Value::Object(mut object) = arguments.clone() else {
        return arguments.clone();
    };
    object.insert("device_serial".into(), Value::String(serial.to_string()));
    Value::Object(object)
}

fn observation_payload(output: &Value, serial: &str) -> Option<Value> {
    let mime_type = output.get("mime_type").and_then(Value::as_str)?;
    let data = output.get("data").and_then(Value::as_str)?;
    if !mime_type.starts_with("image/") || data.is_empty() {
        return None;
    }
    let mut payload = json!({
        "content": "[current Android screen]",
        "device_serial": serial,
        "images": [{"mime_type": mime_type, "data": data}]
    });
    if let Some(foreground_app) = output.get("foreground_app").and_then(Value::as_str) {
        payload["foreground_app"] = json!(foreground_app);
    }
    Some(payload)
}

fn image_attachments(output: &Value) -> Vec<Value> {
    let mut images = output
        .get("images")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if images.is_empty()
        && output
            .get("mime_type")
            .and_then(Value::as_str)
            .is_some_and(|mime| mime.starts_with("image/"))
        && output.get("data").and_then(Value::as_str).is_some()
    {
        images.push(json!({
            "mime_type": output["mime_type"].clone(),
            "data": output["data"].clone(),
        }));
    }
    images
        .into_iter()
        .filter(|image| {
            image
                .get("mime_type")
                .and_then(Value::as_str)
                .is_some_and(|mime| mime.starts_with("image/"))
                && image
                    .get("data")
                    .and_then(Value::as_str)
                    .is_some_and(|data| !data.is_empty())
        })
        .collect()
}

impl Seam for LocalSubagentRuntime {}

#[async_trait]
impl SubagentRuntime for LocalSubagentRuntime {
    async fn run(&self, spec: SubagentSpec) -> Result<SubagentResult, SubagentError> {
        let max_iterations = spec.budget.unwrap_or(8);
        // 隔离会话:每个子任务一个新会话,历史互不影响。
        let session = self
            .manager
            .create(&spec.id)
            .map_err(|e| SubagentError(format!("create session failed: {e}")))?;

        // 上下文经 system 消息注入(日志投影映射为 ChatMessage::System)。
        if let Some(context) = &spec.context {
            session
                .append(SessionEventKind::System, json!({ "content": context }))
                .map_err(|e| SubagentError(format!("append system failed: {e}")))?;
        }
        session
            .append(SessionEventKind::User, json!({ "content": spec.task }))
            .map_err(|e| SubagentError(format!("append user failed: {e}")))?;

        let mobile_observation = spec
            .allowed_tools
            .as_ref()
            .is_some_and(|allowed| allowed.iter().any(|name| name == "screenshot"));
        if mobile_observation {
            let serial = mobile_serial_from_context(spec.context.as_deref());
            let health = self
                .race_control(
                    &spec.id,
                    self.tools.invoke(
                        "device_health",
                        scoped_mobile_arguments("device_health", &json!({}), serial),
                    ),
                )
                .await?
                .map_err(|error| {
                    SubagentError(format!("Android device health check failed: {error}"))
                })?;
            if health.get("ok").and_then(Value::as_bool) != Some(true) {
                return Err(SubagentError(
                    "Android device health check returned not-ready".into(),
                ));
            }
            let screenshot = self
                .race_control(
                    &spec.id,
                    self.tools.invoke(
                        "screenshot",
                        scoped_mobile_arguments("screenshot", &json!({}), serial),
                    ),
                )
                .await?
                .map_err(|error| {
                    SubagentError(format!("initial Android screenshot failed: {error}"))
                })?;
            let payload = observation_payload(&screenshot, serial).ok_or_else(|| {
                SubagentError("initial Android screenshot returned no image payload".into())
            })?;
            session
                .append(SessionEventKind::User, payload)
                .map_err(|e| SubagentError(format!("append Android screenshot failed: {e}")))?;
        }

        if let Some(interrupt) = &self.interrupt {
            let control = interrupt.state(&spec.id);
            if control != AgentControl::Continue {
                let _ = session.append(
                    match control {
                        AgentControl::Interrupt => SessionEventKind::AgentInterrupted,
                        AgentControl::Cancel => SessionEventKind::AgentCanceled,
                        AgentControl::Continue => SessionEventKind::AgentStep,
                    },
                    json!({ "error": control_message(control) }),
                );
                return Err(SubagentError(format!(
                    "subagent {}",
                    control_message(control)
                )));
            }
        }
        for (index, _) in (0..max_iterations).enumerate() {
            let iterations_used = index + 1;
            // 模型可见消息:优先经 context seam 按预算组装;未挂载时日志投影直通。
            let messages = if let Some(context) = &self.context {
                context
                    .assemble(session.as_ref(), self.token_budget)
                    .await
                    .map_err(|e| SubagentError(format!("context assemble failed: {e}")))?
                    .messages
            } else {
                session.derive_messages()
            };
            let response = match self
                .race_control(
                    &spec.id,
                    self.llm.chat(ModelRequest {
                        messages,
                        tools: self.tool_schemas(spec.allowed_tools.as_ref()),
                        ..Default::default()
                    }),
                )
                .await
            {
                Ok(result) => result.map_err(|e| SubagentError(format!("model error: {e}")))?,
                Err(error) => {
                    self.record_control_event(session.as_ref(), &spec.id);
                    return Err(error);
                }
            };

            if response.tool_calls.is_empty() {
                session
                    .append(
                        SessionEventKind::Assistant,
                        json!({ "content": response.content }),
                    )
                    .map_err(|e| SubagentError(format!("append failed: {e}")))?;
                // 日志即真相:记录本轮步进(完成)。
                session
                    .append(
                        SessionEventKind::AgentStep,
                        json!({ "iteration": iterations_used, "tool_calls": 0, "done": true }),
                    )
                    .map_err(|e| SubagentError(format!("append step failed: {e}")))?;
                return Ok(SubagentResult {
                    answer: response.content,
                    iterations_used,
                });
            }

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
                .map_err(|e| SubagentError(format!("append failed: {e}")))?;

            for call in &response.tool_calls {
                let allowed = spec
                    .allowed_tools
                    .as_ref()
                    .map(|allow| allow.contains(&call.name))
                    .unwrap_or(true);
                let (status, output, images) = if !allowed {
                    (
                        "error",
                        format!("tool not allowed: {}", call.name),
                        Vec::new(),
                    )
                } else {
                    let tool_arguments = if mobile_observation {
                        scoped_mobile_arguments(
                            &call.name,
                            &call.arguments,
                            mobile_serial_from_context(spec.context.as_deref()),
                        )
                    } else {
                        call.arguments.clone()
                    };
                    let tool_result = match self
                        .race_control(&spec.id, self.tools.invoke(&call.name, tool_arguments))
                        .await
                    {
                        Ok(result) => result,
                        Err(error) => {
                            self.record_control_event(session.as_ref(), &spec.id);
                            return Err(error);
                        }
                    };
                    match tool_result {
                        Ok(value) => {
                            let images = image_attachments(&value);
                            ("completed", value.to_string(), images)
                        }
                        Err(error) => ("error", format!("tool error: {error}"), Vec::new()),
                    }
                };
                let mut event = json!({
                    "tool_call_id": call.id,
                    "status": status,
                    "output": output,
                });
                if !images.is_empty() {
                    event["images"] = json!(images);
                }
                session
                    .append(SessionEventKind::ToolResult, event)
                    .map_err(|e| SubagentError(format!("append failed: {e}")))?;
                if mobile_observation
                    && status == "completed"
                    && call.name != "screenshot"
                    && call.name != "device_health"
                {
                    let serial = mobile_serial_from_context(spec.context.as_deref());
                    let screenshot = self
                        .race_control(
                            &spec.id,
                            self.tools.invoke(
                                "screenshot",
                                scoped_mobile_arguments("screenshot", &json!({}), serial),
                            ),
                        )
                        .await?
                        .map_err(|error| {
                            SubagentError(format!("post-action Android screenshot failed: {error}"))
                        })?;
                    let payload = observation_payload(&screenshot, serial).ok_or_else(|| {
                        SubagentError(
                            "post-action Android screenshot returned no image payload".into(),
                        )
                    })?;
                    session
                        .append(SessionEventKind::User, payload)
                        .map_err(|e| {
                            SubagentError(format!(
                                "append post-action Android screenshot failed: {e}"
                            ))
                        })?;
                }
            }
            // 日志即真相:记录本轮步进(继续循环)。
            session
                .append(
                    SessionEventKind::AgentStep,
                    json!({
                        "iteration": iterations_used,
                        "tool_calls": response.tool_calls.len(),
                        "done": false,
                    }),
                )
                .map_err(|e| SubagentError(format!("append step failed: {e}")))?;
        }
        Err(SubagentError(format!(
            "subagent budget exceeded: {max_iterations}"
        )))
    }
}

/// delegate_task 工具:委派子任务到隔离会话。
pub struct DelegateTaskTool {
    runtime: Arc<dyn SubagentRuntime>,
}

impl DelegateTaskTool {
    pub fn new(runtime: Arc<dyn SubagentRuntime>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl Tool for DelegateTaskTool {
    fn name(&self) -> &'static str {
        "delegate_task"
    }

    fn description(&self) -> &'static str {
        "delegate a subtask to an isolated subagent; arguments: {task, context?}"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task": { "type": "string" },
                "context": { "type": "string" },
            },
            "required": ["task"],
        })
    }

    async fn invoke(&self, arguments: Value) -> Result<Value, ToolError> {
        let task = arguments
            .get("task")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError("missing string field task".to_string()))?;
        let context = arguments
            .get("context")
            .and_then(Value::as_str)
            .map(str::to_string);
        let result = self
            .runtime
            .run(SubagentSpec {
                id: format!(
                    "sub-{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis())
                        .unwrap_or(0)
                ),
                task: task.to_string(),
                context,
                budget: Some(6),
                allowed_tools: None,
            })
            .await
            .map_err(|e| ToolError(format!("subagent failed: {e}")))?;
        Ok(json!({
            "answer": result.answer,
            "iterations_used": result.iterations_used,
        }))
    }
}

/// 子代理插件:提供 subagent seam,并注册 delegate_task 工具。
pub struct SubagentPlugin;

impl Plugin for SubagentPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-subagent"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![SUBAGENT]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![LLM, TOOLS, SESSIONS, SESSION_MANAGER]
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
        let manager = ctx
            .service::<dyn SessionManager>(&SESSION_MANAGER)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "session-manager seam not registered".to_string(),
            })?;
        let interrupt = ctx.service::<dyn InterruptRuntime>(&INTERRUPT);

        let context = ctx.service::<dyn ContextEngine>(&CONTEXT);
        let mut runtime = LocalSubagentRuntime {
            llm,
            tools,
            manager,
            context: None,
            token_budget: 8192,
            interrupt,
        };
        if let Some(context) = context {
            runtime = runtime.with_context(context, 8192);
        }
        let runtime: Arc<dyn SubagentRuntime> = Arc::new(runtime);
        let mut effects = vec![ctx.register(SUBAGENT, runtime.clone())];

        let registry =
            ctx.service::<dyn ToolRegistry>(&TOOLS)
                .ok_or_else(|| PluginError::Apply {
                    plugin: self.name(),
                    message: "tools seam not registered".to_string(),
                })?;
        effects.push(registry.register(Arc::new(DelegateTaskTool::new(runtime))));
        Ok(effects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            StdArc::new(ah_plugins_agent_control::AgentControlPlugin),
            StdArc::new(ah_plugins_session_log::SessionLogPlugin::new(
                &default_path,
                &session_dir,
            )),
            StdArc::new(SubagentPlugin),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }
    struct SlowModel;

    impl Seam for SlowModel {}

    #[async_trait]
    impl ah_contracts::llm::ModelProvider for SlowModel {
        fn name(&self) -> &'static str {
            "slow-test-model"
        }

        async fn chat(
            &self,
            _request: ah_contracts::llm::ModelRequest,
        ) -> Result<ah_contracts::llm::ModelResponse, ah_contracts::llm::ModelError> {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            Ok(ah_contracts::llm::ModelResponse {
                content: "finished".into(),
                tool_calls: Vec::new(),
                reasoning_content: None,
            })
        }
    }

    #[test]
    fn tool_image_outputs_are_extracted_for_session_projection() {
        let output = json!({
            "ok": true,
            "images": [{"mime_type": "image/png", "data": "data:image/png;base64,AAAA"}]
        });
        let images = image_attachments(&output);
        assert_eq!(images.len(), 1);
        assert_eq!(images[0]["mime_type"], "image/png");
        assert_eq!(images[0]["data"], "data:image/png;base64,AAAA");
    }
    #[tokio::test]
    async fn subagent_runs_isolated_task_with_real_tools() {
        let root = std::env::temp_dir().join(format!("ah-subagent-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);

        // 预写一个文件,子代理的 list_dir 能看到。
        let fs = ctx
            .service::<dyn ah_contracts::fs::FsProvider>(&ah_contracts::keys::FS)
            .expect("fs");
        fs.write("probe.txt", b"x").expect("write");

        let runtime = ctx
            .service::<dyn SubagentRuntime>(&SUBAGENT)
            .expect("subagent");
        let result = runtime
            .run(SubagentSpec {
                id: "sub-1".to_string(),
                task: "explore".to_string(),
                context: Some("You are a subagent. Be concise.".to_string()),
                budget: Some(4),
                allowed_tools: None,
            })
            .await
            .expect("subagent run");

        // 真实工具结果被引用。
        assert!(result.answer.contains("mock final answer"));
        assert!(result.answer.contains("probe.txt"));
        assert!(result.iterations_used >= 2);

        // 隔离会话:manager 里有 sub-1,且包含 system 上下文。
        let manager = ctx
            .service::<dyn SessionManager>(&SESSION_MANAGER)
            .expect("manager");
        assert!(manager.list().contains(&"sub-1".to_string()));
        let sub_session = manager.open("sub-1").expect("open");
        let messages = sub_session.derive_messages();
        assert_eq!(
            messages[0].role,
            ah_contracts::llm::ChatRole::System,
            "上下文经 system 注入"
        );
        assert!(messages[0].content.contains("subagent"));

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn delegate_task_tool_returns_subagent_answer() {
        let root = std::env::temp_dir().join(format!("ah-subagent-tool-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let registry = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");

        let mut names = registry.names();
        names.sort();
        assert!(names.contains(&"delegate_task".to_string()));

        let result = registry
            .invoke("delegate_task", json!({ "task": "do something" }))
            .await
            .expect("delegate");
        assert!(
            result["answer"]
                .as_str()
                .unwrap_or_default()
                .contains("mock final answer")
        );

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
    #[tokio::test]
    async fn cancellation_is_recorded_and_stops_before_model_call() {
        let root = std::env::temp_dir().join(format!("ah-subagent-cancel-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let interrupt = ctx
            .service::<dyn InterruptRuntime>(&INTERRUPT)
            .expect("interrupt");
        interrupt
            .request("cancelled-child", AgentControl::Cancel)
            .await
            .unwrap();
        let runtime = ctx
            .service::<dyn SubagentRuntime>(&SUBAGENT)
            .expect("subagent");
        let error = runtime
            .run(SubagentSpec {
                id: "cancelled-child".into(),
                task: "must not run".into(),
                context: None,
                budget: Some(2),
                allowed_tools: None,
            })
            .await
            .expect_err("cancelled child");
        assert!(error.0.contains("cancelled"));
        let manager = ctx
            .service::<dyn SessionManager>(&SESSION_MANAGER)
            .expect("manager");
        assert!(
            manager
                .open("cancelled-child")
                .unwrap()
                .events()
                .iter()
                .any(|event| { event.kind == SessionEventKind::AgentCanceled })
        );
        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
    #[tokio::test]
    async fn interruption_during_model_call_is_recorded() {
        let root = std::env::temp_dir().join(format!("ah-subagent-race-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let interrupt = ctx
            .service::<dyn InterruptRuntime>(&INTERRUPT)
            .expect("interrupt");
        let manager = ctx
            .service::<dyn SessionManager>(&SESSION_MANAGER)
            .expect("manager");
        let tools = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");
        let runtime = LocalSubagentRuntime {
            llm: StdArc::new(SlowModel),
            tools,
            manager: manager.clone(),
            context: None,
            token_budget: 8192,
            interrupt: Some(interrupt.clone()),
        };
        let task = tokio::spawn(async move {
            runtime
                .run(SubagentSpec {
                    id: "interrupted-child".into(),
                    task: "wait for interruption".into(),
                    context: None,
                    budget: Some(2),
                    allowed_tools: None,
                })
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        interrupt
            .request("interrupted-child", AgentControl::Interrupt)
            .await
            .expect("request interruption");
        let error = task.await.expect("join").expect_err("interrupted child");
        assert!(error.0.contains("interrupted"));
        assert!(
            manager
                .open("interrupted-child")
                .expect("session")
                .events()
                .iter()
                .any(|event| event.kind == SessionEventKind::AgentInterrupted)
        );
        drop(effects);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn screenshot_tool_output_becomes_model_observation() {
        let payload = observation_payload(
            &json!({"mime_type":"image/png","data":"data:image/png;base64,AAAA"}),
            "emulator-5554",
        )
        .expect("image observation");
        assert_eq!(payload["content"], "[current Android screen]");
        assert_eq!(payload["images"][0]["mime_type"], "image/png");
        assert_eq!(payload["device_serial"], "emulator-5554");
    }
}
