//! # ah-plugins-rl-step
//!
//! 真实 RL 训练步数学(对齐 ppo_step 管线的确定性核心):
//! - advantage = reward - value(GAE-lite,单步);
//! - policy ratio = exp(log_prob - old_log_prob);
//! - clipped objective = min(ratio × advantage, clip(ratio, 1±ε) × advantage);
//! - value loss = (value - reward)²;
//! - 聚合 policy_loss(负 surrogate 均值)/ value_loss / mean_advantage。
//!
//! 无效样本(reward/数值非有限)显式剔除并记录。

use std::sync::Arc;

use ah_contracts::keys::RL_STEP;
use ah_contracts::prelude::Effect;
use ah_contracts::rl_step::{PpoParams, RlSample, RlStep, RlStepError, RlStepResult};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 真实 PPO 训练步。
pub struct PpoRlStep;

impl Seam for PpoRlStep {}

impl RlStep for PpoRlStep {
    fn step(&self, samples: &[RlSample], params: &PpoParams) -> Result<RlStepResult, RlStepError> {
        if samples.is_empty() {
            return Err(RlStepError("no samples".to_string()));
        }
        let mut advantages = Vec::new();
        let mut ratios = Vec::new();
        let mut clipped = Vec::new();
        let mut value_losses = Vec::new();
        let mut surrogate_sum = 0.0;
        let mut value_sum = 0.0;
        let mut adv_sum = 0.0;
        let mut n = 0usize;

        for sample in samples {
            if !sample.valid {
                continue; // 无效样本(长度/格式过滤)剔除。
            }
            let reward = sample.reward;
            let value = sample.value;
            if !reward.is_finite() || !value.is_finite() {
                continue; // 非有限显式剔除(对齐 clamp_nonfinite 的保护语义)。
            }
            let advantage = reward - value;
            let ratio = (sample.log_prob - sample.old_log_prob).exp();
            let low = 1.0 - params.clip_epsilon;
            let high = 1.0 + params.clip_epsilon;
            let clipped_ratio = ratio.clamp(low, high);
            let objective = if advantage >= 0.0 {
                ratio.min(clipped_ratio) * advantage
            } else {
                ratio.max(clipped_ratio) * advantage
            };
            let value_loss = (value - reward).powi(2);

            advantages.push(advantage);
            ratios.push(ratio);
            clipped.push(objective);
            value_losses.push(value_loss);
            surrogate_sum += objective;
            value_sum += value_loss;
            adv_sum += advantage;
            n += 1;
        }

        if n == 0 {
            return Err(RlStepError("no valid samples".to_string()));
        }
        Ok(RlStepResult {
            advantages,
            ratios,
            clipped_objectives: clipped,
            value_losses,
            policy_loss: -surrogate_sum / n as f64,
            value_loss: params.value_coef * value_sum / n as f64,
            mean_advantage: adv_sum / n as f64,
        })
    }
}

/// rl-step 插件:提供 rl-step seam。
pub struct RlStepPlugin;

impl Plugin for RlStepPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-rl-step"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![RL_STEP]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let step: Arc<dyn RlStep> = Arc::new(PpoRlStep);
        Ok(vec![ctx.register(RL_STEP, step)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::RL_STEP;
    use ah_contracts::rl_step::RlStep;
    use ah_hub::plugin::DynPlugin;
    use std::sync::Arc as StdArc;

    fn sample(reward: f64, value: f64, log_prob: f64, old_log_prob: f64, valid: bool) -> RlSample {
        RlSample {
            reward,
            value,
            old_log_prob,
            log_prob,
            valid,
        }
    }

    #[test]
    fn step_computes_advantage_ratio_and_clipped_objective() {
        let step = PpoRlStep;
        // log_prob = old → ratio = 1,advantage = 0.5。
        let samples = vec![
            sample(0.8, 0.3, -1.0, -1.0, true),
            sample(0.2, 0.5, -2.0, -2.0, true),
        ];
        let result = step.step(&samples, &PpoParams::default()).expect("step");
        assert_eq!(result.advantages.len(), 2);
        assert!((result.advantages[0] - 0.5).abs() < 1e-9, "0.8-0.3");
        assert!((result.ratios[0] - 1.0).abs() < 1e-9, "ratio=1");
        assert!(
            (result.clipped_objectives[0] - 0.5).abs() < 1e-9,
            "objective=adv"
        );
        assert!(result.policy_loss < 0.0, "negative surrogate mean");
        assert!(result.value_loss > 0.0, "value loss positive");
        assert!(
            (result.mean_advantage - 0.1).abs() < 1e-9,
            "mean advantage (0.5 + -0.3)/2"
        );
    }

    #[test]
    fn clipping_bounds_ratio_when_advantage_positive() {
        let step = PpoRlStep;
        // 大幅策略更新:ratio = exp(3) ≈ 20 → 被 clip 到 1.2。
        let samples = vec![sample(1.0, 0.0, 0.0, -3.0, true)];
        let result = step.step(&samples, &PpoParams::default()).expect("step");
        assert!(result.ratios[0] > 10.0, "raw ratio large");
        assert!(
            (result.clipped_objectives[0] - 1.2).abs() < 1e-9,
            "clipped to 1.2"
        );
    }

    #[test]
    fn invalid_and_nonfinite_samples_are_excluded() {
        let step = PpoRlStep;
        let samples = vec![
            sample(1.0, 0.0, 0.0, 0.0, false),     // invalid → 剔除
            sample(f64::NAN, 0.0, 0.0, 0.0, true), // 非有限 → 剔除
            sample(1.0, 0.5, -1.0, -1.0, true),    // 有效
        ];
        let result = step.step(&samples, &PpoParams::default()).expect("step");
        assert_eq!(result.advantages.len(), 1, "only valid finite sample");
        assert!((result.mean_advantage - 0.5).abs() < 1e-9);

        // 全无效 → 显式报错。
        assert!(
            step.step(&[sample(1.0, 0.0, 0.0, 0.0, false)], &PpoParams::default())
                .is_err()
        );
    }

    #[test]
    fn plugin_registers_rl_step() {
        let ctx = Context::new();
        let plugin: DynPlugin = StdArc::new(RlStepPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let step = ctx.service::<dyn RlStep>(&RL_STEP).expect("rl step");
        let result = step
            .step(&[sample(0.9, 0.2, -1.0, -1.0, true)], &PpoParams::default())
            .expect("step");
        assert!((result.advantages[0] - 0.7).abs() < 1e-9);
        drop(effects);
        assert!(!ctx.has_service(&RL_STEP));
    }
}
