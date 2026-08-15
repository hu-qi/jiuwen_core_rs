//! member_optimizer seam:成员优化管线(对齐 Python rsi/member_optimizer)。
//!
//! attribution → plan → execute → verify → publish:
//! - attribution:从分析问题(issues)确定性归因机制类型(MechanismType:prompt/tool/
//!   skill/memory/workflow 等)与 lever(instruction/action/control/configuration);
//! - plan:机制 → 目标面(surface)与参数路由(llm_call/system_prompt 等);
//! - execute:经 OperatorRegistry + Optimizer 应用文本梯度;
//! - verify:在验证集重新评估,得分改进才算成功;
//! - publish:把 best 提示与结果写入发布目录(真实 JSON 落盘)。

use async_trait::async_trait;

use crate::seam::Seam;

/// 失败机制类型(对齐 MechanismType 子集,确定性可判)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MechanismType {
    Prompt,
    Tool,
    Skill,
    Memory,
    Workflow,
    Context,
    Unknown,
}

/// 优化 lever(对齐 LEVER_* 子集)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lever {
    Instruction,
    Action,
    Control,
    Configuration,
}

/// 归因结果:机制 + lever + 目标面。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Attribution {
    pub mechanism: MechanismType,
    pub lever: Lever,
    /// 目标面(如 llm_call/system_prompt、tool_call/tool_description)。
    pub surface: String,
    pub issue: String,
}

/// 优化计划(单一 action,对齐 max_actions_per_plan=1)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OptimizationPlan {
    pub attribution: Attribution,
    /// 文本梯度(交给 Optimizer 应用)。
    pub gradient: String,
}

/// 验证结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Verification {
    pub passed: bool,
    /// 优化后验证得分。
    pub score: f64,
    /// 优化前基线得分。
    pub baseline_score: f64,
    pub reason: String,
}

/// 发布结果(best 提示 + 引用)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PublishResult {
    /// 发布的提示文本。
    pub published_prompt: String,
    /// 引用文件路径(当前_harness_refs.json)。
    pub refs_path: String,
    pub score: f64,
}

/// 单次成员优化结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MemberOptimizationResult {
    pub attribution: Attribution,
    pub plan: OptimizationPlan,
    pub verification: Verification,
    pub publish: PublishResult,
}

/// member_optimizer 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberOptimizerError(pub String);

impl core::fmt::Display for MemberOptimizerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MemberOptimizerError {}

/// member_optimizer Seam(Service Definition):归因→计划→执行→验证→发布。
#[async_trait]
pub trait MemberOptimizer: Seam {
    /// 归因:从分析问题列表推导机制/lever/目标面(真实规则)。
    fn attribute(&self, issues: &[String]) -> Vec<Attribution>;

    /// 计划:归因 → 单一优化动作(含文本梯度)。
    fn plan(&self, attribution: &Attribution) -> OptimizationPlan;

    /// 执行并验证:应用梯度(经 OperatorRegistry + Optimizer)→ 在 val 集评估;
    /// 得分严格优于基线才通过。返回验证结果。
    async fn execute_and_verify(
        &self,
        plan: &OptimizationPlan,
        baseline_score: f64,
    ) -> Result<Verification, MemberOptimizerError>;

    /// 发布:把当前 best 提示与引用写入发布目录(真实 JSON 落盘),返回结果。
    fn publish(&self, prompt: &str, score: f64) -> Result<PublishResult, MemberOptimizerError>;
}
