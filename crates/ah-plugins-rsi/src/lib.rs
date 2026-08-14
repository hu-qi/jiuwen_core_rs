//! # ah-plugins-rsi
//!
//! 真实 RSI(递归自改进)运行时:
//! - 数据集生成:确定性扩展(改写/组合/边界约束),真实去重;
//! - 用例执行:经 SubagentRuntime 真实委派,expected 匹配或 evolving 轨迹评估;
//! - 报告聚合:一轮内全部用例的真实结果;
//! - 任务提示精化:对最差用例轨迹做 evolving 评估与优化,生成精化提示;
//! - checkpoint:JSONL 落盘,支持中断续跑。

use std::path::PathBuf;
use std::sync::Arc;

use ah_contracts::evolving::{EvolvingRuntime, RefinementTarget, Verdict};
use ah_contracts::keys::{EVOLVING, RSI, SUBAGENT};
use ah_contracts::prelude::Effect;
use ah_contracts::rsi::{RsiCase, RsiCheckpoint, RsiError, RsiReport, RsiRunOutcome, RsiRuntime};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::subagent::{SubagentRuntime, SubagentSpec};
use ah_hub::context::Context;

pub mod analyzer;
pub use analyzer::{AnalyzerPlugin, RuleBasedAnalyzer};

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;

/// 真实 RSI 运行时。
pub struct RsiRuntimeImpl {
    subagent: Arc<dyn SubagentRuntime>,
    evolving: Arc<dyn EvolvingRuntime>,
    /// checkpoint 目录(JSONL 文件)。
    dir: PathBuf,
}

impl RsiRuntimeImpl {
    /// 构造运行时(需要 subagent 与 evolving seam;checkpoint 写入 dir)。
    pub fn new(
        subagent: Arc<dyn SubagentRuntime>,
        evolving: Arc<dyn EvolvingRuntime>,
        dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            subagent,
            evolving,
            dir: dir.into(),
        }
    }

    fn checkpoint_path(&self) -> PathBuf {
        self.dir.join("rsi_checkpoints.jsonl")
    }
}

impl Seam for RsiRuntimeImpl {}

#[async_trait]
impl RsiRuntime for RsiRuntimeImpl {
    fn generate_dataset(
        &self,
        seed_tasks: Vec<String>,
        count: usize,
    ) -> Result<Vec<RsiCase>, RsiError> {
        if seed_tasks.is_empty() {
            return Err(RsiError("no seed tasks".to_string()));
        }
        if count == 0 {
            return Ok(vec![]);
        }
        // 确定性扩展模式(真实改写/组合/边界约束;LLM 生成留待后续)。
        let mut cases: Vec<RsiCase> = Vec::new();
        let mut round = 0usize;
        while cases.len() < count {
            let seed_idx = round % seed_tasks.len();
            let pattern = (round / seed_tasks.len()) % 4;
            let seed = &seed_tasks[seed_idx];
            let (id_suffix, task) = match pattern {
                0 => ("orig", seed.clone()),
                1 => (
                    "explain",
                    format!("{seed} Then explain the steps you took."),
                ),
                2 => (
                    "summarize",
                    format!("First {seed} Then summarize what changed."),
                ),
                _ => (
                    "minimal",
                    format!("{seed} Keep the number of steps minimal."),
                ),
            };
            let id = format!("s{seed_idx}-{id_suffix}");
            if !cases.iter().any(|c| c.id == id) {
                cases.push(RsiCase {
                    id: id.clone(),
                    task,
                    expected: None,
                });
            }
            round += 1;
        }
        Ok(cases)
    }

