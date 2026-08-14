//! # ah-plugins-subagent
//!
//! 真实子代理:在隔离会话中,用预算受限的 agent 循环执行子任务。
//! 上下文经 system 消息注入(日志投影支持);提供 delegate_task 工具,
//! 让模型能把子任务委派给新会话。

use std::sync::Arc;

use ah_contracts::context::ContextEngine;
use ah_contracts::keys::{CONTEXT, LLM, SESSION_MANAGER, SESSIONS, SUBAGENT, TOOLS};
use ah_contracts::llm::{ModelProvider, ModelRequest, ToolSchema};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::session::{SessionEventKind, SessionManager};
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
}

impl LocalSubagentRuntime {
    /// 挂载 context seam 消费(可选)。
    pub fn with_context(mut self, context: Arc<dyn ContextEngine>, token_budget: usize) -> Self {
        self.context = Some(context);
        self.token_budget = token_budget;
        self
    }

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
            let response = self
                .llm
                .chat(ModelRequest {
                    messages,
                    tools: self.tool_schemas(),
                    ..Default::default()
                })
                .await
                .map_err(|e| SubagentError(format!("model error: {e}")))?;

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
                    .map_err(|e| SubagentError(format!("append failed: {e}")))?;
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

        let context = ctx.service::<dyn ContextEngine>(&CONTEXT);
        let mut runtime = LocalSubagentRuntime {
            llm,
            tools,
            manager,
            context: None,
            token_budget: 8192,
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
            StdArc::new(ah_plugins_session_log::SessionLogPlugin::new(
                &default_path,
                &session_dir,
            )),
            StdArc::new(SubagentPlugin),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
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
}
