//! # ah-plugins-agent-loop
//!
//! 真实 ReAct agent 循环:注入 llm + tools 两个 seam,
//! 循环执行"模型请求 → 工具调用 → 执行工具 → 结果回喂",直到模型给出最终回答。
//! 工具执行是真实的(经 tools seam);模型可以是真实 provider 或 boot 桩。

use std::sync::Arc;

use ah_contracts::event::Event;
use ah_contracts::keys::{AGENT_LOOP, LLM, TOOLS};
use ah_contracts::llm::{ChatMessage, ChatRole, ModelProvider, ModelRequest, ToolSchema};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::tools::ToolRegistry;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

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

/// 真实 ReAct 循环。
pub struct AgentLoop {
    llm: Arc<dyn ModelProvider>,
    tools: Arc<dyn ToolRegistry>,
    ctx: Context,
    max_iterations: usize,
}

impl AgentLoop {
    /// 构建循环。
    pub fn new(
        llm: Arc<dyn ModelProvider>,
        tools: Arc<dyn ToolRegistry>,
        ctx: Context,
        max_iterations: usize,
    ) -> Self {
        Self {
            llm,
            tools,
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

    /// 运行一轮任务:模型请求 → 工具调用 → 执行 → 回喂,直到最终回答。
    pub async fn run(&self, input: &str) -> Result<String, AgentLoopError> {
        let mut messages = vec![ChatMessage::new(ChatRole::User, input)];
        for iteration in 0..self.max_iterations {
            let response = self
                .llm
                .chat(ModelRequest {
                    messages: messages.clone(),
                    tools: self.tool_schemas(),
                    ..Default::default()
                })
                .await
                .map_err(|e| AgentLoopError(format!("model error: {e}")))?;

            if response.tool_calls.is_empty() {
                self.ctx.emit(AgentStep {
                    iteration,
                    tool_calls: 0,
                    done: true,
                });
                return Ok(response.content);
            }

            // 记录助手工具调用,逐条执行并回喂结果。
            messages.push(ChatMessage::assistant_with_tool_calls(
                response.tool_calls.clone(),
            ));
            for call in &response.tool_calls {
                let output = self
                    .tools
                    .invoke(&call.name, call.arguments.clone())
                    .await
                    .map_err(|e| AgentLoopError(format!("tool {} failed: {e}", call.name)))?;
                messages.push(ChatMessage::tool(call.id.clone(), output.to_string()));
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

/// agent 循环插件:注入 llm + tools,提供 agent-loop 服务。
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
        let agent = Arc::new(AgentLoop::new(llm, tools, ctx.clone(), self.max_iterations));
        Ok(vec![ctx.register(AGENT_LOOP, agent)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_hub::plugin::DynPlugin;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// 组装 dev 式组合:mock llm(桩)+ 真实工具 + 真实 sysop + 循环。
    fn build_ctx(root: &std::path::Path) -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![
            Arc::new(ah_plugins_mock::MockPlugin),
            Arc::new(ah_plugins_tools::ToolsPlugin),
            Arc::new(ah_plugins_sysop::SysopPlugin::new(root)),
            Arc::new(AgentLoopPlugin::default()),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[tokio::test]
    async fn react_loop_drives_real_tools_and_returns_final_answer() {
        use ah_contracts::fs::FsProvider;
        use ah_contracts::keys::FS;

        let root = std::env::temp_dir().join(format!("ah-loop-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);

        // 真实写入一个文件,让循环调用的 list_dir 能列出它。
        let fs = ctx.service::<dyn FsProvider>(&FS).expect("fs seam");
        fs.write("probe.txt", b"x").expect("write");

        let agent = ctx
            .service::<AgentLoop>(&AGENT_LOOP)
            .expect("agent-loop service");
        let answer = agent.run("explore the workspace").await.expect("run");

        // mock 模型:第一轮调 list_dir(真实工具),第二轮给最终回答;
        // 回答引用真实工具结果,包含真实文件名。
        assert!(answer.contains("mock final answer"));
        assert!(answer.contains("last tool result"));
        assert!(answer.contains("probe.txt"));

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn loop_emits_agent_step_events() {
        let root = std::env::temp_dir().join(format!("ah-loop-events-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);

        let steps = Arc::new(AtomicUsize::new(0));
        let counter = steps.clone();
        let _listener = ctx.on::<AgentStep>(move |_step| {
            counter.fetch_add(1, Ordering::SeqCst);
        });

        let agent = ctx
            .service::<AgentLoop>(&AGENT_LOOP)
            .expect("agent-loop service");
        let _ = agent.run("go").await.expect("run");

        // 至少一轮工具步 + 一轮结束步。
        assert!(steps.load(Ordering::SeqCst) >= 2);

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
}
