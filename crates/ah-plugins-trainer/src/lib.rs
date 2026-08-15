//! # ah-plugins-trainer
//!
//! 真实自进化训练器(对齐 Python agent_evolving/trainer):
//! - 验证基线评估;
//! - 每 epoch:train 前向评估(经 RsiRuntime)→ Optimizer 应用文本梯度
//!   (经 OperatorRegistry)→ 验证集评估;
//! - 验证得分严格优于历史 best 才视为改进(门禁);
//! - best 达到 early_stop_score 提前停止。

use std::sync::Arc;

use ah_contracts::evolving::Evaluation;
use ah_contracts::keys::{OPERATOR, OPTIMIZER, RSI, TRAINER};
use ah_contracts::operator::OperatorRegistry;
use ah_contracts::optimizer::Optimizer;
use ah_contracts::prelude::Effect;
use ah_contracts::rsi::{RsiError, RsiRuntime};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::trainer::{TrainEpoch, TrainRequest, TrainResult, Trainer};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;

/// 真实训练器。
pub struct LocalTrainer {
    rsi: Arc<dyn RsiRuntime>,
    optimizer: Arc<dyn Optimizer>,
    operators: Arc<dyn OperatorRegistry>,
}

impl LocalTrainer {
    pub fn new(
        rsi: Arc<dyn RsiRuntime>,
        optimizer: Arc<dyn Optimizer>,
        operators: Arc<dyn OperatorRegistry>,
    ) -> Self {
        Self {
            rsi,
            optimizer,
            operators,
        }
    }

    /// 评估并转成 Evaluation 列表(供 optimizer backward)。
    fn evaluations_from_report(report: &ah_contracts::rsi::RsiReport) -> Vec<Evaluation> {
        if report.passed >= report.total {
            return vec![Evaluation {
                verdict: ah_contracts::evolving::Verdict::Pass,
                score: 1.0,
                strengths: vec!["all cases passed".to_string()],
                issues: vec![],
                feedback: String::new(),
            }];
        }
        vec![Evaluation {
            verdict: ah_contracts::evolving::Verdict::Fail,
            score: report.avg_score,
            strengths: vec![],
            issues: report.issues.clone(),
            feedback: report.summary.clone(),
        }]
    }
}

impl Seam for LocalTrainer {}

#[async_trait]
impl Trainer for LocalTrainer {
    async fn train(&self, request: &TrainRequest) -> Result<TrainResult, RsiError> {
        if request.train_cases.is_empty() {
            return Err(RsiError("train_cases must not be empty".to_string()));
        }
        if request.max_epochs == 0 {
            return Err(RsiError("max_epochs must be > 0".to_string()));
        }
        let val_cases = request
            .val_cases
            .clone()
            .unwrap_or_else(|| request.train_cases.clone());

        // 验证基线。
        let baseline = self
            .rsi
            .evaluate_round(0, &val_cases, &request.task_prompt)
            .await?;
        let mut best_score = baseline.avg_score;
        let mut epochs = Vec::new();
        let mut early_stopped = false;

        for epoch in 1..=request.max_epochs {
            // 1) train 前向评估。
            let train_report = self
                .rsi
                .evaluate_round(epoch, &request.train_cases, &request.task_prompt)
                .await?;
            // 2) optimizer:从 train 评估推导并应用文本梯度。
            let evaluations = Self::evaluations_from_report(&train_report);
            self.optimizer
                .apply_updates(&evaluations, self.operators.as_ref())
                .map_err(|e| RsiError(e.0))?;
            // 3) 验证集评估(门禁依据)。
            let val_report = self
                .rsi
                .evaluate_round(epoch, &val_cases, &request.task_prompt)
                .await?;
            let improved = val_report.avg_score > best_score + 1e-9;
            if improved {
                best_score = val_report.avg_score;
            }
            epochs.push(TrainEpoch {
                epoch,
                train_report,
                val_report,
                improved,
                best_score,
            });
            if best_score >= request.early_stop_score {
                early_stopped = true;
                break;
            }
        }

        Ok(TrainResult {
            epochs,
            task_prompt: request.task_prompt.clone(),
            best_score,
            early_stopped,
        })
    }
}

/// trainer 插件:注入 rsi + optimizer + operator,提供 trainer seam。
pub struct TrainerPlugin;

impl Plugin for TrainerPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-trainer"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![TRAINER]
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
        let trainer: Arc<dyn Trainer> = Arc::new(LocalTrainer::new(rsi, optimizer, operators));
        Ok(vec![ctx.register(TRAINER, trainer)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::TRAINER;
    use ah_contracts::rsi::RsiCase;
    use ah_contracts::trainer::Trainer;
    use ah_hub::plugin::DynPlugin;
    use std::sync::Arc as StdArc;

    fn build_ctx(root: &std::path::Path) -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let session_dir = root.join("sessions");
        let default_path = session_dir.join("default.jsonl");
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
            StdArc::new(ah_plugins_operator::OperatorPlugin),
            StdArc::new(ah_plugins_optimizer::OptimizerPlugin),
            StdArc::new(TrainerPlugin),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    fn cases(count: usize) -> Vec<RsiCase> {
        (0..count)
            .map(|i| RsiCase {
                id: format!("c{i}"),
                task: "explore the workspace".to_string(),
                expected: Some("mock final answer".to_string()),
            })
            .collect()
    }

    #[tokio::test]
    async fn train_runs_epochs_and_gates_improvement() {
        let root = std::env::temp_dir().join(format!("ah-train-run-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let trainer = ctx.service::<dyn Trainer>(&TRAINER).expect("trainer");

        let result = trainer
            .train(&TrainRequest {
                train_cases: cases(4),
                val_cases: None,
                task_prompt: "You are a helpful agent.".to_string(),
                max_epochs: 2,
                early_stop_score: 2.0, // 不可达阈值,确保两轮都跑。
            })
            .await
            .expect("train");

        assert_eq!(result.epochs.len(), 2, "two epochs ran");
        assert!(
            !result.early_stopped,
            "no early stop at unreachable threshold"
        );
        assert!(result.best_score >= 0.0);
        // 改进门禁:improved 反映验证得分严格提升。
        let mut prev = -1.0;
        for epoch in &result.epochs {
            assert!(epoch.best_score >= prev, "best never decreases");
            prev = epoch.best_score;
        }

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn early_stop_triggers_when_score_reaches_threshold() {
        let root = std::env::temp_dir().join(format!("ah-train-stop-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let trainer = ctx.service::<dyn Trainer>(&TRAINER).expect("trainer");

        // early_stop_score=0 → 第一轮后即停。
        let result = trainer
            .train(&TrainRequest {
                train_cases: cases(3),
                val_cases: None,
                task_prompt: "You are a helpful agent.".to_string(),
                max_epochs: 5,
                early_stop_score: 0.0,
            })
            .await
            .expect("train");
        assert_eq!(result.epochs.len(), 1, "early stop after first epoch");
        assert!(result.early_stopped);

        // 空训练集显式报错。
        assert!(
            trainer
                .train(&TrainRequest {
                    train_cases: vec![],
                    val_cases: None,
                    task_prompt: "x".to_string(),
                    max_epochs: 1,
                    early_stop_score: 1.0,
                })
                .await
                .is_err()
        );

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
}
