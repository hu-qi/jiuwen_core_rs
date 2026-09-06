//! # ah-plugins-member-optimizer
//!
//! 真实成员优化管线(对齐 Python rsi/member_optimizer):
//! - attribution:从分析问题(issues)确定性归因机制/lever/目标面;
//! - plan:归因 → 单一优化动作(文本梯度);
//! - execute:经 OperatorRegistry + Optimizer 应用梯度;
//! - verify:在 val 集重新评估,得分严格优于基线才通过;
//! - publish:best 提示 + 引用写入发布目录(真实 JSON 落盘)。

use std::path::PathBuf;
use std::sync::Arc;

use ah_contracts::evolving::Evaluation;
use ah_contracts::keys::{MEMBER_OPTIMIZER, OPERATOR, OPTIMIZER, RSI};
use ah_contracts::member_optimizer::{
    Attribution, Lever, MechanismType, MemberOptimizer, MemberOptimizerError, OptimizationPlan,
    PublishResult, Verification,
};
use ah_contracts::operator::OperatorRegistry;
use ah_contracts::optimizer::Optimizer;
use ah_contracts::prelude::Effect;
use ah_contracts::rsi::{RsiCase, RsiRuntime};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use serde_json::json;

/// 机制类型判定(问题文本 → 机制,真实规则)。
fn mechanism_for(issue: &str) -> MechanismType {
    let lower = issue.to_lowercase();
    if lower.contains("tool") || lower.contains("工具") {
        MechanismType::Tool
    } else if lower.contains("skill") || lower.contains("技能") {
        MechanismType::Skill
    } else if lower.contains("memory") || lower.contains("记忆") {
        MechanismType::Memory
    } else if lower.contains("workflow") || lower.contains("工作流") {
        MechanismType::Workflow
    } else if lower.contains("context") || lower.contains("上下文") {
        MechanismType::Context
    } else {
        MechanismType::Prompt
    }
}

fn lever_for(mechanism: MechanismType) -> Lever {
    match mechanism {
        MechanismType::Tool => Lever::Action,
        MechanismType::Workflow | MechanismType::Context => Lever::Control,
        MechanismType::Memory => Lever::Configuration,
        _ => Lever::Instruction,
    }
}

fn surface_for(mechanism: MechanismType) -> &'static str {
    match mechanism {
        MechanismType::Tool => "tool_call/tool_description",
        MechanismType::Skill => "skill_call/experience",
        MechanismType::Memory => "memory_call/max_retries",
        MechanismType::Workflow => "workflow/plan",
        MechanismType::Context => "context/summary",
        _ => "llm_call/system_prompt",
    }
}

/// 真实成员优化器。
pub struct LocalMemberOptimizer {
    rsi: Arc<dyn RsiRuntime>,
    optimizer: Arc<dyn Optimizer>,
    operators: Arc<dyn OperatorRegistry>,
    /// 验证集(从 RsiRuntime 无法直接取,构造时注入;单用例验证走 evaluate_round)。
    val_cases: Vec<RsiCase>,
    task_prompt: String,
    publish_dir: PathBuf,
}

impl LocalMemberOptimizer {
    pub fn new(
        rsi: Arc<dyn RsiRuntime>,
        optimizer: Arc<dyn Optimizer>,
        operators: Arc<dyn OperatorRegistry>,
        val_cases: Vec<RsiCase>,
        task_prompt: String,
        publish_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            rsi,
            optimizer,
            operators,
            val_cases,
            task_prompt,
            publish_dir: publish_dir.into(),
        }
    }
}

impl Seam for LocalMemberOptimizer {}

#[async_trait]
impl MemberOptimizer for LocalMemberOptimizer {
    fn attribute(&self, issues: &[String]) -> Vec<Attribution> {
        issues
            .iter()
            .map(|issue| {
                let mechanism = mechanism_for(issue);
                Attribution {
                    mechanism,
                    lever: lever_for(mechanism),
                    surface: surface_for(mechanism).to_string(),
                    issue: issue.clone(),
                }
            })
            .collect()
    }

