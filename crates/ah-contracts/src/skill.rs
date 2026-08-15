//! skill seam:技能注册与评估(skill_creator/evaluator)。
//!
//! 技能 = 命名能力描述 + 步骤清单(文件后端持久化);评估 = 以技能步骤为上下文
//! 真实委派 subagent 执行任务,再以 evolving 评估轨迹给出 verdict/score。

use async_trait::async_trait;

use crate::evolving::Verdict;
use crate::seam::Seam;

/// 一个技能。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub steps: Vec<String>,
    pub created_ms: u64,
}

/// 技能评估结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SkillEvaluation {
    pub skill_id: String,
    pub task: String,
    pub verdict: Verdict,
    pub score: f64,
    pub answer: String,
    pub iterations: usize,
}

/// skill 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillError(pub String);

impl core::fmt::Display for SkillError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SkillError {}

/// skill Seam(Service Definition):注册与评估。
#[async_trait]
pub trait SkillRegistry: Seam {
    /// 创建技能(文件落盘);id 冲突显式报错。
    fn create(
        &self,
        id: &str,
        name: &str,
        description: &str,
        steps: Vec<String>,
    ) -> Result<Skill, SkillError>;

    fn get(&self, id: &str) -> Option<Skill>;

    /// 全部技能(按 id 排序)。
    fn list(&self) -> Vec<Skill>;

    fn remove(&self, id: &str) -> Result<(), SkillError>;

    /// 评估技能:以步骤为上下文委派 subagent,再用 evolving 评估轨迹。
    async fn evaluate(&self, skill_id: &str, task: &str) -> Result<SkillEvaluation, SkillError>;
}
