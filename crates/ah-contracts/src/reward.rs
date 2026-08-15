//! reward seam:RL 奖励计算(agent_rl 的 reward 部分)。
//!
//! 真实确定性奖励函数:通过 + 奖励,失败 + 惩罚,超时/工具错误/迭代数逐项扣罚。
//! VERL/PPO 训练、LoRA、gateway 留待后续,文档注明。

use crate::seam::Seam;

/// 一条奖励样本(来自评测结果)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RewardCase {
    pub case_id: String,
    pub passed: bool,
    /// 归一化得分(0..1)。
    pub score: f64,
    pub timed_out: bool,
    pub tool_errors: usize,
    pub iterations: u32,
}

/// 奖励配置。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RewardConfig {
    /// 通过样本的基础奖励。
    pub pass_reward: f64,
    /// 失败样本的基础惩罚(负数)。
    pub fail_penalty: f64,
    pub timeout_penalty: f64,
    pub tool_error_penalty: f64,
    pub iteration_penalty: f64,
}

impl Default for RewardConfig {
    fn default() -> Self {
        Self {
            pass_reward: 1.0,
            fail_penalty: -1.0,
            timeout_penalty: -0.5,
            tool_error_penalty: -0.2,
            iteration_penalty: -0.01,
        }
    }
}

/// 奖励输出。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RewardOutput {
    /// 逐样本奖励。
    pub per_case: Vec<f64>,
    pub mean: f64,
    pub total: f64,
}

/// reward 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewardError(pub String);

impl core::fmt::Display for RewardError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RewardError {}

/// reward Seam(Service Definition):确定性奖励函数。
pub trait RewardFunction: Seam {
    /// 默认配置(文档值)。
    fn default_config(&self) -> RewardConfig;

    /// 计算逐样本/均值/总奖励(真实数学,确定性)。
    fn compute(
        &self,
        config: &RewardConfig,
        cases: &[RewardCase],
    ) -> Result<RewardOutput, RewardError>;
}
