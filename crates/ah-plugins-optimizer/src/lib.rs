//! # ah-plugins-optimizer
//!
//! 真实文本梯度优化器(对齐 Python agent_evolving/optimizer):
//! - backward:从评估问题(issues)推导文本梯度——失败/待改进信号 → 修正指令;
//! - step:经 OperatorRegistry 应用到算子(set_parameter,冻结参数拒绝并记录);
//! - 规则真实:按问题前缀路由到对应参数(llm_call/system_prompt 等)。

use std::sync::Arc;

use ah_contracts::evolving::{Evaluation, Verdict};
use ah_contracts::keys::OPTIMIZER;
use ah_contracts::operator::OperatorRegistry;
use ah_contracts::optimizer::{Optimizer, OptimizerError, TextualGradient, UpdateResult};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use serde_json::json;

/// 问题 → 参数映射(按问题前缀路由)。
fn route_parameter(issue: &str) -> (&'static str, &'static str) {
    let lower = issue.to_lowercase();
    if lower.contains("tool") || lower.contains("工具") {
        ("agent/tool_call", "tool_description")
    } else if lower.contains("memory") || lower.contains("记忆") {
        ("agent/memory_call", "max_retries")
    } else if lower.contains("skill") || lower.contains("技能") {
        ("agent/skill_call", "experience")
    } else {
        // 默认:llm_call 的 system_prompt。
        ("agent/llm_call", "system_prompt")
    }
}

/// 从一条问题生成修正指令(真实规则,确定性)。
fn gradient_for(issue: &str, operator_id: &str, parameter: &str) -> String {
    match (operator_id, parameter) {
        ("agent/llm_call", "system_prompt") => {
            format!(
                "Instruction correction: address this failure in your next attempt. Issue: {issue}"
            )
        }
        ("agent/tool_call", "tool_description") => {
            format!("Tool description correction: update to address: {issue}")
        }
        ("agent/memory_call", "max_retries") => {
            "Memory correction: retry failed memory operations once more.".to_string()
        }
        ("agent/skill_call", "experience") => {
            format!("Experience note: {issue}")
        }
        _ => format!("Correction for {operator_id}/{parameter}: {issue}"),
    }
}

/// 真实文本梯度优化器。
pub struct RuleBasedOptimizer;

impl Seam for RuleBasedOptimizer {}

impl Optimizer for RuleBasedOptimizer {
    fn backward(&self, evaluations: &[Evaluation]) -> Vec<TextualGradient> {
        let mut gradients = Vec::new();
        for evaluation in evaluations {
            // 仅失败/待改进信号。
            if evaluation.verdict == Verdict::Pass {
                continue;
            }
            for issue in &evaluation.issues {
                let (operator_id, parameter) = route_parameter(issue);
                gradients.push(TextualGradient {
                    operator_id: operator_id.to_string(),
                    parameter: parameter.to_string(),
                    gradient: gradient_for(issue, operator_id, parameter),
                    issue: issue.clone(),
                });
            }
        }
        gradients
    }

    fn step(
        &self,
        gradients: &[TextualGradient],
        registry: &dyn OperatorRegistry,
    ) -> Vec<UpdateResult> {
        let mut results = Vec::new();
        for gradient in gradients {
            let Some(operator) = registry.get(&gradient.operator_id) else {
                results.push(UpdateResult {
                    operator_id: gradient.operator_id.clone(),
                    parameter: gradient.parameter.clone(),
                    applied: false,
                    reason: Some("operator not registered".to_string()),
                });
                continue;
            };
            // 参数值按类型构造:tool_description 是字典,其余为字符串。
            let value = if gradient.parameter == "tool_description" {
                json!({ gradient.issue.clone(): gradient.gradient.clone() })
            } else {
                json!(gradient.gradient.clone())
            };
            match operator.set_parameter(&gradient.parameter, value) {
                Ok(()) => results.push(UpdateResult {
                    operator_id: gradient.operator_id.clone(),
                    parameter: gradient.parameter.clone(),
                    applied: true,
                    reason: None,
                }),
                Err(error) => results.push(UpdateResult {
                    operator_id: gradient.operator_id.clone(),
                    parameter: gradient.parameter.clone(),
                    applied: false,
                    reason: Some(error.0),
                }),
            }
        }
        results
    }

