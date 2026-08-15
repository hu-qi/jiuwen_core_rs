//! # single_harness 实现:迭代编排 + 候选门禁
//!
//! 真实单 harness 优化(对齐 Python rsi/single_harness):
//! - 数据集拆 train/holdout;
//! - 每 epoch:当前提示在 train 评测 → evolving 精化候选 → 候选在 holdout 评测;
//! - 候选门禁:holdout 得分严格优于当前 best 才接受(防过拟合);
//! - best 与 checkpoint JSONL 落盘,支持中断续跑。

use std::path::PathBuf;
use std::sync::Arc;

use ah_contracts::prelude::Effect;
use ah_contracts::rsi::{RsiCase, RsiError, RsiRuntime};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::single_harness::{
    EpochOutcome, SingleHarnessCheckpoint, SingleHarnessRequest, SingleHarnessResult,
    SingleHarnessRuntime,
};

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// single_harness 插件:注入 rsi,提供 single-harness seam。
pub struct SingleHarnessPlugin {
    dir: PathBuf,
}

impl SingleHarnessPlugin {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

impl ah_hub::plugin::Plugin for SingleHarnessPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-rsi-single-harness"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![ah_contracts::keys::SINGLE_HARNESS]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![ah_contracts::keys::RSI]
    }

    fn apply(
        &self,
        ctx: &ah_hub::context::Context,
    ) -> Result<Vec<Effect>, ah_hub::plugin::PluginError> {
        let rsi = ctx
            .service::<dyn RsiRuntime>(&ah_contracts::keys::RSI)
            .ok_or_else(|| ah_hub::plugin::PluginError::Apply {
                plugin: self.name(),
                message: "rsi seam not registered".to_string(),
            })?;
        let runtime: Arc<dyn SingleHarnessRuntime> =
            Arc::new(SingleHarnessRuntimeImpl::new(rsi, self.dir.clone()));
        Ok(vec![
            ctx.register(ah_contracts::keys::SINGLE_HARNESS, runtime),
        ])
    }
}

/// 真实 single_harness 运行时(复用 RsiRuntime 的评测/精化)。
pub struct SingleHarnessRuntimeImpl {
    rsi: Arc<dyn RsiRuntime>,
    dir: PathBuf,
}

impl SingleHarnessRuntimeImpl {
    pub fn new(rsi: Arc<dyn RsiRuntime>, dir: impl Into<PathBuf>) -> Self {
        Self {
            rsi,
            dir: dir.into(),
        }
    }

    fn checkpoint_path(&self) -> PathBuf {
        self.dir.join("single_harness.jsonl")
    }
}

impl Seam for SingleHarnessRuntimeImpl {}

#[async_trait::async_trait]
impl SingleHarnessRuntime for SingleHarnessRuntimeImpl {
    async fn optimize(
        &self,
        request: &SingleHarnessRequest,
    ) -> Result<SingleHarnessResult, RsiError> {
        if request.cases.is_empty() {
            return Err(RsiError("no cases".to_string()));
        }
        if request.max_epochs == 0 {
            return Err(RsiError("max_epochs must be > 0".to_string()));
        }
        let ratio = request.holdout_ratio.clamp(0.0, 0.9);
        let holdout_count = ((request.cases.len() as f64) * ratio).round() as usize;
        let train: Vec<RsiCase> = request.cases[..request.cases.len() - holdout_count].to_vec();
        let holdout: Vec<RsiCase> = request.cases[request.cases.len() - holdout_count..].to_vec();
        if train.is_empty() || holdout.is_empty() {
            return Err(RsiError(
                "holdout_ratio too small/large: need non-empty train and holdout".to_string(),
            ));
        }

        // 续跑:加载 checkpoint,跳过已完成 epoch。
        let checkpoint = self.load_checkpoint()?;
        let mut best_prompt = checkpoint
            .as_ref()
            .map(|c| c.best_prompt.clone())
            .unwrap_or_else(|| request.task_prompt.clone());
        let mut best_score: Option<f64> = checkpoint.as_ref().and_then(|c| c.best_score);
        let mut completed: Vec<u32> = checkpoint
            .as_ref()
            .map(|c| c.completed_epochs.clone())
            .unwrap_or_default();
        let mut epochs: Vec<EpochOutcome> = Vec::new();

        for epoch in 1..=request.max_epochs {
            if completed.contains(&epoch) {
                continue; // 已由 checkpoint 覆盖。
            }
            // 1) 当前 best 在 train 上评测(报告+评分)。
            let train_report = self.rsi.evaluate_round(epoch, &train, &best_prompt).await?;
            // 2) 精化候选提示。
            let candidate_prompt = self.rsi.refine_task(&train_report, &best_prompt).await?;
            // 3) 候选在 holdout 上评测(门禁依据)。
            let holdout_report = self
                .rsi
                .evaluate_round(epoch, &holdout, &candidate_prompt)
                .await?;
            let candidate_score = holdout_report.avg_score;
            // 4) 候选门禁:严格优于当前 best 才接受。
            let (accepted, gate_reason, new_best) = match best_score {
                Some(best) if candidate_score <= best => (
                    false,
                    format!(
                        "candidate {candidate_score:.2} <= best {best:.2}, rejected (holdout gate)"
                    ),
                    best,
                ),
                _ => (
                    true,
                    format!(
                        "candidate {candidate_score:.2} > best {:?}, accepted",
                        best_score
                    ),
                    candidate_score,
                ),
            };
            if accepted {
                best_prompt = candidate_prompt.clone();
                best_score = Some(new_best);
            }
            epochs.push(EpochOutcome {
                epoch,
                train_report,
                holdout_report,
                accepted,
                gate_reason,
                best_score: new_best,
            });
            completed.push(epoch);
            // checkpoint 落盘(续跑)。
            self.save_checkpoint(&SingleHarnessCheckpoint {
                epoch,
                best_prompt: best_prompt.clone(),
                best_score,
                completed_epochs: completed.clone(),
                updated_at_ms: now_ms(),
            })?;
        }

        Ok(SingleHarnessResult {
            epochs,
            published_prompt: best_prompt,
            best_score,
        })
    }

