//! trainer seam:自进化训练循环(对齐 Python agent_evolving/trainer)。
//!
//! 编排 "评估 → 优化更新 → 验证 → checkpoint" 循环:
//! - 先做验证基线评估;
//! - 每 epoch:train 前向评估 → Optimizer 应用文本梯度(经 OperatorRegistry)
//!   → 验证集评估 → 改进才 checkpoint;
//! - best_score 达到 early_stop_score 提前停止。

use async_trait::async_trait;

use crate::rsi::{RsiCase, RsiReport};
use crate::seam::Seam;

/// 训练请求。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TrainRequest {
    /// 训练用例(前向评估)。
    pub train_cases: Vec<RsiCase>,
    /// 验证用例(基线 + 每 epoch 门禁);缺省用 train_cases。
    pub val_cases: Option<Vec<RsiCase>>,
    /// 初始任务提示。
    pub task_prompt: String,
    /// 最大 epoch 数。
    pub max_epochs: u32,
    /// 达到即提前停止的验证得分(0..1)。
    pub early_stop_score: f64,
}

/// 单个 epoch 的训练结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TrainEpoch {
    pub epoch: u32,
    /// train 前向评估报告。
    pub train_report: RsiReport,
    /// 验证评估报告(门禁依据)。
    pub val_report: RsiReport,
    /// 是否改进(验证得分 > 历史 best)。
    pub improved: bool,
    /// 本 epoch 后 best 得分。
    pub best_score: f64,
}

/// 训练结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TrainResult {
    pub epochs: Vec<TrainEpoch>,
    /// 最终任务提示(改进后取 Optimizer 更新后的算子状态可导出;
    /// 简化:保留原始 prompt,epoch 详情含验证得分)。
    pub task_prompt: String,
    pub best_score: f64,
    /// 是否因达到 early_stop_score 提前停止。
    pub early_stopped: bool,
}

/// trainer 错误(复用 rsi 错误)。
pub type TrainerError = crate::rsi::RsiError;

/// trainer Seam(Service Definition):训练循环编排。
#[async_trait]
pub trait Trainer: Seam {
    /// 运行训练(基线评估 → 多轮 前向/更新/验证 → checkpoint 语义)。
    async fn train(&self, request: &TrainRequest) -> Result<TrainResult, TrainerError>;
}
