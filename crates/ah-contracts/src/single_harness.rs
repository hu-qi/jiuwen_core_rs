//! single_harness seam:单 harness 迭代编排 + 候选门禁(对齐 Python rsi/single_harness)。
//!
//! 与 orchestrator 不同,本 seam 面向单条 harness 的迭代优化:
//! - 数据集按比例拆 train/holdout;
//! - 每 epoch:评测当前提示 → 精化出候选提示 → 在 holdout 上评测候选;
//! - **候选门禁**:候选在 holdout 得分必须严格优于当前 best 才接受(防过拟合);
//! - best 与 checkpoint 真实落盘,支持中断续跑;结束时发布 best。

use async_trait::async_trait;

use crate::rsi::{RsiCase, RsiError, RsiReport};
use crate::seam::Seam;

/// 单 harness 优化请求。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SingleHarnessRequest {
    /// 评测数据集(真实用例)。
    pub cases: Vec<RsiCase>,
    /// 初始任务提示。
    pub task_prompt: String,
    /// 最大 epoch 数。
    pub max_epochs: u32,
    /// holdout 比例(0..1),默认 0.2。
    pub holdout_ratio: f64,
}

/// 单个 epoch 的结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EpochOutcome {
    pub epoch: u32,
    /// 当前提示在 train 上的报告。
    pub train_report: RsiReport,
    /// 候选提示在 holdout 上的报告(门禁依据)。
    pub holdout_report: RsiReport,
    /// 候选是否通过门禁并成为新 best。
    pub accepted: bool,
    /// 门禁说明(如 "candidate 0.85 > best 0.70, accepted")。
    pub gate_reason: String,
    /// 接受后更新的 best 得分。
    pub best_score: f64,
}

/// 优化结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SingleHarnessResult {
    /// 各 epoch 结果(按序)。
    pub epochs: Vec<EpochOutcome>,
    /// 最终发布的任务提示(best)。
    pub published_prompt: String,
    /// best 在 holdout 上的得分(无则 None)。
    pub best_score: Option<f64>,
}

/// single_harness 错误(复用 rsi 错误)。
pub type SingleHarnessError = RsiError;

/// single_harness checkpoint(续跑)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SingleHarnessCheckpoint {
    pub epoch: u32,
    /// 当前 best 提示。
    pub best_prompt: String,
    pub best_score: Option<f64>,
    /// 已完成的 epoch(按序,避免重复)。
    pub completed_epochs: Vec<u32>,
    pub updated_at_ms: u64,
}

/// single_harness Seam(Service Definition):迭代编排 + 候选门禁。
#[async_trait]
pub trait SingleHarnessRuntime: Seam {
    /// 运行优化(支持从 checkpoint 续跑:已完成的 epoch 跳过)。
    async fn optimize(
        &self,
        request: &SingleHarnessRequest,
    ) -> Result<SingleHarnessResult, SingleHarnessError>;

    /// 保存 checkpoint(真实 JSONL 落盘)。
    fn save_checkpoint(
        &self,
        checkpoint: &SingleHarnessCheckpoint,
    ) -> Result<(), SingleHarnessError>;

    /// 加载 checkpoint(无则 None)。
    fn load_checkpoint(&self) -> Result<Option<SingleHarnessCheckpoint>, SingleHarnessError>;
}
