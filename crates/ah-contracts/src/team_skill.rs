//! team_skill seam:团队技能生成与演化(对齐 Python rsi/team_skill_generator)。
//!
//! 从任务生成 Team Skill(命名能力 + 步骤清单):确定性计划(从任务关键词提取
//! 技能名/描述/步骤)→ 可选 LLM 精化 → 在源任务上评估验证 → 注册到 skill seam。
//! 生成失败显式报错;评估不合格可重试生成(受 max_repair_attempts 限制)。

use async_trait::async_trait;

use crate::seam::Seam;
use crate::skill::{Skill, SkillError};

/// 技能生成请求。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GenerateTeamSkillRequest {
    /// 源任务(生成依据 + 验证用)。
    pub task: String,
    /// 期望技能 id(缺省由任务名生成)。
    pub skill_id: Option<String>,
    /// 最大修复重试次数(生成后验证不合格时重试)。
    pub max_repair_attempts: u32,
}

/// 生成结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GenerateTeamSkillResult {
    pub skill: Skill,
    /// 验证评估得分(0..1)。
    pub validation_score: f64,
    /// 是否一次通过(无重试)。
    pub passed_first_try: bool,
}

/// team_skill 错误(复用 skill 错误)。
pub type TeamSkillError = SkillError;

/// team_skill Seam(Service Definition):技能生成与验证。
#[async_trait]
pub trait TeamSkillGenerator: Seam {
    /// 生成技能:确定性计划(可选 LLM 精化)→ 注册 → 在源任务上验证;
    /// 验证不合格按 max_repair_attempts 重试,仍失败显式报错。
    async fn generate(
        &self,
        request: &GenerateTeamSkillRequest,
    ) -> Result<GenerateTeamSkillResult, TeamSkillError>;
}
