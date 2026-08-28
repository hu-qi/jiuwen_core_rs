//! agent 域共享类型与事件。

use crate::event::Event;
use crate::seam::Seam;
use async_trait::async_trait;
use serde_json::Value;

/// agent 每轮(step)事件:emit 模式,供遥测/日志监听。
///
/// 定义在契约层(跨插件共享):生产者是 agent 循环插件,
/// 消费者是遥测/日志/UI 等任意插件。
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

/// Stable lifecycle state for a single agent execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRunState {
    Running,
    Interrupted,
    Cancelled,
    TimedOut,
    Completed,
    Failed,
}

/// 结构化失败种类(P1-02):消费方直接读字段判定,禁止解析错误字符串。
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentFailure {
    InvalidInput,
    Interrupted,
    Cancelled,
    TimedOut,
    Session,
    Model,
    Context,
    IterationLimit,
}

/// Public result returned by an agent execution.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AgentResult {
    pub session_id: String,
    pub state: AgentRunState,
    pub answer: Option<String>,
    pub iterations: usize,
    /// 实际执行的工具调用总数(含失败/被拒,均已入日志)。
    #[serde(default)]
    pub tool_calls: usize,
    /// 结构化失败种类:Failed 状态下的精确原因;正常完成/中断/取消/超时为对应值。
    #[serde(default)]
    pub failure: Option<AgentFailure>,
    /// 人类可读错误消息(非正常完成时)。
    pub error: Option<String>,
}

/// Agent card describing a callable agent capability.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AgentCard {
    pub id: String,
    pub name: String,
    pub description: String,
    pub capabilities: Vec<String>,
}

/// Cooperative control signal for an agent run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentControl {
    Continue,
    Interrupt,
    Cancel,
}

/// Interrupt/control seam. Implementations may be backed by a UI, API, or host.
#[async_trait]
pub trait InterruptRuntime: Seam {
    async fn request(
        &self,
        session_id: &str,
        control: AgentControl,
    ) -> Result<(), AgentControlError>;
    fn state(&self, session_id: &str) -> AgentControl;
    fn clear(&self, session_id: &str);
}

/// Control seam error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentControlError(pub String);

impl core::fmt::Display for AgentControlError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for AgentControlError {}

/// Callback invocation context for lifecycle hooks.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AgentCallbackContext {
    pub session_id: String,
    pub state: AgentRunState,
    pub iteration: usize,
    pub payload: Value,
}

/// Callback manager seam for lifecycle observers and recovery hooks.
#[async_trait]
pub trait AgentCallbackManager: Seam {
    async fn notify(&self, callback: AgentCallbackContext) -> Result<(), AgentControlError>;
}

/// Agent loop execution seam consumed by application routing.
#[async_trait]
pub trait AgentLoopRuntime: Seam {
    /// Describes the callable runtime without exposing its concrete plugin type.
    fn card(&self) -> AgentCard {
        AgentCard {
            id: "agent-runtime".into(),
            name: "Agent Runtime".into(),
            description: "A callable agent runtime.".into(),
            capabilities: vec!["chat".into()],
        }
    }

    /// 运行一轮任务并返回结构化结果(状态/终止原因/统计,不再依赖错误字符串)。
    async fn run(&self, input: &str) -> AgentResult;

    async fn run_in_session(
        &self,
        session: std::sync::Arc<dyn crate::session::SessionLog>,
        input: &str,
    ) -> AgentResult;

    async fn run_in_session_with_timeout(
        &self,
        session: std::sync::Arc<dyn crate::session::SessionLog>,
        input: &str,
        _timeout_ms: Option<u64>,
    ) -> AgentResult {
        self.run_in_session(session, input).await
    }
}

/// Application-level request selecting an LLM agent or workflow agent.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AgentRequest {
    pub session_id: String,
    pub input: String,
    pub workflow: Option<serde_json::Value>,
    pub timeout_ms: Option<u64>,
    /// Optional named session checkpoint to restore before execution.
    #[serde(default)]
    pub restore_checkpoint: Option<String>,
}

/// Application runtime seam for routing agent requests.
#[async_trait]
pub trait ApplicationRuntime: Seam {
    async fn invoke(&self, request: AgentRequest) -> Result<AgentResult, AgentControlError>;
}
