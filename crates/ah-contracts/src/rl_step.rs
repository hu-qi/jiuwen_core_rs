//! rl_step seam:RL 训练步数学(agent_rl PPO/GRPO step 的纯计算部分)。
//!
//! 对齐 ppo_step 管线的确定性核心:reward → baseline(value)→ advantage →
//! clipped policy objective(ratio × clip)→ value loss → metrics。
//! 无 torch/GPU 依赖,纯数值计算,可测试。

use crate::seam::Seam;

/// 一条训练样本(一个生成轮次的观测)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RlSample {
    /// 该样本的 reward。
    pub reward: f64,
    /// value baseline 预测(0..1,训练前模型估计)。
    pub value: f64,
    /// 旧策略对数概率(训练前)。
    pub old_log_prob: f64,
    /// 当前策略对数概率(更新后)。
    pub log_prob: f64,
    /// 是否被判定有效(如长度/格式过滤)。
    pub valid: bool,
}

/// 一个训练步的结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RlStepResult {
    /// 逐样本 advantage(GAE-lite:reward - value)。
    pub advantages: Vec<f64>,
    /// 逐样本 policy ratio(exp(log_prob - old_log_prob))。
    pub ratios: Vec<f64>,
    /// 逐样本 clipped surrogate objective。
    pub clipped_objectives: Vec<f64>,
    /// 逐样本 value loss。
    pub value_losses: Vec<f64>,
    /// 聚合:policy loss 均值(负 surrogate)、value loss 均值、advantage 均值。
    pub policy_loss: f64,
    pub value_loss: f64,
    pub mean_advantage: f64,
}

/// rl_step 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RlStepError(pub String);

impl core::fmt::Display for RlStepError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RlStepError {}

/// PPO clip 参数。
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PpoParams {
    /// 裁剪系数(默认 0.2)。
    pub clip_epsilon: f64,
    /// value loss 系数(默认 0.5)。
    pub value_coef: f64,
}

impl Default for PpoParams {
    fn default() -> Self {
        Self {
            clip_epsilon: 0.2,
            value_coef: 0.5,
        }
    }
}

/// rl_step Seam(Service Definition):确定性训练步计算。
pub trait RlStep: Seam {
    /// 计算一个训练步:advantage/ratio/clipped objective/value loss + 聚合。
    fn step(&self, samples: &[RlSample], params: &PpoParams) -> Result<RlStepResult, RlStepError>;
}
