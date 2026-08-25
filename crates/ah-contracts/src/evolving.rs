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
/// 一条经验(评估结果持久化,可检索复用)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Experience {
    pub id: String,
    pub task: String,
    pub verdict: Verdict,
    pub score: f64,
    pub issues: Vec<String>,
    pub saved_ms: u64,
}

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

    /// 保存一次评估为经验(真实 JSONL 落盘)。
    fn save_experience(&self, experience: &Experience) -> Result<(), EvolvingError>;

    /// 加载全部经验(按保存顺序)。
    fn load_experiences(&self) -> Result<Vec<Experience>, EvolvingError>;

    /// 检索与任务相关的经验(任务标题包含查询词)。
    fn search_experiences(&self, query: &str) -> Result<Vec<Experience>, EvolvingError>;
}
// ---------------------------------------------------------------------------
// 进化更新契约(对齐 agent_evolving/types.py + protocols.py)
// ---------------------------------------------------------------------------

/// 更新模式(对齐 UpdateMode)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateMode {
    Replace,
    Append,
    Merge,
}

impl UpdateMode {
    pub fn as_str(self) -> &'static str {
        match self {
            UpdateMode::Replace => "replace",
            UpdateMode::Append => "append",
            UpdateMode::Merge => "merge",
        }
    }
}

/// 更新效果(对齐 UpdateEffect)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateEffect {
    State,
    PendingChange,
}

impl UpdateEffect {
    pub fn as_str(self) -> &'static str {
        match self {
            UpdateEffect::State => "state",
            UpdateEffect::PendingChange => "pending_change",
        }
    }
}

/// 结构化更新契约(对齐 UpdateValue)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UpdateValue {
    pub payload: serde_json::Value,
    pub mode: UpdateMode,
    pub effect: UpdateEffect,
    pub change_type: Option<String>,
    pub metadata: serde_json::Map<String, serde_json::Value>,
}

impl UpdateValue {
    pub fn new(payload: serde_json::Value) -> Self {
        Self {
            payload,
            mode: UpdateMode::Replace,
            effect: UpdateEffect::State,
            change_type: None,
            metadata: serde_json::Map::new(),
        }
    }

    /// 旧值归一化(对齐 normalize_update_value):
    /// - experiences 目标 → append + pending_change + skill_experience_entry;
    /// - 其他 → replace + state。
    pub fn normalize(value: serde_json::Value, target: Option<&str>) -> Self {
        if target == Some("experiences") {
            let mut metadata = serde_json::Map::new();
            metadata.insert(
                "change_type".to_string(),
                serde_json::Value::String("skill_experience_entry".to_string()),
            );
            Self {
                payload: value,
                mode: UpdateMode::Append,
                effect: UpdateEffect::PendingChange,
                change_type: Some("skill_experience_entry".to_string()),
                metadata,
            }
        } else {
            Self::new(value)
        }
    }
}

/// 应用结果(对齐 ApplyResult)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ApplyResult {
    pub operator_id: String,
    pub target: String,
    pub applied: bool,
    pub mode: UpdateMode,
    pub effect: UpdateEffect,
    pub value: Option<serde_json::Value>,
    /// 应用的记录列表(对齐 ApplyResult.records)。
    #[serde(default)]
    pub records: Vec<serde_json::Value>,
    pub change_type: Option<String>,
    /// 生命周期阶段(对齐 ApplyResult.lifecycle_stage,如 local_apply_completed)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle_stage: Option<String>,
    /// 关联 pending change id(对齐 ApplyResult.pending_change_id)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_change_id: Option<String>,
    pub errors: Vec<String>,
    pub metadata: serde_json::Map<String, serde_json::Value>,
}

impl ApplyResult {
    pub fn ok(&self) -> bool {
        self.applied && self.errors.is_empty()
    }
}

/// 更新键(operator_id, target)。
pub type UpdateKey = (String, String);