    async fn run_case(
        &self,
        round: u32,
        case: &RsiCase,
        task_prompt: &str,
    ) -> Result<RsiRunOutcome, RsiError> {
        // 隔离会话:rsi-r{round}-{case_id},执行后可抽取轨迹。
        let session_id = format!("rsi-r{round}-{}", case.id);
        let context = Some(format!(
            "{task_prompt}\n\nYou must produce a final answer for the case below."
        ));
        let result = self
            .subagent
            .run(SubagentSpec {
                id: session_id.clone(),
                task: case.task.clone(),
                context,
                budget: Some(8),
            })
            .await;
        let result = match result {
            Ok(r) => r,
            Err(e) => {
                return Ok(RsiRunOutcome {
                    case_id: case.id.clone(),
                    passed: false,
                    score: 0.0,
                    answer: String::new(),
                    error: Some(e.0),
                });
            }
        };

        // expected 匹配优先(确定性判据)。
        if let Some(expected) = &case.expected {
            let passed = result.answer.contains(expected);
            return Ok(RsiRunOutcome {
                case_id: case.id.clone(),
                passed,
                score: if passed { 1.0 } else { 0.0 },
                answer: result.answer,
                error: None,
            });
        }

        // 轨迹评估(evolving 本地判据 + LLM judge)。
        let traj = self
            .evolving
            .extract_session(&case.task, &session_id)
            .map_err(|e| RsiError(format!("extract trajectory: {e}")))?;
        let eval = self
            .evolving
            .evaluate(&traj)
            .await
            .map_err(|e| RsiError(format!("evaluate trajectory: {e}")))?;
        let passed = eval.verdict == Verdict::Pass;
        Ok(RsiRunOutcome {
            case_id: case.id.clone(),
            passed,
            score: eval.score,
            answer: result.answer,
            error: None,
        })
    }

    async fn evaluate_round(
        &self,
        round: u32,
        cases: &[RsiCase],
        task_prompt: &str,
    ) -> Result<RsiReport, RsiError> {
        let mut outcomes: Vec<RsiRunOutcome> = Vec::with_capacity(cases.len());
        for case in cases {
            let outcome = self.run_case(round, case, task_prompt).await?;
            outcomes.push(outcome);
        }
        let total = outcomes.len();
        let passed = outcomes.iter().filter(|o| o.passed).count();
        let avg_score = if total == 0 {
            0.0
        } else {
            outcomes.iter().map(|o| o.score).sum::<f64>() / total as f64
        };
        let mut issues: Vec<String> = Vec::new();
        for o in &outcomes {
            if !o.passed {
                let reason = o
                    .error
                    .clone()
                    .unwrap_or_else(|| format!("failed (score {:.2})", o.score));
                issues.push(format!("case {}: {reason}", o.case_id));
            }
        }
        let summary = format!("round {round}: {passed}/{total} passed, avg score {avg_score:.2}");
        Ok(RsiReport {
            round,
            total,
            passed,
            avg_score,
            issues,
            summary,
        })
    }

    fn save_checkpoint(&self, checkpoint: &RsiCheckpoint) -> Result<(), RsiError> {
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| RsiError(format!("create checkpoint dir: {e}")))?;
        let line = serde_json::to_string(checkpoint)
            .map_err(|e| RsiError(format!("serialize checkpoint: {e}")))?;
        let path = self.checkpoint_path();
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| RsiError(format!("open checkpoint: {e}")))?;
        use std::io::Write;
        writeln!(file, "{line}").map_err(|e| RsiError(format!("write checkpoint: {e}")))?;
        Ok(())
    }

    fn load_checkpoint(&self) -> Result<Option<RsiCheckpoint>, RsiError> {
        let path = self.checkpoint_path();
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|e| RsiError(format!("read checkpoint: {e}")))?;
        let latest = content.lines().next_back();
        match latest {
            Some(line) => serde_json::from_str(line)
                .map(Some)
                .map_err(|e| RsiError(format!("parse checkpoint: {e}"))),
            None => Ok(None),
        }
    }

    async fn refine_task(&self, report: &RsiReport, task_prompt: &str) -> Result<String, RsiError> {
        if report.issues.is_empty() {
            return Ok(task_prompt.to_string());
        }
        // 取最差用例:从第一条 issue 解析 case id,读其真实轨迹。
        let first = &report.issues[0];
        let case_id = first
            .strip_prefix("case ")
            .and_then(|s| s.split(':').next())
            .unwrap_or_default();
        if case_id.is_empty() {
            return Ok(task_prompt.to_string());
        }
        let session_id = format!("rsi-r{}-{case_id}", report.round);
        let traj = self
            .evolving
            .extract_session("", &session_id)
            .map_err(|e| RsiError(format!("extract worst trajectory: {e}")))?;
        let eval = self
            .evolving
            .evaluate(&traj)
            .await
            .map_err(|e| RsiError(format!("evaluate worst trajectory: {e}")))?;
        let refinements = self
            .evolving
            .optimize(&traj, &eval)
            .await
            .map_err(|e| RsiError(format!("optimize worst trajectory: {e}")))?;
        // 取第一条高置信、非工具类的建议应用到提示。
        if let Some(r) = refinements
            .iter()
            .find(|r| r.confidence >= 0.5 && r.target != RefinementTarget::Tools)
        {
            Ok(format!(
                "{task_prompt}\n\n[RSI round {} refinement] {}(rationale: {})",
                report.round, r.suggestion, r.rationale
            ))
        } else {
            Ok(task_prompt.to_string())
        }
    }

    async fn run_rounds(
        &self,
        seed_tasks: Vec<String>,
        rounds: u32,
        task_prompt: &str,
    ) -> Result<Vec<RsiReport>, RsiError> {
        if rounds == 0 {
            return Ok(vec![]);
        }
        let cases = self.generate_dataset(seed_tasks, 3)?;
        let mut prompt = task_prompt.to_string();
        let mut reports: Vec<RsiReport> = Vec::new();
        for round in 1..=rounds {
            // 检查点续跑:该轮已由 checkpoint 覆盖则跳过(只补跑新轮)。
            if let Some(cp) = self.load_checkpoint()?
                && cp.round >= round
            {
                prompt = cp.task_prompt;
                continue;
            }
            let report = self.evaluate_round(round, &cases, &prompt).await?;
            prompt = self.refine_task(&report, &prompt).await?;
            self.save_checkpoint(&RsiCheckpoint {
                round,
                cases: cases.clone(),
                task_prompt: prompt.clone(),
                updated_at_ms: now_ms(),
            })?;
            reports.push(report);
        }
        Ok(reports)
    }
}

