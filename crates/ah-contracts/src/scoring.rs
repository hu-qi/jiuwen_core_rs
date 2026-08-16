//! experience-scorer seam:经验评分与维护(对齐 openjiuwen/agent_evolving/experience/scorer.py)。
//!
//! 确定性核心(无 LLM 也可算):
//! - E(有效性)= 贝叶斯平滑 (positive+1)/(total+2),无数据时中性 0.5;
//! - U(利用率)= used/presented,无数据时中性 0.5;
//! - F(新鲜度)= 0.5 + 0.5·2^(-days/半衰期90),版本过期 ×0.7;
//! - 总分 = 0.5·E + 0.3·U + 0.2·F;
//! - update:按评估结果(used/positive/negative)累加统计并重算分数。
//!
//! LLM 评估(prompt 评估对话片段)与库整理(simplify)留待后续,由插件侧扩展。

use crate::seam::Seam;

/// 使用统计(对齐 UsageStats)。
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct UsageStats {
    pub times_used: u64,
    pub times_positive: u64,
    pub times_negative: u64,
    pub times_presented: u64,
    pub last_evaluated_at: Option<String>,
}

/// 带评分状态的经验记录(评分所需字段;timestamp 为 ISO-8601 字符串)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScoredExperience {
    pub id: String,
    pub timestamp: Option<String>,
    pub skill_version: Option<String>,
    pub usage_stats: UsageStats,
    pub score: f64,
}

impl ScoredExperience {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            timestamp: None,
            skill_version: None,
            usage_stats: UsageStats::default(),
            score: 0.0,
        }
    }
}

/// 评分错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScorerError(pub String);

impl core::fmt::Display for ScorerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ScorerError {}

/// 经验评分 Seam(Service Definition):E/U/F 三要素与加权总分。
pub trait ExperienceScorer: Seam {
    /// E:贝叶斯平滑有效性。
    fn effectiveness(&self, stats: &UsageStats) -> f64;

    /// U:利用率(used/presented)。
    fn utilization(&self, stats: &UsageStats) -> f64;

    /// F:新鲜度(时间衰减 + 版本过期惩罚)。
    fn freshness(
        &self,
        timestamp: Option<&str>,
        record_skill_version: Option<&str>,
        current_skill_version: Option<&str>,
    ) -> f64;

    /// 总分 = 0.5·E + 0.3·U + 0.2·F。
    fn score(&self, record: &ScoredExperience, current_skill_version: Option<&str>) -> f64;

    /// 按评估结果更新统计并重算分数,返回新分数。
    fn update(
        &self,
        record: &mut ScoredExperience,
        used: bool,
        positive: bool,
        negative: bool,
        current_skill_version: Option<&str>,
    ) -> Result<f64, ScorerError>;
}
