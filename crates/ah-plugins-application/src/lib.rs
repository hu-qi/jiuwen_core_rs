use std::sync::Arc;

use ah_contracts::agent::{
    AgentControlError, AgentLoopRuntime, AgentRequest, AgentResult, AgentRunConfig, AgentRunState,
    ApplicationRuntime,
};
use ah_contracts::controller::{Controller, Intent, IntentType, Task, TaskStatus};
use ah_contracts::keys::{
    AGENT_LOOP, APPLICATION, CONTROLLER, LLM, MEMORY, SESSION_MANAGER, WORKFLOW,
};
use ah_contracts::llm::ModelProvider;
use ah_contracts::memory::MemoryProvider;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::session::{SessionEventKind, SessionManager};
use ah_contracts::workflow::{WorkflowEngine, WorkflowSpec, WorkflowStreamSink};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;

pub struct LocalApplicationRuntime {
    agent: Arc<dyn AgentLoopRuntime>,
    workflow: Arc<dyn WorkflowEngine>,
    sessions: Arc<dyn SessionManager>,
    controller: Option<Arc<dyn Controller>>,
    ctx: Context,
}
impl Seam for LocalApplicationRuntime {}

fn extract_task_id_from_natural_language(input: &str) -> Option<&str> {
    input.split_whitespace().find_map(|token| {
        let candidate =
            token.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_');
        (candidate.starts_with("task-") && candidate.len() > "task-".len()).then_some(candidate)
    })
}

