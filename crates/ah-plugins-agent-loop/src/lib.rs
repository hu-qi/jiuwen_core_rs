//! # ah-plugins-agent-loop
//!
//! 真实 ReAct agent 循环:注入 llm + tools + sessions 三个 seam。
//! 循环以会话日志为唯一事实来源(日志即真相):
//! 每次模型请求的消息序列由日志投影(derive_messages)重建,
//! 每轮的用户消息/助手消息/工具调用/工具结果都追加到日志。

use std::sync::Arc;

use ah_contracts::event::Event;
use ah_contracts::keys::{AGENT_LOOP, LLM, SESSIONS, TOOLS};
use ah_contracts::llm::{ModelProvider, ModelRequest, ToolSchema};
use ah_contracts::prelude::Effect;
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

/// agent 每轮(step)事件:emit 模式,供遥测/日志监听。
#[derive(Clone, Debug)]
pub struct AgentStep {
    /// 第几轮(0 起)。
    pub iteration: usize,
    /// 本轮模型请求的工具调用数。
    pub tool_calls: usize,
    /// 是否已得到最终回答(循环结束)。
    pub done: bool,
}

impl Event for AgentStep {
    const ID: &'static str = "agent/step";
}

/// 真实 ReAct 循环(日志驱动)。
pub struct AgentLoop {
    llm: Arc<dyn ModelProvider>,
    tools: Arc<dyn ToolRegistry>,
    sessions: Arc<dyn SessionLog>,
    ctx: Context,
    max_iterations: usize,
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
        }
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

    /// 运行一轮任务:日志驱动的 ReAct 循环。
    pub async fn run(&self, input: &str) -> Result<String, AgentLoopError> {
        // 1) 用户消息入日志。
        self.sessions
            .append(SessionEventKind::User, json!({ "content": input }))
            .map_err(|e| AgentLoopError(format!("session append failed: {e}")))?;

        for iteration in 0..self.max_iterations {
            // 2) 从日志投影模型可见消息(日志即真相)。
            let messages = self.sessions.derive_messages();
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
                self.sessions
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
            self.sessions
                .append(SessionEventKind::Assistant, json!({ "tool_calls": calls }))
                .map_err(|e| AgentLoopError(format!("session append failed: {e}")))?;

            // 5) 真实执行工具,结果入日志。
            for call in &response.tool_calls {
                let output = self
                    .tools
                    .invoke(&call.name, call.arguments.clone())
                    .await
                    .map_err(|e| AgentLoopError(format!("tool {} failed: {e}", call.name)))?;
                self.sessions
                    .append(
                        SessionEventKind::ToolResult,
                        json!({
                            "tool_call_id": call.id,
                            "output": output.to_string(),
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
        let agent = Arc::new(AgentLoop::new(
            llm,
            tools,
            sessions,
            ctx.clone(),
            self.max_iterations,
        ));
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
    use std::sync::Arc as StdArc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// 组装 dev 式组合:mock llm(桩)+ 真实工具 + 真实 sysop + 会话日志 + 循环。
    fn build_ctx(root: &std::path::Path, session_path: &std::path::Path) -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![
            StdArc::new(ah_plugins_mock::MockPlugin),
            StdArc::new(ah_plugins_tools::ToolsPlugin),
            StdArc::new(ah_plugins_sysop::SysopPlugin::new(root)),
            StdArc::new(ah_plugins_session_log::SessionLogPlugin::new(session_path)),
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
}