/// rsi 插件:注入 subagent 与 evolving,提供 rsi seam。
pub struct RsiPlugin {
    dir: PathBuf,
}

impl RsiPlugin {
    /// 以 checkpoint 目录创建插件。
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

impl Plugin for RsiPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-rsi"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![RSI]
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
        let runtime: Arc<dyn RsiRuntime> =
            Arc::new(RsiRuntimeImpl::new(subagent, evolving, self.dir.clone()));
        Ok(vec![ctx.register(RSI, runtime)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::{RSI, SESSION_MANAGER};
    use ah_contracts::session::{SessionEventKind, SessionManager};
    use ah_hub::plugin::DynPlugin;
    use serde_json::json;
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
            StdArc::new(ah_plugins_evolving::EvolvingPlugin),
            StdArc::new(RsiPlugin::new(root.join("rsi"))),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[tokio::test]
    async fn generate_dataset_expands_seeds() {
        let root = std::env::temp_dir().join(format!("ah-rsi-gen-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let rsi = ctx.service::<dyn RsiRuntime>(&RSI).expect("rsi");

        let cases = rsi
            .generate_dataset(vec!["list files".to_string()], 4)
            .expect("gen");
        assert_eq!(cases.len(), 4);
        let mut ids: Vec<&str> = cases.iter().map(|c| c.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 4, "ids must be unique");
        assert!(cases.iter().all(|c| c.task.contains("list files")));

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn run_case_delegates_for_real() {
        let root = std::env::temp_dir().join(format!("ah-rsi-run-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let rsi = ctx.service::<dyn RsiRuntime>(&RSI).expect("rsi");

        let case = RsiCase {
            id: "c1".to_string(),
            task: "list the workspace".to_string(),
            expected: Some("mock final answer".to_string()),
        };
        let outcome = rsi
            .run_case(1, &case, "You are a helpful agent")
            .await
            .expect("run");
        // mock 模型确定性地给出 "mock final answer",expected 匹配即通过。
        assert!(outcome.passed);
        assert_eq!(outcome.score, 1.0);
        assert!(outcome.answer.contains("mock final answer"));

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn evaluate_round_aggregates_report() {
        let root = std::env::temp_dir().join(format!("ah-rsi-round-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let rsi = ctx.service::<dyn RsiRuntime>(&RSI).expect("rsi");

        let cases = vec![
            RsiCase {
                id: "c1".into(),
                task: "list files".into(),
                expected: Some("mock final answer".into()),
            },
            RsiCase {
                id: "c2".into(),
                task: "read a file".into(),
                expected: Some("mock final answer".into()),
            },
        ];
        let report = rsi
            .evaluate_round(1, &cases, "You are a helpful agent")
            .await
            .expect("round");
        assert_eq!(report.total, 2);
        assert_eq!(report.passed, 2);
        assert!(report.avg_score >= 0.9);
        assert!(report.issues.is_empty());
        assert!(report.summary.contains("round 1"));

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn checkpoint_roundtrip() {
        let root = std::env::temp_dir().join(format!("ah-rsi-cp-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let rsi = ctx.service::<dyn RsiRuntime>(&RSI).expect("rsi");

        let cp = RsiCheckpoint {
            round: 3,
            cases: vec![RsiCase {
                id: "c1".into(),
                task: "t".into(),
                expected: None,
            }],
            task_prompt: "prompt v3".to_string(),
            updated_at_ms: 12345,
        };
        rsi.save_checkpoint(&cp).expect("save");
        let loaded = rsi.load_checkpoint().expect("load").expect("some");
        assert_eq!(loaded.round, 3);
        assert_eq!(loaded.task_prompt, "prompt v3");
        assert_eq!(loaded.cases.len(), 1);

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn refine_task_uses_real_worst_trajectory() {
        let root = std::env::temp_dir().join(format!("ah-rsi-refine-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let rsi = ctx.service::<dyn RsiRuntime>(&RSI).expect("rsi");

        // 手工构造最差用例的真实会话:错误工具结果 + 未完成。
        let manager = ctx
            .service::<dyn SessionManager>(&SESSION_MANAGER)
            .expect("manager");
        let log = manager.create("rsi-r1-c1").expect("create");
        log.append(SessionEventKind::User, json!({"content": "do impossible"}))
            .expect("u");
        log.append(
            SessionEventKind::Assistant,
            json!({"tool_calls": [{"id": "e1", "name": "run_shell", "arguments": {"cmd": "boom"}}]}),
        )
        .expect("a");
        log.append(
            SessionEventKind::ToolResult,
            json!({"tool_call_id": "e1", "output": "error: command failed"}),
        )
        .expect("t");
        log.append(
            SessionEventKind::AgentStep,
            json!({"iteration": 3, "tool_calls": 1, "done": false}),
        )
        .expect("s");

        let report = RsiReport {
            round: 1,
            total: 1,
            passed: 0,
            avg_score: 0.15,
            issues: vec!["case c1: failed (score 0.15)".to_string()],
            summary: "round 1: 0/1 passed".to_string(),
        };
        let refined = rsi
            .refine_task(&report, "Do the task")
            .await
            .expect("refine");
        // 真实优化建议被应用到提示。
        assert!(refined.contains("RSI round 1 refinement"));
        assert!(refined.starts_with("Do the task"));

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn run_rounds_produces_reports_and_checkpoints() {
        let root = std::env::temp_dir().join(format!("ah-rsi-rounds-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let rsi = ctx.service::<dyn RsiRuntime>(&RSI).expect("rsi");

        let reports = rsi
            .run_rounds(
                vec!["list files".to_string()],
                2,
                "You are a helpful agent.",
            )
            .await
            .expect("rounds");
        assert_eq!(reports.len(), 2, "two fresh rounds");
        assert!(reports.iter().all(|r| r.summary.contains("round")));
        assert!(reports.iter().all(|r| r.total >= 1));

        // checkpoint 落盘:轮次推进被记录。
        let cp = rsi.load_checkpoint().expect("load").expect("some");
        assert_eq!(cp.round, 2);

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn run_rounds_resumes_from_checkpoint() {
        let root = std::env::temp_dir().join(format!("ah-rsi-resume-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let rsi = ctx.service::<dyn RsiRuntime>(&RSI).expect("rsi");

        // 第一轮跑 2 轮 → checkpoint round=2。
        let first = rsi
            .run_rounds(
                vec!["list files".to_string()],
                2,
                "You are a helpful agent.",
            )
            .await
            .expect("first run");
        assert_eq!(first.len(), 2);

        // 续跑 3 轮:前 2 轮被 checkpoint 覆盖跳过,只补跑第 3 轮。
        let resumed = rsi
            .run_rounds(
                vec!["list files".to_string()],
                3,
                "You are a helpful agent.",
            )
            .await
            .expect("resumed");
        assert_eq!(resumed.len(), 1, "only new round reported");
        assert_eq!(resumed[0].round, 3);

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
}