fn intent_from_command(command: &serde_json::Value) -> Result<Intent, AgentControlError> {
    let object = command
        .as_object()
        .ok_or_else(|| AgentControlError("command must be a JSON object".to_string()))?;
    let intent_type = object
        .get("intent_type")
        .or_else(|| object.get("type"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| AgentControlError("command requires intent_type".to_string()))?;
    let intent_type =
        serde_json::from_value::<IntentType>(serde_json::Value::String(intent_type.to_string()))
            .map_err(|_| AgentControlError(format!("unknown command intent: {intent_type}")))?;
    let task_text = object
        .get("task_text")
        .or_else(|| object.get("description"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let task_id = object
        .get("task_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let confidence = object
        .get("confidence")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(1.0);
    if !(0.0..=1.0).contains(&confidence) {
        return Err(AgentControlError(
            "command confidence must be between 0 and 1".to_string(),
        ));
    }
    Ok(Intent {
        intent_type,
        task_text,
        task_id,
        confidence,
    })
}

fn memory_context(memory: &dyn MemoryProvider, user_id: &str, query: &str) -> Option<String> {
    let user_prefix = format!("{user_id}:");
    let records = memory
        .search(query)
        .into_iter()
        .filter(|record| record.key.starts_with(&user_prefix))
        .collect::<Vec<_>>();
    if records.is_empty() {
        return None;
    }
    let content = records
        .into_iter()
        .map(|record| format!("{}: {}", record.key, record.content))
        .collect::<Vec<_>>()
        .join("\n");
    Some(format!("Relevant long-term memory:\n{content}"))
}

fn persist_memory(
    memory: &dyn MemoryProvider,
    user_id: &str,
    session_id: &str,
    input: &str,
    answer: &str,
) {
    if answer.trim().is_empty() {
        return;
    }
    let key = format!("{user_id}:session:{session_id}");
    let content = format!("user: {input}\nassistant: {answer}");
    let _ = memory.store(&key, &content, vec!["conversation".to_string()]);
}

#[async_trait]
impl ApplicationRuntime for LocalApplicationRuntime {
    async fn invoke(&self, request: AgentRequest) -> Result<AgentResult, AgentControlError> {
        if request.session_id.trim().is_empty() || request.input.trim().is_empty() {
            return Err(AgentControlError(
                "session_id and input must not be empty".to_string(),
            ));
        }
        let session = if let Some(checkpoint) = request.restore_checkpoint.as_deref() {
            if checkpoint.trim().is_empty() {
                return Err(AgentControlError(
                    "restore_checkpoint must not be empty".to_string(),
                ));
            }
            self.sessions
                .restore(&request.session_id, checkpoint)
                .map_err(|e| AgentControlError(format!("session restore failed: {e}")))?
        } else {
            self.sessions
                .open(&request.session_id)
                .or_else(|_| self.sessions.create(&request.session_id))
                .map_err(|e| AgentControlError(format!("session open failed: {e}")))?
        };
        let controller = self
            .controller
            .clone()
            .or_else(|| self.ctx.service::<dyn Controller>(&CONTROLLER));
        if let Some(controller) = controller {
            let intent = if let Some(command) = request.command.as_ref() {
                intent_from_command(command)?
            } else if let Some(llm) = self.ctx.service::<dyn ModelProvider>(&LLM) {
                match controller
                    .recognize_intent_with_llm(&request.input, llm.clone())
                    .await
                {
                    Ok(intent) => intent,
                    Err(error) if llm.name() == "mock" => {
                        controller.recognize_intent(&request.input)
                    }
                    Err(error) => {
                        return Err(AgentControlError(format!(
                            "intent recognition failed: {error}"
                        )));
                    }
                }
            } else {
                controller.recognize_intent(&request.input)
            };
            let command_task_id = request
                .command
                .as_ref()
                .and_then(|command| command.get("task_id"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            if intent.intent_type == IntentType::CreateTask {
                let task_id = intent
                    .task_id
                    .clone()
                    .or(command_task_id.clone())
                    .ok_or_else(|| {
                        AgentControlError("create_task command requires task_id".to_string())
                    })?;
                let command = request.command.as_ref();
                let task_type = command
                    .and_then(|value| value.get("task_type"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("agent");
                let priority = command
                    .and_then(|value| value.get("priority"))
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(1);
                let mut task = Task::submitted(
                    &request.session_id,
                    &task_id,
                    task_type,
                    intent
                        .task_text
                        .clone()
                        .unwrap_or_else(|| request.input.clone()),
                    priority as i32,
                );
                if let Some(payload) = command.and_then(|value| value.get("payload")) {
                    task.payload = payload.clone();
                }
                let result = controller.create_task(task);
                let (state, answer, error) = match result {
                    Ok(_) => (
                        AgentRunState::Completed,
                        Some(format!("task {task_id} created")),
                        None,
                    ),
                    Err(error) => (AgentRunState::Failed, None, Some(error.0)),
                };
                session
                    .append(
                        SessionEventKind::System,
                        serde_json::json!({
                            "command": request.command,
                            "intent": "CreateTask",
                            "task_id": task_id,
                            "state": format!("{state:?}"),
                            "error": error,
                        }),
                    )
                    .map_err(|e| AgentControlError(format!("session append failed: {e}")))?;
                return Ok(AgentResult {
                    session_id: request.session_id,
                    state,
                    answer,
                    iterations: 0,
                    tool_calls: 0,
                    failure: None,
                    error,
                });
            }
            match intent.intent_type {
                IntentType::CancelTask
                | IntentType::PauseTask
                | IntentType::ResumeTask
                | IntentType::RetryTask => {
                    let task_id = match intent
                        .task_id
                        .clone()
                        .or(command_task_id)
                        .or_else(|| {
                            request.input.split_once(':').and_then(|(_, id)| {
                                (!id.trim().is_empty()).then_some(id.trim().to_string())
                            })
                        })
                        .or_else(|| {
                            extract_task_id_from_natural_language(&request.input)
                                .map(str::to_string)
                        }) {
                        Some(task_id) => task_id,
                        None => {
                            let error = "controller command requires task id after ':'";
                            session
                                .append(
                                    SessionEventKind::System,
                                    serde_json::json!({
                                        "command": request.input,
                                        "intent": format!("{:?}", intent.intent_type),
                                        "state": "Failed",
                                        "error": error,
                                    }),
                                )
                                .map_err(|e| {
                                    AgentControlError(format!("session append failed: {e}"))
                                })?;
                            return Err(AgentControlError(error.to_string()));
                        }
                    };
                    let result = match intent.intent_type {
                        IntentType::CancelTask => controller.cancel_task(&task_id).await,
                        IntentType::PauseTask => controller
                            .update_status(&task_id, TaskStatus::Paused)
                            .map(|_| ()),
                        IntentType::ResumeTask => {
                            match controller.update_status(&task_id, TaskStatus::Submitted) {
                                Ok(()) => controller.run_task(&task_id).await.map(|_| ()),
                                Err(error) => Err(error),
                            }
                        }
                        IntentType::RetryTask => controller.retry_task(&task_id),
                        _ => unreachable!(),
                    };
                    let (state, answer, error) = match result {
                        Ok(()) => (
                            AgentRunState::Completed,
                            Some(format!("task {task_id} updated")),
                            None,
                        ),
                        Err(error) => (AgentRunState::Failed, None, Some(error.0)),
                    };
                    session
                        .append(
                            SessionEventKind::System,
                            serde_json::json!({
                                "command": request.command.as_ref().unwrap_or(&serde_json::Value::String(request.input.clone())),
                                "intent": format!("{:?}", intent.intent_type),
                                "task_id": task_id,
                                "state": format!("{state:?}"),
                                "error": error,
                            }),
                        )
                        .map_err(|e| AgentControlError(format!("session append failed: {e}")))?;
                    return Ok(AgentResult {
                        session_id: request.session_id,
                        state,
                        answer,
                        iterations: 0,
                        tool_calls: 0,
                        failure: None,
                        error,
                    });
                }
                _ => {}
            }
        }
        let system_context = request
            .user_id
            .as_deref()
            .and_then(|user_id| {
                self.ctx
                    .service::<dyn MemoryProvider>(&MEMORY)
                    .map(|memory| (user_id, memory))
            })
            .and_then(|(user_id, memory)| memory_context(memory.as_ref(), user_id, &request.input));
        if let Some(workflow) = request.workflow {
            let spec: WorkflowSpec = serde_json::from_value(workflow)
                .map_err(|e| AgentControlError(format!("invalid workflow: {e}")))?;
            let output = self
                .workflow
                .run(&spec, serde_json::json!({"input": request.input}))
                .await
                .map_err(|e| AgentControlError(e.0))?;
            session
                .append(
                    SessionEventKind::User,
                    serde_json::json!({"content": request.input}),
                )
                .map_err(|e| AgentControlError(format!("session append failed: {e}")))?;
            session
                .append(
                    SessionEventKind::Assistant,
                    serde_json::json!({"content": output.output.to_string()}),
                )
                .map_err(|e| AgentControlError(format!("session append failed: {e}")))?;
            let answer = output.output.to_string();
            if let (Some(user_id), Some(memory)) = (
                request.user_id.as_deref(),
                self.ctx.service::<dyn MemoryProvider>(&MEMORY),
            ) {
                persist_memory(
                    memory.as_ref(),
                    user_id,
                    &request.session_id,
                    &request.input,
                    &answer,
                );
            }
            return Ok(AgentResult {
                session_id: request.session_id,
                state: AgentRunState::Completed,
                answer: Some(answer),
                iterations: output.executed.len(),
                tool_calls: 0,
                failure: None,
                error: None,
            });
        }
        let session_id = request.session_id.clone();
        // agent 循环返回结构化 AgentResult(P1-02):状态/终止原因/统计直接可用,
        // 不再解析错误字符串判定 interrupted/cancelled/timed out。
        let result = self
            .agent
            .run_in_session_with_config(
                session.clone(),
                &request.input,
                AgentRunConfig {
                    timeout_ms: request.timeout_ms,
                    model: request.model.clone(),
                    temperature: request.temperature,
                    system_context,
                    identity: request.user_id.clone(),
                },
            )
            .await;
        if result.state != AgentRunState::Completed {
            // 失败/中断/取消/超时:结构化状态写入 System 事件,原样返回 result。
            if let Err(log_error) = session.append(
                SessionEventKind::System,
                serde_json::json!({
                    "command": request.input,
                    "state": format!("{:?}", result.state).to_lowercase(),
                    "failure": result
                        .failure
                        .map(|f| format!("{:?}", f).to_lowercase())
                        .unwrap_or_default(),
                    "error": result.error,
                    "iterations": result.iterations,
                    "tool_calls": result.tool_calls,
                }),
            ) {
                return Ok(AgentResult {
                    session_id,
                    state: AgentRunState::Failed,
                    answer: None,
                    iterations: result.iterations,
                    tool_calls: result.tool_calls,
                    failure: Some(ah_contracts::agent::AgentFailure::Session),
                    error: Some(format!(
                        "{}; failure event logging failed: {log_error}",
                        result.error.unwrap_or_default()
                    )),
                });
            }
            return Ok(result);
        }
        if let (Some(user_id), Some(answer), Some(memory)) = (
            request.user_id.as_deref(),
            result.answer.as_deref(),
            self.ctx.service::<dyn MemoryProvider>(&MEMORY),
        ) {
            persist_memory(
                memory.as_ref(),
                user_id,
                &request.session_id,
                &request.input,
                answer,
            );
        }
        Ok(AgentResult {
            session_id: request.session_id,
            state: AgentRunState::Completed,
            answer: result.answer,
            iterations: result.iterations,
            tool_calls: result.tool_calls,
            failure: None,
            error: None,
        })
    }
    async fn stream(
        &self,
        request: AgentRequest,
        sink: Arc<dyn WorkflowStreamSink>,
    ) -> Result<AgentResult, AgentControlError> {
        if request.session_id.trim().is_empty() || request.input.trim().is_empty() {
            return Err(AgentControlError(
                "session_id and input must not be empty".to_string(),
            ));
        }
        let session = if let Some(checkpoint) = request.restore_checkpoint.as_deref() {
            if checkpoint.trim().is_empty() {
                return Err(AgentControlError(
                    "restore_checkpoint must not be empty".to_string(),
                ));
            }
            self.sessions
                .restore(&request.session_id, checkpoint)
                .map_err(|e| AgentControlError(format!("session restore failed: {e}")))?
        } else {
            self.sessions
                .open(&request.session_id)
                .or_else(|_| self.sessions.create(&request.session_id))
                .map_err(|e| AgentControlError(format!("session open failed: {e}")))?
        };
        let workflow = request
            .workflow
            .ok_or_else(|| AgentControlError("stream requires a workflow".to_string()))?;
        let spec: WorkflowSpec = serde_json::from_value(workflow)
            .map_err(|e| AgentControlError(format!("invalid workflow: {e}")))?;
        let output = self
            .workflow
            .stream(&spec, serde_json::json!({"input": request.input}), sink)
            .await
            .map_err(|e| AgentControlError(e.0))?;
        session
            .append(
                SessionEventKind::User,
                serde_json::json!({"content": request.input}),
            )
            .map_err(|e| AgentControlError(format!("session append failed: {e}")))?;
        let answer = output.output.to_string();
        session
            .append(
                SessionEventKind::Assistant,
                serde_json::json!({"content": answer}),
            )
            .map_err(|e| AgentControlError(format!("session append failed: {e}")))?;
        if let (Some(user_id), Some(memory)) = (
            request.user_id.as_deref(),
            self.ctx.service::<dyn MemoryProvider>(&MEMORY),
        ) {
            persist_memory(
                memory.as_ref(),
                user_id,
                &request.session_id,
                &request.input,
                &answer,
            );
        }
        Ok(AgentResult {
            session_id: request.session_id,
            state: AgentRunState::Completed,
            answer: Some(answer),
            iterations: output.executed.len(),
            tool_calls: 0,
            failure: None,
            error: None,
        })
    }
}

pub struct ApplicationPlugin;

impl Plugin for ApplicationPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-application"
    }
    fn provides(&self) -> Vec<ServiceKey> {
        vec![APPLICATION]
    }
    fn inject(&self) -> Vec<ServiceKey> {
        vec![AGENT_LOOP, WORKFLOW, SESSION_MANAGER, LLM]
    }
    fn apply(&self, ctx: &Context) -> Result<Vec<ah_contracts::Effect>, PluginError> {
        let agent = ctx
            .service::<dyn AgentLoopRuntime>(&AGENT_LOOP)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "agent-loop seam not registered".to_string(),
            })?;
        let workflow = ctx
            .service::<dyn WorkflowEngine>(&WORKFLOW)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "workflow seam not registered".to_string(),
            })?;
        let sessions = ctx
            .service::<dyn SessionManager>(&SESSION_MANAGER)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "session-manager seam not registered".to_string(),
            })?;
        let runtime: Arc<dyn ApplicationRuntime> = Arc::new(LocalApplicationRuntime {
            agent,
            workflow,
            sessions,
            controller: None,
            ctx: ctx.clone(),
        });
        Ok(vec![ctx.register(APPLICATION, runtime)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::agent::ApplicationRuntime;
    use ah_contracts::keys::SESSION_MANAGER;
    use ah_contracts::session::{SessionEventKind, SessionManager};
    use ah_contracts::tools::ToolRegistry;
    use ah_contracts::workflow::{WorkflowError, WorkflowStreamSink};
    use ah_plugins_agent_loop::AgentLoop;

    struct RecordingApplicationSink {
        chunks: std::sync::Mutex<Vec<serde_json::Value>>,
        closed: std::sync::atomic::AtomicBool,
    }

    impl Seam for RecordingApplicationSink {}

    #[async_trait]
    impl WorkflowStreamSink for RecordingApplicationSink {
        async fn emit(&self, chunk: serde_json::Value) -> Result<(), WorkflowError> {
            self.chunks.lock().unwrap().push(chunk);
            Ok(())
        }

        async fn close(&self) -> Result<(), WorkflowError> {
            self.closed.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn streams_workflow_through_application_runtime() {
        let root = std::env::temp_dir().join(format!("ah-app-stream-{}", std::process::id()));
        let session_dir = root.join("sessions");
        let ctx = Context::new();
        let effects = ctx
            .mount_all(vec![
                Arc::new(ah_plugins_mock::MockPlugin),
                Arc::new(ah_plugins_tools::ToolsPlugin),
                Arc::new(ah_plugins_sysop::SysopPlugin::new(&root)),
                Arc::new(ah_plugins_session_log::SessionLogPlugin::new(
                    session_dir.join("default.jsonl"),
                    &session_dir,
                )),
                Arc::new(ah_plugins_agent_control::AgentControlPlugin),
                Arc::new(ah_plugins_model_backup::ModelBackupPlugin::new(Vec::new())),
                Arc::new(ah_plugins_workflow::WorkflowPlugin),
                Arc::new(ah_plugins_agent_loop::AgentLoopPlugin::default()),
                Arc::new(ApplicationPlugin),
            ])
            .expect("mount");
        let application = ctx
            .service::<dyn ApplicationRuntime>(&ah_contracts::keys::APPLICATION)
            .unwrap();
        let sink = Arc::new(RecordingApplicationSink {
            chunks: std::sync::Mutex::new(Vec::new()),
            closed: std::sync::atomic::AtomicBool::new(false),
        });
        let result = application
            .stream(
                AgentRequest {
                    session_id: "stream-session".into(),
                    input: "stream input".into(),
                    workflow: Some(serde_json::json!({
                        "id": "stream-workflow",
                        "nodes": [
                            {"id": "start", "kind": "start", "config": {}},
                            {"id": "end", "kind": "end", "config": {}}
                        ],
                        "edges": [{"from": "start", "to": "end"}]
                    })),
                    timeout_ms: None,
                    restore_checkpoint: None,
                    command: None,
                    user_id: None,
                    model: None,
                    temperature: None,
                },
                sink.clone(),
            )
            .await
            .expect("stream invoke");
        assert_eq!(result.state, AgentRunState::Completed);
        assert_eq!(
            result.answer.as_deref(),
            Some("{\"input\":\"stream input\"}")
        );
        assert!(sink.closed.load(std::sync::atomic::Ordering::SeqCst));
        assert!(
            sink.chunks
                .lock()
                .unwrap()
                .iter()
                .any(|chunk| chunk["type"] == "workflow_final")
        );
        drop(effects);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn routes_llm_request_into_named_persistent_session() {
        let root = std::env::temp_dir().join(format!("ah-app-route-{}", std::process::id()));
        let session_dir = root.join("sessions");
        let ctx = Context::new();
        let effects = ctx
            .mount_all(vec![
                Arc::new(ah_plugins_mock::MockPlugin),
                Arc::new(ah_plugins_tools::ToolsPlugin),
                Arc::new(ah_plugins_sysop::SysopPlugin::new(&root)),
                Arc::new(ah_plugins_session_log::SessionLogPlugin::new(
                    session_dir.join("default.jsonl"),
                    &session_dir,
                )),
                Arc::new(ah_plugins_agent_control::AgentControlPlugin),
                Arc::new(ah_plugins_model_backup::ModelBackupPlugin::new(Vec::new())),
                Arc::new(ah_plugins_workflow::WorkflowPlugin),
                Arc::new(ah_plugins_agent_loop::AgentLoopPlugin::default()),
                Arc::new(ApplicationPlugin),
            ])
            .expect("mount");
        let app = ctx
            .service::<dyn ApplicationRuntime>(&ah_contracts::keys::APPLICATION)
            .unwrap();
        let result = app
            .invoke(AgentRequest {
                session_id: "named".into(),
                input: "hello".into(),
                workflow: None,
                timeout_ms: None,
                restore_checkpoint: None,
                command: None,
                user_id: None,
                model: None,
                temperature: None,
            })
            .await
            .expect("invoke");
        assert_eq!(result.session_id, "named");
        assert_eq!(result.state, AgentRunState::Completed);
        let manager = ctx.service::<dyn SessionManager>(&SESSION_MANAGER).unwrap();
        let named = manager.open("named").expect("named session");
        assert!(
            named
                .events()
                .iter()
                .any(|e| e.kind == SessionEventKind::User)
        );
        assert!(
            named
                .events()
                .iter()
                .any(|e| e.kind == SessionEventKind::Assistant)
        );
        drop(effects);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn routes_workflow_and_persists_result_events() {
        let root = std::env::temp_dir().join(format!("ah-app-workflow-{}", std::process::id()));
        let session_dir = root.join("sessions");
        let ctx = Context::new();
        let effects = ctx
            .mount_all(vec![
                Arc::new(ah_plugins_mock::MockPlugin),
                Arc::new(ah_plugins_tools::ToolsPlugin),
                Arc::new(ah_plugins_sysop::SysopPlugin::new(&root)),
                Arc::new(ah_plugins_session_log::SessionLogPlugin::new(
                    session_dir.join("default.jsonl"),
                    &session_dir,
                )),
                Arc::new(ah_plugins_agent_control::AgentControlPlugin),
                Arc::new(ah_plugins_model_backup::ModelBackupPlugin::new(Vec::new())),
                Arc::new(ah_plugins_workflow::WorkflowPlugin),
                Arc::new(ah_plugins_agent_loop::AgentLoopPlugin::default()),
                Arc::new(ApplicationPlugin),
            ])
            .expect("mount");
        let app = ctx
            .service::<dyn ApplicationRuntime>(&ah_contracts::keys::APPLICATION)
            .unwrap();
        let workflow = serde_json::json!({
            "id": "wf", "nodes": [
                {"id": "start", "kind": "start", "config": {}},
                {"id": "end", "kind": "end", "config": {}}
            ], "edges": [{"from": "start", "to": "end"}]
        });
        let result = app
            .invoke(AgentRequest {
                session_id: "workflow-session".into(),
                input: "workflow input".into(),
                workflow: Some(workflow),
                timeout_ms: None,
                restore_checkpoint: None,
                command: None,
                user_id: None,
                model: None,
                temperature: None,
            })
            .await
            .expect("workflow invoke");
        assert_eq!(result.state, AgentRunState::Completed);
        let manager = ctx.service::<dyn SessionManager>(&SESSION_MANAGER).unwrap();
        let session = manager.open("workflow-session").unwrap();
        assert_eq!(session.events().len(), 2);
        assert_eq!(session.events()[0].kind, SessionEventKind::User);
        assert_eq!(session.events()[1].kind, SessionEventKind::Assistant);
        drop(effects);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn restores_checkpoint_before_workflow_execution() {
        let root = std::env::temp_dir().join(format!("ah-app-restore-{}", std::process::id()));
        let session_dir = root.join("sessions");
        let ctx = Context::new();
        let effects = ctx
            .mount_all(vec![
                Arc::new(ah_plugins_mock::MockPlugin),
                Arc::new(ah_plugins_tools::ToolsPlugin),
                Arc::new(ah_plugins_sysop::SysopPlugin::new(&root)),
                Arc::new(ah_plugins_session_log::SessionLogPlugin::new(
                    session_dir.join("default.jsonl"),
                    &session_dir,
                )),
                Arc::new(ah_plugins_agent_control::AgentControlPlugin),
                Arc::new(ah_plugins_model_backup::ModelBackupPlugin::new(Vec::new())),
                Arc::new(ah_plugins_workflow::WorkflowPlugin),
                Arc::new(ah_plugins_agent_loop::AgentLoopPlugin::default()),
                Arc::new(ApplicationPlugin),
            ])
            .expect("mount");
        let manager = ctx.service::<dyn SessionManager>(&SESSION_MANAGER).unwrap();
        let session = manager.create("restore-session").unwrap();
        session
            .append(
                SessionEventKind::User,
                serde_json::json!({"content": "before checkpoint"}),
            )
            .unwrap();
        manager.checkpoint("restore-session", "before-run").unwrap();
        let app = ctx
            .service::<dyn ApplicationRuntime>(&ah_contracts::keys::APPLICATION)
            .unwrap();
        let workflow = serde_json::json!({"id":"wf","nodes":[{"id":"start","kind":"start","config":{}},{"id":"end","kind":"end","config":{}}],"edges":[{"from":"start","to":"end"}]});
        let result = app
            .invoke(AgentRequest {
                session_id: "restore-session".into(),
                input: "after restore".into(),
                workflow: Some(workflow),
                timeout_ms: None,
                restore_checkpoint: Some("before-run".into()),
                command: None,
                user_id: None,
                model: None,
                temperature: None,
            })
            .await
            .unwrap();
        assert_eq!(result.state, AgentRunState::Completed);
        let restored = manager.open("restore-session").unwrap();
        assert!(
            restored
                .events()
                .iter()
                .any(|event| event.payload["content"] == "before checkpoint")
        );
        assert!(
            restored
                .events()
                .iter()
                .any(|event| event.payload["content"] == "after restore")
        );
        drop(effects);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn returns_cancelled_state_instead_of_losing_lifecycle_error() {
        let root = std::env::temp_dir().join(format!("ah-app-cancel-{}", std::process::id()));
        let session_dir = root.join("sessions");
        let ctx = Context::new();
        let effects = ctx
            .mount_all(vec![
                Arc::new(ah_plugins_mock::MockPlugin),
                Arc::new(ah_plugins_tools::ToolsPlugin),
                Arc::new(ah_plugins_sysop::SysopPlugin::new(&root)),
                Arc::new(ah_plugins_session_log::SessionLogPlugin::new(
                    session_dir.join("default.jsonl"),
                    &session_dir,
                )),
                Arc::new(ah_plugins_agent_control::AgentControlPlugin),
                Arc::new(ah_plugins_model_backup::ModelBackupPlugin::new(Vec::new())),
                Arc::new(ah_plugins_workflow::WorkflowPlugin),
                Arc::new(ah_plugins_agent_loop::AgentLoopPlugin::default()),
                Arc::new(ApplicationPlugin),
            ])
            .expect("mount");
        let interrupt = ctx
            .service::<dyn ah_contracts::agent::InterruptRuntime>(&ah_contracts::keys::INTERRUPT)
            .unwrap();
        interrupt
            .request("cancelled", ah_contracts::agent::AgentControl::Cancel)
            .await
            .unwrap();
        let app = ctx
            .service::<dyn ApplicationRuntime>(&ah_contracts::keys::APPLICATION)
            .unwrap();
        let result = app
            .invoke(AgentRequest {
                session_id: "cancelled".into(),
                input: "stop".into(),
                workflow: None,
                timeout_ms: None,
                restore_checkpoint: None,
                command: None,
                user_id: None,
                model: None,
                temperature: None,
            })
            .await
            .expect("structured cancellation");
        assert_eq!(result.state, AgentRunState::Cancelled);
        assert!(result.answer.is_none());
        drop(effects);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn persists_agent_failure_as_system_event() {
        let sessions = Arc::new(TestSessions::default());
        let runtime = LocalApplicationRuntime {
            agent: Arc::new(FailingAgentLoop),
            workflow: Arc::new(RejectWorkflow),
            sessions: sessions.clone(),
            controller: None,
            ctx: Context::new(),
        };
        let result = runtime
            .invoke(AgentRequest {
                session_id: "failed-agent".into(),
                input: "run failure".into(),
                workflow: None,
                timeout_ms: None,
                restore_checkpoint: None,
                command: None,
                user_id: None,
                model: None,
                temperature: None,
            })
            .await
            .expect("structured failure");
        assert_eq!(result.state, AgentRunState::Failed);
        let session = sessions.open("failed-agent").unwrap();
        let event = session
            .events()
            .into_iter()
            .find(|event| event.kind == SessionEventKind::System)
            .expect("failure event");
        assert_eq!(event.payload["state"], "failed");
        assert_eq!(event.payload["error"], "model unavailable");
    }

    #[tokio::test]
    async fn routes_retry_command_to_controller() {
        let controller = Arc::new(CommandController {
            fail: false,
            runs: std::sync::atomic::AtomicUsize::new(0),
        });
        let runtime = LocalApplicationRuntime {
            agent: Arc::new(AgentLoop::new(
                Arc::new(RejectModel),
                Arc::new(EmptyTools),
                Arc::new(EmptySession),
                Context::new(),
                1,
            )),
            workflow: Arc::new(RejectWorkflow),
            sessions: Arc::new(TestSessions::default()),
            controller: Some(controller),
            ctx: Context::new(),
        };
        let result = runtime
            .invoke(AgentRequest {
                session_id: "retry-command".into(),
                input: "retry:task-1".into(),
                workflow: None,
                timeout_ms: None,
                restore_checkpoint: None,
                command: None,
                user_id: None,
                model: None,
                temperature: None,
            })
            .await
            .unwrap();
        assert_eq!(result.state, AgentRunState::Completed);
        assert_eq!(result.answer.as_deref(), Some("task task-1 updated"));
    }

    #[tokio::test]
    async fn resumes_submitted_task_by_scheduling_it() {
        let controller = Arc::new(CommandController {
            fail: false,
            runs: std::sync::atomic::AtomicUsize::new(0),
        });
        let runtime = LocalApplicationRuntime {
            agent: Arc::new(AgentLoop::new(
                Arc::new(RejectModel),
                Arc::new(EmptyTools),
                Arc::new(EmptySession),
                Context::new(),
                1,
            )),
            workflow: Arc::new(RejectWorkflow),
            sessions: Arc::new(TestSessions::default()),
            controller: Some(controller.clone()),
            ctx: Context::new(),
        };
        let result = runtime
            .invoke(AgentRequest {
                session_id: "resume-command".into(),
                input: "resume:task-1".into(),
                workflow: None,
                timeout_ms: None,
                restore_checkpoint: None,
                command: None,
                user_id: None,
                model: None,
                temperature: None,
            })
            .await
            .unwrap();
        assert_eq!(result.state, AgentRunState::Completed);
        assert_eq!(controller.runs.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(result.answer.as_deref(), Some("task task-1 updated"));
    }

    #[tokio::test]
    async fn routes_natural_language_cancel_intent_to_controller() {
        let controller = Arc::new(CommandController {
            fail: false,
            runs: std::sync::atomic::AtomicUsize::new(0),
        });
        let runtime = LocalApplicationRuntime {
            agent: Arc::new(AgentLoop::new(
                Arc::new(RejectModel),
                Arc::new(EmptyTools),
                Arc::new(EmptySession),
                Context::new(),
                1,
            )),
            workflow: Arc::new(RejectWorkflow),
            sessions: Arc::new(TestSessions::default()),
            controller: Some(controller),
            ctx: Context::new(),
        };
        let result = runtime
            .invoke(AgentRequest {
                session_id: "natural-command".into(),
                input: "please cancel task-1".into(),
                workflow: None,
                timeout_ms: None,
                restore_checkpoint: None,
                command: None,
                user_id: None,
                model: None,
                temperature: None,
            })
            .await
            .unwrap();
        assert_eq!(result.state, AgentRunState::Completed);
        assert_eq!(result.answer.as_deref(), Some("task task-1 updated"));
        let session = runtime.sessions.open("natural-command").unwrap();
        assert!(session.events().iter().any(|event| {
            event.kind == SessionEventKind::System
                && event.payload["intent"] == "CancelTask"
                && event.payload["task_id"] == "task-1"
                && event.payload["state"] == "Completed"
        }));
    }

    #[tokio::test]
    async fn routes_cancel_command_to_controller() {
        let controller = Arc::new(CommandController {
            fail: false,
            runs: std::sync::atomic::AtomicUsize::new(0),
        });
        let runtime = LocalApplicationRuntime {
            agent: Arc::new(AgentLoop::new(
                Arc::new(RejectModel),
                Arc::new(EmptyTools),
                Arc::new(EmptySession),
                Context::new(),
                1,
            )),
            workflow: Arc::new(RejectWorkflow),
            sessions: Arc::new(TestSessions::default()),
            controller: Some(controller),
            ctx: Context::new(),
        };
        let result = runtime
            .invoke(AgentRequest {
                session_id: "command".into(),
                input: "cancel:task-1".into(),
                workflow: None,
                timeout_ms: None,
                restore_checkpoint: None,
                command: None,
                user_id: None,
                model: None,
                temperature: None,
            })
            .await
            .unwrap();
        assert_eq!(result.state, AgentRunState::Completed);
        assert_eq!(result.answer.as_deref(), Some("task task-1 updated"));
        let session = runtime.sessions.open("command").unwrap();
        assert!(session.events().iter().any(|event| {
            event.kind == SessionEventKind::System
                && event.payload["command"] == "cancel:task-1"
                && event.payload["task_id"] == "task-1"
                && event.payload["state"] == "Completed"
        }));
    }

    #[tokio::test]
    async fn persists_controller_command_failure() {
        let runtime = LocalApplicationRuntime {
            agent: Arc::new(AgentLoop::new(
                Arc::new(RejectModel),
                Arc::new(EmptyTools),
                Arc::new(EmptySession),
                Context::new(),
                1,
            )),
            workflow: Arc::new(RejectWorkflow),
            sessions: Arc::new(TestSessions::default()),
            controller: Some(Arc::new(CommandController {
                fail: true,
                runs: std::sync::atomic::AtomicUsize::new(0),
            })),
            ctx: Context::new(),
        };
        let result = runtime
            .invoke(AgentRequest {
                session_id: "command-failure".into(),
                input: "cancel:task-1".into(),
                workflow: None,
                timeout_ms: None,
                restore_checkpoint: None,
                command: None,
                user_id: None,
                model: None,
                temperature: None,
            })
            .await
            .unwrap();
        assert_eq!(result.state, AgentRunState::Failed);
        assert!(
            result
                .error
                .as_deref()
                .unwrap()
                .contains("controller unavailable")
        );
        let session = runtime.sessions.open("command-failure").unwrap();
        let event = session
            .events()
            .into_iter()
            .find(|event| event.kind == SessionEventKind::System)
            .expect("failure event");
        assert_eq!(event.payload["command"], "cancel:task-1");
        assert_eq!(event.payload["error"], "controller unavailable");
    }

    #[tokio::test]
    async fn rejects_control_command_without_task_id() {
        let runtime = LocalApplicationRuntime {
            agent: Arc::new(AgentLoop::new(
                Arc::new(RejectModel),
                Arc::new(EmptyTools),
                Arc::new(EmptySession),
                Context::new(),
                1,
            )),
            workflow: Arc::new(RejectWorkflow),
            sessions: Arc::new(TestSessions::default()),
            controller: Some(Arc::new(CommandController {
                fail: false,
                runs: std::sync::atomic::AtomicUsize::new(0),
            })),
            ctx: Context::new(),
        };
        let error = runtime
            .invoke(AgentRequest {
                session_id: "command".into(),
                input: "cancel".into(),
                workflow: None,
                timeout_ms: None,
                restore_checkpoint: None,
                command: None,
                user_id: None,
                model: None,
                temperature: None,
            })
            .await
            .unwrap_err();
        assert!(error.0.contains("requires task id"));
        let session = runtime.sessions.open("command").unwrap();
        let event = session
            .events()
            .into_iter()
            .find(|event| event.kind == SessionEventKind::System)
            .expect("invalid command event");
        assert_eq!(event.payload["command"], "cancel");
        assert_eq!(event.payload["state"], "Failed");
        assert_eq!(
            event.payload["error"],
            "controller command requires task id after ':'"
        );
    }

    #[tokio::test]
    async fn rejects_empty_restore_checkpoint_explicitly() {
        let runtime = LocalApplicationRuntime {
            agent: Arc::new(AgentLoop::new(
                Arc::new(RejectModel),
                Arc::new(EmptyTools),
                Arc::new(EmptySession),
                Context::new(),
                1,
            )),
            workflow: Arc::new(RejectWorkflow),
            sessions: Arc::new(RejectSessions),
            controller: None,
            ctx: Context::new(),
        };
        let error = runtime
            .invoke(AgentRequest {
                session_id: "restore".into(),
                input: "continue".into(),
                workflow: None,
                timeout_ms: None,
                restore_checkpoint: Some(" ".into()),
                command: None,
                user_id: None,
                model: None,
                temperature: None,
            })
            .await
            .unwrap_err();
        assert!(error.0.contains("restore_checkpoint must not be empty"));
    }

    #[tokio::test]
    async fn rejects_missing_restore_checkpoint_explicitly() {
        let runtime = LocalApplicationRuntime {
            agent: Arc::new(AgentLoop::new(
                Arc::new(RejectModel),
                Arc::new(EmptyTools),
                Arc::new(EmptySession),
                Context::new(),
                1,
            )),
            workflow: Arc::new(RejectWorkflow),
            sessions: Arc::new(RejectSessions),
            controller: None,
            ctx: Context::new(),
        };
        let error = runtime
            .invoke(AgentRequest {
                session_id: "restore".into(),
                input: "continue".into(),
                workflow: None,
                timeout_ms: None,
                restore_checkpoint: Some("missing".into()),
                command: None,
                user_id: None,
                model: None,
                temperature: None,
            })
            .await
            .unwrap_err();
        assert!(error.0.contains("session restore failed"));
    }

    #[tokio::test]
    async fn rejects_invalid_request_without_running() {
        let runtime = LocalApplicationRuntime {
            agent: Arc::new(AgentLoop::new(
                Arc::new(RejectModel),
                Arc::new(EmptyTools),
                Arc::new(EmptySession),
                Context::new(),
                1,
            )),
            workflow: Arc::new(RejectWorkflow),
            sessions: Arc::new(RejectSessions),
            controller: None,
            ctx: Context::new(),
        };
        let err = runtime
            .invoke(AgentRequest {
                session_id: String::new(),
                input: "x".into(),
                workflow: None,
                timeout_ms: None,
                restore_checkpoint: None,
                command: None,
                user_id: None,
                model: None,
                temperature: None,
            })
            .await
            .unwrap_err();
        assert!(err.0.contains("must not be empty"));
    }

    #[test]
    fn parses_structured_controller_command() {
        let intent = intent_from_command(&serde_json::json!({
            "type": "create_task",
            "task_id": "task-42",
            "description": "inspect workspace",
            "confidence": 0.99
        }))
        .expect("command should parse");
        assert_eq!(intent.intent_type, IntentType::CreateTask);
        assert_eq!(intent.task_id.as_deref(), Some("task-42"));
        assert_eq!(intent.task_text.as_deref(), Some("inspect workspace"));
        assert_eq!(intent.confidence, 0.99);
    }

    #[test]
    fn rejects_invalid_structured_command_confidence() {
        let error = intent_from_command(&serde_json::json!({
            "intent_type": "cancel_task",
            "confidence": 2.0
        }))
        .unwrap_err();
        assert!(error.0.contains("between 0 and 1"));
    }

    struct CommandController {
        fail: bool,
        runs: std::sync::atomic::AtomicUsize,
    }
    impl Seam for CommandController {}
    #[async_trait]
    impl Controller for CommandController {
        fn create_task(
            &self,
            task: ah_contracts::controller::Task,
        ) -> Result<ah_contracts::controller::Task, ah_contracts::controller::ControllerError>
        {
            Ok(task)
        }
        fn get_task(&self, _: &str) -> Option<ah_contracts::controller::Task> {
            None
        }
        fn filter_tasks(
            &self,
            _: &ah_contracts::controller::TaskFilter,
        ) -> Result<Vec<ah_contracts::controller::Task>, ah_contracts::controller::ControllerError>
        {
            Ok(vec![])
        }
        fn remove_task(&self, _: &str) -> Result<(), ah_contracts::controller::ControllerError> {
            Ok(())
        }
        fn update_status(
            &self,
            _: &str,
            _: ah_contracts::controller::TaskStatus,
        ) -> Result<(), ah_contracts::controller::ControllerError> {
            if self.fail {
                Err(ah_contracts::controller::ControllerError(
                    "controller unavailable".into(),
                ))
            } else {
                Ok(())
            }
        }
        fn link_parent(
            &self,
            _: &str,
            _: &str,
        ) -> Result<(), ah_contracts::controller::ControllerError> {
            Ok(())
        }
        fn pending_tasks(&self, _: &str) -> Vec<ah_contracts::controller::Task> {
            vec![]
        }
        fn register_executor(
            &self,
            _: Arc<dyn ah_contracts::controller::TaskExecutor>,
        ) -> ah_contracts::Effect {
            ah_contracts::Effect::noop()
        }
        async fn run_task(
            &self,
            _: &str,
        ) -> Result<String, ah_contracts::controller::ControllerError> {
            self.runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if self.fail {
                Err(ah_contracts::controller::ControllerError(
                    "controller unavailable".into(),
                ))
            } else {
                Ok("resumed".into())
            }
        }
        async fn run_pending(
            &self,
            _: Option<&str>,
        ) -> Vec<(
            String,
            Result<String, ah_contracts::controller::ControllerError>,
        )> {
            vec![]
        }
        fn retry_task(&self, _: &str) -> Result<(), ah_contracts::controller::ControllerError> {
            if self.fail {
                Err(ah_contracts::controller::ControllerError(
                    "controller unavailable".into(),
                ))
            } else {
                Ok(())
            }
        }
        async fn cancel_task(
            &self,
            _: &str,
        ) -> Result<(), ah_contracts::controller::ControllerError> {
            if self.fail {
                Err(ah_contracts::controller::ControllerError(
                    "controller unavailable".into(),
                ))
            } else {
                Ok(())
            }
        }
        fn recognize_intent(&self, query: &str) -> ah_contracts::controller::Intent {
            ah_contracts::controller::Intent {
                intent_type: if query.contains("cancel") {
                    IntentType::CancelTask
                } else if query.contains("pause") {
                    IntentType::PauseTask
                } else if query.contains("resume") {
                    IntentType::ResumeTask
                } else if query.contains("retry") {
                    IntentType::RetryTask
                } else {
                    IntentType::UnknownTask
                },
                task_text: None,
                task_id: None,
                confidence: 1.0,
            }
        }
    }

    struct FailingAgentLoop;
    impl Seam for FailingAgentLoop {}
    #[async_trait]
    impl ah_contracts::agent::AgentLoopRuntime for FailingAgentLoop {
        async fn run(&self, _: &str) -> ah_contracts::agent::AgentResult {
            ah_contracts::agent::AgentResult {
                session_id: String::new(),
                state: ah_contracts::agent::AgentRunState::Failed,
                answer: None,
                iterations: 0,
                tool_calls: 0,
                failure: Some(ah_contracts::agent::AgentFailure::Model),
                error: Some("model unavailable".into()),
            }
        }

        async fn run_in_session(
            &self,
            _: Arc<dyn ah_contracts::session::SessionLog>,
            _: &str,
        ) -> ah_contracts::agent::AgentResult {
            ah_contracts::agent::AgentResult {
                session_id: String::new(),
                state: ah_contracts::agent::AgentRunState::Failed,
                answer: None,
                iterations: 0,
                tool_calls: 0,
                failure: Some(ah_contracts::agent::AgentFailure::Model),
                error: Some("model unavailable".into()),
            }
        }
    }

    struct RejectModel;
    impl Seam for RejectModel {}
    #[async_trait]
    impl ah_contracts::llm::ModelProvider for RejectModel {
        fn name(&self) -> &'static str {
            "reject"
        }
        async fn chat(
            &self,
            _: ah_contracts::llm::ModelRequest,
        ) -> Result<ah_contracts::llm::ModelResponse, ah_contracts::llm::ModelError> {
            Err(ah_contracts::llm::ModelError("should not run".into()))
        }
    }
    struct EmptyTools;
    impl Seam for EmptyTools {}
    #[async_trait]
    impl ToolRegistry for EmptyTools {
        fn register(&self, _: Arc<dyn ah_contracts::tools::Tool>) -> ah_contracts::Effect {
            ah_contracts::Effect::noop()
        }
        fn get(&self, _: &str) -> Option<Arc<dyn ah_contracts::tools::Tool>> {
            None
        }
        fn names(&self) -> Vec<String> {
            vec![]
        }
        async fn invoke(
            &self,
            _: &str,
            _: serde_json::Value,
        ) -> Result<serde_json::Value, ah_contracts::tools::ToolError> {
            Err(ah_contracts::tools::ToolError("unused".into()))
        }
    }
    struct EmptySession;
    impl Seam for EmptySession {}
    impl ah_contracts::session::SessionLog for EmptySession {
        fn id(&self) -> &str {
            "s"
        }
        fn append(
            &self,
            _: ah_contracts::session::SessionEventKind,
            _: serde_json::Value,
        ) -> Result<ah_contracts::session::SessionEvent, ah_contracts::session::SessionError>
        {
            Err(ah_contracts::session::SessionError("unused".into()))
        }
        fn events(&self) -> Vec<ah_contracts::session::SessionEvent> {
            vec![]
        }
        fn claim_tool_call(
            &self,
            _: &str,
            _: &str,
            _: u64,
        ) -> Result<bool, ah_contracts::session::SessionError> {
            Ok(false)
        }
        fn since(&self, _: u64) -> Vec<ah_contracts::session::SessionEvent> {
            vec![]
        }
        fn derive_messages(&self) -> Vec<ah_contracts::llm::ChatMessage> {
            vec![]
        }
    }
    #[derive(Default)]
    struct TestSessions(
        std::sync::Mutex<
            std::collections::HashMap<String, Arc<dyn ah_contracts::session::SessionLog>>,
        >,
    );
    impl Seam for TestSessions {}
    impl SessionManager for TestSessions {
        fn create(
            &self,
            id: &str,
        ) -> Result<Arc<dyn ah_contracts::session::SessionLog>, ah_contracts::session::SessionError>
        {
            let log: Arc<dyn ah_contracts::session::SessionLog> = Arc::new(TestSession {
                id: id.to_string(),
                events: std::sync::Mutex::new(vec![]),
            });
            self.0.lock().unwrap().insert(id.to_string(), log.clone());
            Ok(log)
        }
        fn open(
            &self,
            id: &str,
        ) -> Result<Arc<dyn ah_contracts::session::SessionLog>, ah_contracts::session::SessionError>
        {
            self.0
                .lock()
                .unwrap()
                .get(id)
                .cloned()
                .ok_or_else(|| ah_contracts::session::SessionError("missing".into()))
        }
        fn fork(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Arc<dyn ah_contracts::session::SessionLog>, ah_contracts::session::SessionError>
        {
            Err(ah_contracts::session::SessionError("unused".into()))
        }
        fn checkpoint(&self, _: &str, _: &str) -> Result<(), ah_contracts::session::SessionError> {
            Ok(())
        }
        fn restore(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Arc<dyn ah_contracts::session::SessionLog>, ah_contracts::session::SessionError>
        {
            Err(ah_contracts::session::SessionError("unused".into()))
        }
        fn list(&self) -> Vec<String> {
            self.0.lock().unwrap().keys().cloned().collect()
        }
    }
    struct TestSession {
        id: String,
        events: std::sync::Mutex<Vec<ah_contracts::session::SessionEvent>>,
    }
    impl Seam for TestSession {}
    impl ah_contracts::session::SessionLog for TestSession {
        fn id(&self) -> &str {
            &self.id
        }
        fn append(
            &self,
            kind: SessionEventKind,
            payload: serde_json::Value,
        ) -> Result<ah_contracts::session::SessionEvent, ah_contracts::session::SessionError>
        {
            let mut events = self.events.lock().unwrap();
            let event = ah_contracts::session::SessionEvent {
                seq: events.len() as u64 + 1,
                timestamp_ms: 0,
                kind,
                payload,
            };
            events.push(event.clone());
            Ok(event)
        }
        fn events(&self) -> Vec<ah_contracts::session::SessionEvent> {
            self.events.lock().unwrap().clone()
        }
        fn claim_tool_call(
            &self,
            _: &str,
            _: &str,
            _: u64,
        ) -> Result<bool, ah_contracts::session::SessionError> {
            Ok(false)
        }
        fn since(&self, seq: u64) -> Vec<ah_contracts::session::SessionEvent> {
            self.events
                .lock()
                .unwrap()
                .iter()
                .filter(|e| e.seq > seq)
                .cloned()
                .collect()
        }
        fn derive_messages(&self) -> Vec<ah_contracts::llm::ChatMessage> {
            vec![]
        }
    }

    struct RejectSessions;
    impl Seam for RejectSessions {}
    impl SessionManager for RejectSessions {
        fn create(
            &self,
            _: &str,
        ) -> Result<Arc<dyn ah_contracts::session::SessionLog>, ah_contracts::session::SessionError>
        {
            Err(ah_contracts::session::SessionError("unused".into()))
        }
        fn open(
            &self,
            _: &str,
        ) -> Result<Arc<dyn ah_contracts::session::SessionLog>, ah_contracts::session::SessionError>
        {
            Err(ah_contracts::session::SessionError("unused".into()))
        }
        fn fork(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Arc<dyn ah_contracts::session::SessionLog>, ah_contracts::session::SessionError>
        {
            Err(ah_contracts::session::SessionError("unused".into()))
        }
        fn checkpoint(&self, _: &str, _: &str) -> Result<(), ah_contracts::session::SessionError> {
            Err(ah_contracts::session::SessionError("unused".into()))
        }
        fn restore(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Arc<dyn ah_contracts::session::SessionLog>, ah_contracts::session::SessionError>
        {
            Err(ah_contracts::session::SessionError("unused".into()))
        }
        fn list(&self) -> Vec<String> {
            vec![]
        }
    }
    struct RejectWorkflow;
    impl Seam for RejectWorkflow {}
    #[async_trait]
    impl WorkflowEngine for RejectWorkflow {
        async fn run(
            &self,
            _: &WorkflowSpec,
            _: serde_json::Value,
        ) -> Result<ah_contracts::workflow::WorkflowOutput, ah_contracts::workflow::WorkflowError>
        {
            Err(ah_contracts::workflow::WorkflowError("unused".into()))
        }
        async fn run_checkpointed(
            &self,
            _: &WorkflowSpec,
            _: serde_json::Value,
            _: &std::path::Path,
        ) -> Result<ah_contracts::workflow::CheckpointedOutput, ah_contracts::workflow::WorkflowError>
        {
            Err(ah_contracts::workflow::WorkflowError("unused".into()))
        }
    }
    struct TestMemory {
        records: Vec<ah_contracts::memory::MemoryRecord>,
    }

    impl Seam for TestMemory {}
    impl ah_contracts::memory::MemoryProvider for TestMemory {
        fn store(
            &self,
            _: &str,
            _: &str,
            _: Vec<String>,
        ) -> Result<ah_contracts::memory::MemoryRecord, ah_contracts::memory::MemoryError> {
            unreachable!("memory context test does not store")
        }
        fn retrieve(&self, key: &str) -> Option<ah_contracts::memory::MemoryRecord> {
            self.records
                .iter()
                .find(|record| record.key == key)
                .cloned()
        }
        fn search(&self, query: &str) -> Vec<ah_contracts::memory::MemoryRecord> {
            self.records
                .iter()
                .filter(|record| record.content.contains(query))
                .cloned()
                .collect()
        }
        fn list(&self) -> Vec<ah_contracts::memory::MemoryRecord> {
            self.records.clone()
        }
        fn remove(&self, _: &str) -> Result<(), ah_contracts::memory::MemoryError> {
            unreachable!("memory context test does not remove")
        }
    }

    #[test]
    fn memory_context_does_not_cross_user_boundaries() {
        let memory = TestMemory {
            records: vec![
                ah_contracts::memory::MemoryRecord {
                    key: "alice:session:a".into(),
                    content: "shared query from alice".into(),
                    tags: vec![],
                    created_ms: 1,
                },
                ah_contracts::memory::MemoryRecord {
                    key: "bob:session:b".into(),
                    content: "shared query from bob".into(),
                    tags: vec![],
                    created_ms: 2,
                },
            ],
        };
        let context = memory_context(&memory, "alice", "shared query").unwrap();
        assert!(context.contains("from alice"));
        assert!(!context.contains("from bob"));
    }
}
