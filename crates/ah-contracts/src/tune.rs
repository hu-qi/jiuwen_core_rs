//! tune seam:训练流水线(optimizer/evaluator/trainer)。
//!
//! 真实循环:用当前 prompt 委派 subagent 执行任务 → evolving 评估轨迹得分 →
//! evolving 优化建议精化 prompt → 记录最优 prompt。全部真实 seam,无 mock。

use async_trait::async_trait;

use crate::seam::Seam;

/// 训练请求。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TuneRequest {
    pub task: String,
    /// 初始 prompt 池(每轮取一个作起点,后续轮用精化结果)。
    pub seed_prompts: Vec<String>,
    pub rounds: u32,
}

/// 单轮结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TuneRoundResult {
    pub round: u32,
    pub prompt: String,
    pub score: f64,
    pub issues: Vec<String>,
}

/// 训练结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TuneResult {
    pub rounds: Vec<TuneRoundResult>,
    pub best_prompt: String,
    pub best_score: f64,
}

/// tune 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuneError(pub String);

impl core::fmt::Display for TuneError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TuneError {}

/// tune Seam(Service Definition):prompt 训练流水线。
#[async_trait]
pub trait TunePipeline: Seam {
    /// 运行训练:每轮 委派执行 → 评估 → 优化精化,记录最优。
    async fn tune(&self, request: TuneRequest) -> Result<TuneResult, TuneError>;
}