    fn plan(&self, attribution: &Attribution) -> OptimizationPlan {
        let gradient = match attribution.mechanism {
            MechanismType::Tool => format!(
                "Tool correction: update the tool description to address: {}",
                attribution.issue
            ),
            MechanismType::Skill => format!("Skill experience note: {}", attribution.issue),
            MechanismType::Memory => {
                "Memory correction: retry failed memory operations once more.".to_string()
            }
            _ => format!(
                "Instruction correction: address this failure. Issue: {}",
                attribution.issue
            ),
        };
        OptimizationPlan {
            attribution: attribution.clone(),
            gradient,
        }
    }

    async fn execute_and_verify(
        &self,
        plan: &OptimizationPlan,
        baseline_score: f64,
    ) -> Result<Verification, MemberOptimizerError> {
        // execute:经 Optimizer 应用单条文本梯度。
        let evaluations = vec![Evaluation {
            verdict: ah_contracts::evolving::Verdict::Fail,
            score: baseline_score,
            strengths: vec![],
            issues: vec![plan.attribution.issue.clone()],
            feedback: String::new(),
        }];
        self.optimizer
            .apply_updates(&evaluations, self.operators.as_ref())
            .map_err(|e| MemberOptimizerError(e.0))?;

        // verify:在 val 集重新评估。
        let report = self
            .rsi
            .evaluate_round(0, &self.val_cases, &self.task_prompt)
            .await
            .map_err(|e| MemberOptimizerError(e.0))?;
        let score = report.avg_score;
        let passed = score > baseline_score + 1e-9;
        Ok(Verification {
            passed,
            score,
            baseline_score,
            reason: if passed {
                format!("score {score:.2} improved over baseline {baseline_score:.2}")
            } else {
                format!("score {score:.2} did not beat baseline {baseline_score:.2}")
            },
        })
    }

    fn publish(&self, prompt: &str, score: f64) -> Result<PublishResult, MemberOptimizerError> {
        std::fs::create_dir_all(&self.publish_dir)
            .map_err(|e| MemberOptimizerError(format!("create publish dir: {e}")))?;
        let refs_path = self.publish_dir.join("current_harness_refs.json");
        let payload = json!({
            "published_prompt": prompt,
            "score": score,
            "published_at_ms": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
        });
        std::fs::write(
            &refs_path,
            serde_json::to_string_pretty(&payload).unwrap_or_default(),
        )
        .map_err(|e| MemberOptimizerError(format!("write refs: {e}")))?;
        Ok(PublishResult {
            published_prompt: prompt.to_string(),
            refs_path: refs_path.display().to_string(),
            score,
        })
    }
}

/// member_optimizer 插件:注入 rsi + optimizer + operator,提供 member-optimizer seam。
pub struct MemberOptimizerPlugin {
    val_cases: Vec<RsiCase>,
    task_prompt: String,
    publish_dir: PathBuf,
}

impl MemberOptimizerPlugin {
    pub fn new(
        val_cases: Vec<RsiCase>,
        task_prompt: String,
        publish_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            val_cases,
            task_prompt,
            publish_dir: publish_dir.into(),
        }
    }
}