    fn apply_updates(
        &self,
        evaluations: &[Evaluation],
        registry: &dyn OperatorRegistry,
    ) -> Result<Vec<UpdateResult>, OptimizerError> {
        let gradients = self.backward(evaluations);
        Ok(self.step(&gradients, registry))
    }
}

/// optimizer 插件:提供 optimizer seam(不注入;消费方自行提供 OperatorRegistry)。
pub struct OptimizerPlugin;

impl Plugin for OptimizerPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-optimizer"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![OPTIMIZER]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let optimizer: Arc<dyn Optimizer> = Arc::new(RuleBasedOptimizer);
        Ok(vec![ctx.register(OPTIMIZER, optimizer)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::evolving::{Evaluation, Verdict};
    use ah_contracts::keys::OPTIMIZER;
    use ah_contracts::operator::OperatorRegistry;
    use ah_hub::plugin::DynPlugin;
    use std::sync::Arc as StdArc;

    fn build_ctx() -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![
            StdArc::new(ah_plugins_operator::OperatorPlugin),
            StdArc::new(OptimizerPlugin),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    fn failing_evaluation(issue: &str) -> Evaluation {
        Evaluation {
            verdict: Verdict::Fail,
            score: 0.2,
            strengths: vec![],
            issues: vec![issue.to_string()],
            feedback: String::new(),
        }
    }

    #[test]
    fn backward_only_selects_failing_evaluations() {
        let (ctx, effects) = build_ctx();
        let optimizer = ctx.service::<dyn Optimizer>(&OPTIMIZER).expect("optimizer");

        let gradients = optimizer.backward(&[
            failing_evaluation("tool returned an error"),
            Evaluation {
                verdict: Verdict::Pass,
                score: 1.0,
                strengths: vec!["ok".to_string()],
                issues: vec![],
                feedback: String::new(),
            },
        ]);
        assert_eq!(gradients.len(), 1, "pass is skipped");
        assert_eq!(gradients[0].parameter, "tool_description");
        assert!(gradients[0].gradient.contains("tool returned an error"));

        // 默认路由:无关键词 → llm_call/system_prompt。
        let gradients = optimizer.backward(&[failing_evaluation("output was wrong")]);
        assert_eq!(gradients[0].operator_id, "agent/llm_call");
        assert_eq!(gradients[0].parameter, "system_prompt");

        drop(effects);
    }

    #[test]
    fn step_applies_via_registry_and_reports_freeze() {
        let (ctx, effects) = build_ctx();
        let optimizer = ctx.service::<dyn Optimizer>(&OPTIMIZER).expect("optimizer");
        let registry = ctx
            .service::<dyn OperatorRegistry>(&ah_contracts::keys::OPERATOR)
            .expect("registry");

        let results = optimizer
            .apply_updates(&[failing_evaluation("tool call failed")], registry.as_ref())
            .expect("apply");
        assert_eq!(results.len(), 1);
        assert!(results[0].applied, "applied through registry");
        // 验证算子状态确实更新。
        let tool = registry.get("agent/tool_call").expect("tool op");
        let state = tool.get_state();
        let desc = state["tool_description"]
            .get("tool call failed")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        assert!(
            desc.contains("Tool description correction"),
            "real update: {desc}"
        );

        // 冻结参数显式拒绝(llm_call user_prompt 冻结)。
        let frozen = vec![TextualGradient {
            operator_id: "agent/llm_call".to_string(),
            parameter: "user_prompt".to_string(),
            gradient: "x".to_string(),
            issue: "frozen".to_string(),
        }];
        let results = optimizer.step(&frozen, registry.as_ref());
        assert!(!results[0].applied);
        assert!(
            results[0]
                .reason
                .as_deref()
                .unwrap_or("")
                .contains("frozen")
        );

        // 缺失算子显式记录。
        let missing = vec![TextualGradient {
            operator_id: "agent/nope".to_string(),
            parameter: "system_prompt".to_string(),
            gradient: "x".to_string(),
            issue: "missing".to_string(),
        }];
        let results = optimizer.step(&missing, registry.as_ref());
        assert!(!results[0].applied);
        assert!(
            results[0]
                .reason
                .as_deref()
                .unwrap_or("")
                .contains("not registered")
        );

        drop(effects);
    }
}
