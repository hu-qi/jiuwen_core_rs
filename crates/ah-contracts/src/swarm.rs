//! swarmflow seam:团队工作流编排(对应 agent_teams/workflow)。
//!
//! 4 层运行模型:SwarmFlowScript → 按 phase 顺序执行,phase 内 agent 并行或串行,
//! 每个 agent 活动产出 {prompt, activity, outcome};预算上限、进度事件流、
//! journal 续跑(同名已完成运行短路)。worker 后端为 SubagentRuntime(真实委派)。

use async_trait::async_trait;

use crate::seam::Seam;

/// 脚本中的一个 agent 活动。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SwarmAgentActivity {
    pub label: String,
    pub prompt: String,
    /// 模型提示(可选;None 用 worker 默认)。
    pub model: Option<String>,
}

/// 脚本中的一个 phase。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SwarmPhase {
    pub title: String,
    pub agents: Vec<SwarmAgentActivity>,
    /// true 时 phase 内 agent 并行执行(fork-join barrier)。
    pub parallel: bool,
}

/// 脚本:META name + 有序 phases。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SwarmFlowScript {
    pub name: String,
    pub phases: Vec<SwarmPhase>,
}

/// agent 活动状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwarmAgentStatus {
    Running,
    Completed,
    Failed,
}

/// 一次 agent 活动的记录(第 4 层)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SwarmAgentRecord {
    pub label: String,
    pub prompt: String,
    /// 过程叙述(如错误信息)。
    pub activity: Vec<String>,
    pub outcome: Option<String>,
    pub status: SwarmAgentStatus,
}

/// 一个 phase 的记录(第 2+3 层)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SwarmPhaseRecord {
    pub title: String,
    pub agents: Vec<SwarmAgentRecord>,
}

/// 运行状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwarmRunStatus {
    Running,
    Completed,
    BudgetExhausted,
}

/// 整个运行(第 1 层)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SwarmRun {
    pub name: String,
    pub run_id: String,
    pub status: SwarmRunStatus,
    pub phases: Vec<SwarmPhaseRecord>,
}

/// swarm 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwarmError(pub String);

impl core::fmt::Display for SwarmError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SwarmError {}

/// swarmflow 进度事件(引擎 emit,观察者折叠为 SwarmRun)。
#[derive(Clone, Debug, PartialEq)]
pub enum SwarmEventKind {
    WorkflowStarted,
    PhaseStarted(String),
    AgentStarted(String),
    AgentCompleted(String, bool),
    WorkflowCompleted(bool),
}

#[derive(Clone, Debug)]
pub struct SwarmEvent {
    pub run_id: String,
    pub kind: SwarmEventKind,
}

impl crate::event::Event for SwarmEvent {
    const ID: &'static str = "teams/swarm";
}

/// swarmflow Seam(Service Definition):执行与规划预览。
#[async_trait]
pub trait SwarmflowRunner: Seam {
    /// 执行脚本:phase 顺序、phase 内并行/串行、预算上限(总 agent 活动数)、
    /// 进度事件流、journal 续跑(同名已完成运行短路)。返回 4 层运行快照。
    async fn run(&self, script: SwarmFlowScript, budget: usize) -> Result<SwarmRun, SwarmError>;

    /// 规划预览:不执行,产出 running 态 4 层结构(供 UI/干跑展示)。
    fn preprocess(&self, script: &SwarmFlowScript) -> SwarmRun;
}
