//! evolving seam:轨迹抽取、评估(evaluator)、优化(optimizer)。
//!
//! 对应 openjiuwen/agent_evolving 的 trajectory / evaluator / optimizer:
//! - 轨迹:从真实会话日志(session/event)抽取;
//! - 评估:本地确定性判据必算(真实指标),LLM judge 可用时附加反馈;
//! - 优化:从评估问题推导可执行建议(真实规则),LLM 建议可用时附加。

use async_trait::async_trait;

use crate::seam::Seam;
use crate::session::SessionEvent;

/// 轨迹步骤结局。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepOutcome {
    Success,
    Error,
    Skipped,
}

/// 轨迹中的一个步骤(agent 的一轮行动)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TrajectoryStep {
    /// 会话事件序号。
    pub seq: u64,
    /// 行动摘要(助手消息内容或工具调用描述)。
    pub action: String,
    /// 调用的工具名(非工具步骤为 None)。
    pub tool: Option<String>,
    pub outcome: StepOutcome,
    /// 错误信息(Error 步骤)。
    pub error: Option<String>,
    /// 该步骤消耗的迭代预算。
    pub budget_used: u32,
}

/// 一次任务执行的完整轨迹(从会话日志抽取)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Trajectory {
    pub task: String,
    pub steps: Vec<TrajectoryStep>,
    /// 是否在预算内完成任务(取自 AgentStep.done)。
    pub finished: bool,
}

/// 评估结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Pass,
    NeedsWork,
    Fail,
}

/// 评估结果:本地判据 + 可选 LLM judge 反馈。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Evaluation {
    pub verdict: Verdict,
    /// 0..1 归一化得分。
    pub score: f64,
    pub strengths: Vec<String>,
    pub issues: Vec<String>,
    /// 人类可读反馈(含 LLM judge 原文或不可用原因)。
    pub feedback: String,
}

/// 优化建议的目标。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefinementTarget {
    Task,
    Prompt,
    Workflow,
    Tools,
}

/// 一条可执行的优化建议。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Refinement {
    pub target: RefinementTarget,
    pub suggestion: String,
    pub rationale: String,
    pub confidence: f64,
}

/// evolving 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvolvingError(pub String);

impl core::fmt::Display for EvolvingError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for EvolvingError {}

/// evolving Seam(Service Definition):真实轨迹抽取 + 本地判据评估 + 优化建议。
///
/// 消费方(如 rsi 编排)通过 `evolving` 服务键解析本 trait。
#[async_trait]
pub trait EvolvingRuntime: Seam {
    /// 从会话事件日志抽取轨迹(真实解析:工具调用配对结果、迭代预算、完成标志)。
    fn extract_trajectory(
        &self,
        task: &str,
        events: &[SessionEvent],
    ) -> Result<Trajectory, EvolvingError>;

    /// 打开会话管理器中的会话并抽取轨迹。
    fn extract_session(&self, task: &str, session_id: &str) -> Result<Trajectory, EvolvingError>;

    /// 评估轨迹:本地判据必算;LLM judge 可用时附加反馈(不可用原因显式记录)。
    async fn evaluate(&self, trajectory: &Trajectory) -> Result<Evaluation, EvolvingError>;

    /// 依据评估生成优化建议(真实规则推导;LLM 建议可用时附加)。
    async fn optimize(
        &self,
        trajectory: &Trajectory,
        evaluation: &Evaluation,
    ) -> Result<Vec<Refinement>, EvolvingError>;
}