    fn save_checkpoint(&self, checkpoint: &SingleHarnessCheckpoint) -> Result<(), RsiError> {
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| RsiError(format!("create single_harness dir: {e}")))?;
        let line = serde_json::to_string(checkpoint)
            .map_err(|e| RsiError(format!("serialize checkpoint: {e}")))?;
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.checkpoint_path())
            .map_err(|e| RsiError(format!("open checkpoint: {e}")))?;
        writeln!(file, "{line}").map_err(|e| RsiError(format!("append checkpoint: {e}")))?;
        Ok(())
    }

    fn load_checkpoint(&self) -> Result<Option<SingleHarnessCheckpoint>, RsiError> {
        let path = self.checkpoint_path();
        if !path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| RsiError(format!("read checkpoint: {e}")))?;
        let mut latest: Option<SingleHarnessCheckpoint> = None;
        for line in text.lines() {
            if let Ok(checkpoint) = serde_json::from_str::<SingleHarnessCheckpoint>(line) {
                latest = Some(checkpoint);
            }
        }
        Ok(latest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::SINGLE_HARNESS;
    use ah_contracts::prelude::Effect;
    use ah_hub::context::Context;
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
            StdArc::new(crate::RsiPlugin::new(root.join("rsi"))),
            StdArc::new(SingleHarnessPlugin::new(root.join("single_harness"))),
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
    async fn optimize_gates_candidates_and_persists_best() {
        let root = std::env::temp_dir().join(format!("ah-sh-run-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let runtime = ctx
            .service::<dyn SingleHarnessRuntime>(&SINGLE_HARNESS)
            .expect("single harness");

        let result = runtime
            .optimize(&SingleHarnessRequest {
                cases: cases(6),
                task_prompt: "You are a helpful agent.".to_string(),
                max_epochs: 2,
                holdout_ratio: 0.33,
            })
            .await
            .expect("optimize");

        assert_eq!(result.epochs.len(), 2, "two epochs ran");
        assert!(!result.published_prompt.is_empty());
        assert!(result.best_score.is_some());
        // 门禁:best_score 非降(第一轮接受或拒绝后均为 >= 0)。
        let mut prev = -1.0;
        for epoch in &result.epochs {
            assert!(epoch.best_score >= prev, "best never decreases");
            prev = epoch.best_score;
        }
        // checkpoint 落盘。
        let checkpoint = runtime.load_checkpoint().expect("load");
        assert!(checkpoint.is_some(), "checkpoint persisted");
        assert_eq!(
            checkpoint.as_ref().unwrap().completed_epochs,
            vec![1, 2],
            "both epochs completed"
        );

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn resume_skips_completed_epochs() {
        let root = std::env::temp_dir().join(format!("ah-sh-resume-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let runtime = ctx
            .service::<dyn SingleHarnessRuntime>(&SINGLE_HARNESS)
            .expect("single harness");

        // 第一次:max_epochs=1。
        runtime
            .optimize(&SingleHarnessRequest {
                cases: cases(6),
                task_prompt: "You are a helpful agent.".to_string(),
                max_epochs: 1,
                holdout_ratio: 0.33,
            })
            .await
            .expect("first run");

        // 续跑:max_epochs=2 → 只执行 epoch 2。
        let result = runtime
            .optimize(&SingleHarnessRequest {
                cases: cases(6),
                task_prompt: "You are a helpful agent.".to_string(),
                max_epochs: 2,
                holdout_ratio: 0.33,
            })
            .await
            .expect("resume");
        assert_eq!(result.epochs.len(), 1, "only epoch 2 executed on resume");
        assert_eq!(result.epochs[0].epoch, 2);

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn checkpoint_roundtrip() {
        let root = std::env::temp_dir().join(format!("ah-sh-cp-{}", std::process::id()));
        let (ctx, _effects) = build_ctx(&root);
        // 直接构造 impl 验证落盘/加载。
        let runtime = SingleHarnessRuntimeImpl::new(
            ctx.service::<dyn RsiRuntime>(&ah_contracts::keys::RSI)
                .expect("rsi"),
            root.join("sh"),
        );
        let checkpoint = SingleHarnessCheckpoint {
            epoch: 3,
            best_prompt: "prompt v3".to_string(),
            best_score: Some(0.9),
            completed_epochs: vec![1, 2, 3],
            updated_at_ms: 42,
        };
        runtime.save_checkpoint(&checkpoint).expect("save");
        let loaded = runtime.load_checkpoint().expect("load").expect("some");
        assert_eq!(loaded.best_prompt, "prompt v3");
        assert_eq!(loaded.completed_epochs, vec![1, 2, 3]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
