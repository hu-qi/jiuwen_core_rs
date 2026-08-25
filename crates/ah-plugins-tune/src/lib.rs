//! # ah-plugins-tune
//!
//! 真实训练流水线(dev_tools tune):每轮用当前 prompt 委派 subagent 执行任务,
//! evolving 评估真实轨迹得分,evolving 优化建议精化 prompt,记录最优 prompt。
//! 另提供 `tune-kit` seam(确定性训练工具门面,见 kit.rs)。

mod kit;
pub use kit::{TuneKitImpl, TuneKitPlugin};

use std::sync::Arc;

use ah_contracts::evolving::{EvolvingRuntime, RefinementTarget};
use ah_contracts::keys::{EVOLVING, SUBAGENT, TUNE};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::subagent::{SubagentRuntime, SubagentSpec};
use ah_contracts::tune::{TuneError, TunePipeline, TuneRequest, TuneResult, TuneRoundResult};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;

/// 真实训练流水线。
pub struct TunePipelineImpl {
    subagent: Arc<dyn SubagentRuntime>,
    evolving: Arc<dyn EvolvingRuntime>,
}

impl TunePipelineImpl {
    pub fn new(subagent: Arc<dyn SubagentRuntime>, evolving: Arc<dyn EvolvingRuntime>) -> Self {
        Self { subagent, evolving }
    }
}

impl Seam for TunePipelineImpl {}

#[async_trait]
impl TunePipeline for TunePipelineImpl {
    async fn tune(&self, request: TuneRequest) -> Result<TuneResult, TuneError> {
        if request.seed_prompts.is_empty() {
            return Err(TuneError("no seed prompts".to_string()));
        }
        if request.rounds == 0 {
            return Err(TuneError("rounds must be positive".to_string()));
        }
        let mut prompt = request.seed_prompts[0].clone();
        let mut rounds: Vec<TuneRoundResult> = Vec::new();
        let mut best_prompt = prompt.clone();
        let mut best_score = f64::MIN;

        for round in 1..=request.rounds {
            let session_id = format!("tune-{round}-{}", request.task.len());
            let result = self
                .subagent
                .run(SubagentSpec {
                    id: session_id.clone(),
                    task: request.task.clone(),
                    context: Some(prompt.clone()),
                    budget: Some(6),
                    allowed_tools: None,
                })
                .await
                .map_err(|e| TuneError(format!("subagent failed: {e}")))?;
            let trajectory = self
                .evolving
                .extract_session(&request.task, &session_id)
                .map_err(|e| TuneError(format!("extract trajectory: {e}")))?;
            let evaluation = self
                .evolving
                .evaluate(&trajectory)
                .await
                .map_err(|e| TuneError(format!("evaluate: {e}")))?;
            let score = evaluation.score;

            // 优化:取非工具类建议精化 prompt。
            let refinements = self
                .evolving
                .optimize(&trajectory, &evaluation)
                .await
                .map_err(|e| TuneError(format!("optimize: {e}")))?;
            if let Some(r) = refinements
                .iter()
                .find(|r| r.target != RefinementTarget::Tools && r.confidence >= 0.5)
            {
                prompt = format!(
                    "{prompt}\n[tune round {round}] {}(rationale: {})",
                    r.suggestion, r.rationale
                );
            }

            if score > best_score {
                best_score = score;
                best_prompt = prompt.clone();
            }
            rounds.push(TuneRoundResult {
                round,
                prompt: prompt.clone(),
                score,
                issues: evaluation.issues,
            });
            let _ = result;
        }
        Ok(TuneResult {
            rounds,
            best_prompt,
            best_score,
        })
    }
}

/// tune 插件:注入 subagent + evolving,提供 tune seam。
pub struct TunePlugin;

impl Plugin for TunePlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-tune"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![TUNE]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![SUBAGENT, EVOLVING]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let subagent = ctx
            .service::<dyn SubagentRuntime>(&SUBAGENT)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "subagent seam not registered".to_string(),
            })?;
        let evolving = ctx
            .service::<dyn EvolvingRuntime>(&EVOLVING)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "evolving seam not registered".to_string(),
            })?;
        let pipeline: Arc<dyn TunePipeline> = Arc::new(TunePipelineImpl::new(subagent, evolving));
        Ok(vec![ctx.register(TUNE, pipeline)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::TUNE;
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
            StdArc::new(TunePlugin),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[tokio::test]
    async fn tune_runs_rounds_and_tracks_best() {
        let root = std::env::temp_dir().join(format!("ah-tune-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let tune = ctx.service::<dyn TunePipeline>(&TUNE).expect("tune");

        let result = tune
            .tune(TuneRequest {
                task: "list the workspace".to_string(),
                seed_prompts: vec!["You are helpful.".to_string()],
                rounds: 2,
            })
            .await
            .expect("tune");
        assert_eq!(result.rounds.len(), 2, "two training rounds");
        for round in &result.rounds {
            assert!((0.0..=1.0).contains(&round.score), "real evolving score");
        }
        assert!(!result.best_prompt.is_empty());
        assert!(result.best_score >= 0.0);
        // 每轮 prompt 真实精化(带 tune round 标注)。
        assert!(
            result.rounds[1].prompt.contains("[tune round 1]")
                || result.rounds[1].prompt.contains("[tune round 2]"),
            "prompt refined by optimizer: {}",
            result.rounds[1].prompt
        );

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn empty_seed_errors_explicitly() {
        let root = std::env::temp_dir().join(format!("ah-tune-err-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let tune = ctx.service::<dyn TunePipeline>(&TUNE).expect("tune");
        let err = tune
            .tune(TuneRequest {
                task: "x".to_string(),
                seed_prompts: vec![],
                rounds: 1,
            })
            .await
            .expect_err("no seeds");
        assert!(err.0.contains("no seed prompts"));
        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
}
