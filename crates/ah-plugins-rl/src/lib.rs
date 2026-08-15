//! # ah-plugins-rl
//!
//! 真实 RL 奖励函数(agent_rl 的 reward 部分;VERL/PPO 训练、LoRA、gateway
//! 留待后续,文档注明)。确定性数学:通过 + 基础奖励,失败 + 惩罚,
//! 超时/工具错误/迭代数逐项扣罚。

use ah_contracts::keys::REWARD;
use ah_contracts::prelude::Effect;
use ah_contracts::reward::{RewardCase, RewardConfig, RewardError, RewardFunction, RewardOutput};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 确定性奖励函数。
pub struct LinearRewardFunction;

impl Seam for LinearRewardFunction {}

impl RewardFunction for LinearRewardFunction {
    fn default_config(&self) -> RewardConfig {
        RewardConfig::default()
    }

    fn compute(
        &self,
        config: &RewardConfig,
        cases: &[RewardCase],
    ) -> Result<RewardOutput, RewardError> {
        if cases.is_empty() {
            return Err(RewardError("no reward cases".to_string()));
        }
        let mut per_case = Vec::with_capacity(cases.len());
        let mut total = 0.0;
        for case in cases {
            let mut reward = if case.passed {
                config.pass_reward + case.score
            } else {
                config.fail_penalty + case.score * 0.5
            };
            if case.timed_out {
                reward += config.timeout_penalty;
            }
            reward += config.tool_error_penalty * case.tool_errors as f64;
            reward += config.iteration_penalty * case.iterations as f64;
            per_case.push(reward);
            total += reward;
        }
        let mean = total / cases.len() as f64;
        Ok(RewardOutput {
            per_case,
            mean,
            total,
        })
    }
}

/// rl 插件:提供奖励函数 seam。
pub struct RlPlugin;

impl Plugin for RlPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-rl"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![REWARD]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let reward: std::sync::Arc<dyn RewardFunction> = std::sync::Arc::new(LinearRewardFunction);
        Ok(vec![ctx.register(REWARD, reward)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::REWARD;
    use ah_hub::plugin::DynPlugin;
    use std::sync::Arc as StdArc;

    fn build_ctx() -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![StdArc::new(RlPlugin)];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    fn case(
        id: &str,
        passed: bool,
        timed_out: bool,
        tool_errors: usize,
        iterations: u32,
    ) -> RewardCase {
        RewardCase {
            case_id: id.to_string(),
            passed,
            score: if passed { 1.0 } else { 0.1 },
            timed_out,
            tool_errors,
            iterations,
        }
    }

    #[test]
    fn passing_cases_earn_positive_reward() {
        let (ctx, effects) = build_ctx();
        let reward = ctx.service::<dyn RewardFunction>(&REWARD).expect("reward");
        let config = reward.default_config();
        let output = reward
            .compute(
                &config,
                &[case("a", true, false, 0, 2), case("b", true, false, 0, 1)],
            )
            .expect("compute");
        assert!(
            output.per_case[0] > 0.0,
            "passed case positive: {}",
            output.per_case[0]
        );
        assert!(output.mean > 0.0);
        assert!((output.total - (output.per_case[0] + output.per_case[1])).abs() < 1e-9);
        drop(effects);
    }

    #[test]
    fn penalties_apply_to_failed_cases() {
        let (ctx, effects) = build_ctx();
        let reward = ctx.service::<dyn RewardFunction>(&REWARD).expect("reward");
        let config = reward.default_config();

        // 失败 + 超时 + 2 工具错误 + 高迭代 → 明显负奖励。
        let output = reward
            .compute(&config, &[case("bad", false, true, 2, 10)])
            .expect("compute");
        let value = output.per_case[0];
        assert!(value < -1.0, "heavily penalized: {value}");
        assert!(output.mean < 0.0);

        // 对照:失败但无额外扣罚 > 失败 + 超时。
        let plain = reward
            .compute(&config, &[case("p", false, false, 0, 1)])
            .expect("p");
        assert!(plain.per_case[0] > value, "extra penalties reduce reward");

        drop(effects);
    }

    #[test]
    fn empty_input_errors_explicitly() {
        let (ctx, effects) = build_ctx();
        let reward = ctx.service::<dyn RewardFunction>(&REWARD).expect("reward");
        let err = reward
            .compute(&reward.default_config(), &[])
            .expect_err("empty");
        assert!(err.0.contains("no reward cases"));
        drop(effects);
    }
}