impl Plugin for MemberOptimizerPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-member-optimizer"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![MEMBER_OPTIMIZER]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![RSI, OPTIMIZER, OPERATOR]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let rsi = ctx
            .service::<dyn RsiRuntime>(&RSI)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "rsi seam not registered".to_string(),
            })?;
        let optimizer =
            ctx.service::<dyn Optimizer>(&OPTIMIZER)
                .ok_or_else(|| PluginError::Apply {
                    plugin: self.name(),
                    message: "optimizer seam not registered".to_string(),
                })?;
        let operators = ctx
            .service::<dyn OperatorRegistry>(&OPERATOR)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "operator seam not registered".to_string(),
            })?;
        let optimizer: Arc<dyn MemberOptimizer> = Arc::new(LocalMemberOptimizer::new(
            rsi,
            optimizer,
            operators,
            self.val_cases.clone(),
            self.task_prompt.clone(),
            self.publish_dir.clone(),
        ));
        Ok(vec![ctx.register(MEMBER_OPTIMIZER, optimizer)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::MEMBER_OPTIMIZER;
    use ah_contracts::member_optimizer::MemberOptimizer;
    use ah_hub::plugin::DynPlugin;
    use std::sync::Arc as StdArc;

    fn build_ctx(root: &std::path::Path) -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let session_dir = root.join("sessions");
        let default_path = session_dir.join("default.jsonl");
        let val_cases = vec![RsiCase {
            id: "v1".to_string(),
            task: "explore the workspace".to_string(),
            expected: Some("mock final answer".to_string()),
        }];
        let plugins: Vec<DynPlugin> = vec![
            StdArc::new(ah_plugins_mock::MockPlugin),
            StdArc::new(ah_plugins_tools::ToolsPlugin),
            StdArc::new(ah_plugins_sysop::SysopPlugin::new(root)),
            StdArc::new(ah_plugins_session_log::SessionLogPlugin::new(
                &default_path,
                &session_dir,
            )),
            StdArc::new(ah_plugins_subagent::SubagentPlugin),
            StdArc::new(ah_plugins_evolving::EvolvingPlugin::new(
                root.join("evolving"),
            )),
            StdArc::new(ah_plugins_rsi::RsiPlugin::new(root.join("rsi"))),
            StdArc::new(ah_plugins_evolving::UpdaterPlugin),
            StdArc::new(ah_plugins_operator::OperatorPlugin),
            StdArc::new(ah_plugins_optimizer::OptimizerPlugin),
            StdArc::new(MemberOptimizerPlugin::new(
                val_cases,
                "You are a helpful agent.".to_string(),
                root.join("publish"),
            )),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[test]
    fn attribution_routes_mechanism_and_lever() {
        let root = std::env::temp_dir().join(format!("ah-mo-attr-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let optimizer = ctx
            .service::<dyn MemberOptimizer>(&MEMBER_OPTIMIZER)
            .expect("member optimizer");

        let attrs = optimizer.attribute(&[
            "tool returned an error".to_string(),
            "memory is stale".to_string(),
            "output was wrong".to_string(),
        ]);
        assert_eq!(attrs.len(), 3);
        assert_eq!(attrs[0].mechanism, MechanismType::Tool);
        assert_eq!(attrs[0].lever, Lever::Action);
        assert_eq!(attrs[1].mechanism, MechanismType::Memory);
        assert_eq!(attrs[1].lever, Lever::Configuration);
        assert_eq!(attrs[2].mechanism, MechanismType::Prompt);
        assert_eq!(attrs[2].lever, Lever::Instruction);

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn full_pipeline_executes_verifies_and_publishes() {
        let root = std::env::temp_dir().join(format!("ah-mo-run-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let optimizer = ctx
            .service::<dyn MemberOptimizer>(&MEMBER_OPTIMIZER)
            .expect("member optimizer");

        let attrs = optimizer.attribute(&["tool call failed".to_string()]);
        let plan = optimizer.plan(&attrs[0]);
        assert!(plan.gradient.contains("Tool correction"));
        assert_eq!(plan.attribution.surface, "tool_call/tool_description");

        let verification = optimizer
            .execute_and_verify(&plan, 0.0)
            .await
            .expect("verify");
        assert!(verification.score >= 0.0);
        assert!(verification.reason.contains("score"));

        let publish = optimizer.publish("best prompt v2", 0.9).expect("publish");
        assert!(
            std::path::Path::new(&publish.refs_path).exists(),
            "refs persisted"
        );
        let text = std::fs::read_to_string(&publish.refs_path).expect("read refs");
        assert!(
            text.contains("best prompt v2"),
            "published prompt persisted"
        );

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
}
