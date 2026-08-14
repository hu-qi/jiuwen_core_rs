//! # ah-plugins-autoharness
//!
//! 真实 auto_harness 编排:assess(git 状态)→ plan(rsi 数据集)→ implement(subagent 委派)
//! → verify(ci 门禁)→ commit(git 提交)→ publish(git 分支);任一阶段失败即停。

use std::sync::Arc;

use ah_contracts::autoharness::{
    AutoHarness, AutoHarnessConfig, AutoHarnessError, CycleResult, StageKind, StageResult,
};
use ah_contracts::ci::CiGateRunner;
use ah_contracts::git::GitProvider;
use ah_contracts::keys::{AUTO_HARNESS, CI, GIT, RSI, SUBAGENT};
use ah_contracts::prelude::Effect;
use ah_contracts::rsi::RsiRuntime;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::subagent::{SubagentRuntime, SubagentSpec};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 真实 auto_harness 编排器。
pub struct AutoHarnessOrchestrator {
    subagent: Arc<dyn SubagentRuntime>,
    rsi: Arc<dyn RsiRuntime>,
    ci: Arc<dyn CiGateRunner>,
    git: Arc<dyn GitProvider>,
}

impl AutoHarnessOrchestrator {
    /// 构造编排器(需要 subagent/rsi/ci/git seam)。
    pub fn new(
        subagent: Arc<dyn SubagentRuntime>,
        rsi: Arc<dyn RsiRuntime>,
        ci: Arc<dyn CiGateRunner>,
        git: Arc<dyn GitProvider>,
    ) -> Self {
        Self {
            subagent,
            rsi,
            ci,
            git,
        }
    }

    async fn run_stage(
        &self,
        stage: StageKind,
        config: &AutoHarnessConfig,
    ) -> Result<StageResult, AutoHarnessError> {
        let result = match stage {
            StageKind::Assess => {
                if !self.git.is_repo(&config.workspace) {
                    return Ok(StageResult {
                        stage,
                        ok: false,
                        detail: "workspace is not a git repository".to_string(),
                        commit: None,
                    });
                }
                let status = self
                    .git
                    .status(&config.workspace)
                    .map_err(|e| AutoHarnessError(e.0))?;
                let diff = self
                    .git
                    .diff_stat(&config.workspace)
                    .map_err(|e| AutoHarnessError(e.0))?;
                StageResult {
                    stage,
                    ok: true,
                    detail: format!(
                        "workspace assessed: {} dirty entries, {} uncommitted paths",
                        status.len(),
                        diff
                    ),
                    commit: None,
                }
            }
            StageKind::Plan => {
                let cases = self
                    .rsi
                    .generate_dataset(vec![config.task.clone()], 2)
                    .map_err(|e| AutoHarnessError(e.0))?;
                StageResult {
                    stage,
                    ok: true,
                    detail: format!("planned {} evaluation cases from task", cases.len()),
                    commit: None,
                }
            }
            StageKind::Implement => {
                let result = self
                    .subagent
                    .run(SubagentSpec {
                        id: format!("autoharness-{}", now_ms()),
                        task: config.task.clone(),
                        context: Some(format!(
                            "You are implementing a change in {}.",
                            config.workspace.display()
                        )),
                        budget: Some(6),
                    })
                    .await
                    .map_err(|e| AutoHarnessError(format!("implement failed: {e}")))?;
                StageResult {
                    stage,
                    ok: true,
                    detail: format!(
                        "subagent implemented in {} iterations",
                        result.iterations_used
                    ),
                    commit: None,
                }
            }
            StageKind::Verify => {
                if config.gates.is_empty() {
                    StageResult {
                        stage,
                        ok: true,
                        detail: "no gates configured".to_string(),
                        commit: None,
                    }
                } else {
                    let mut details = Vec::new();
                    for (i, gate) in config.gates.iter().enumerate() {
                        let result = self
                            .ci
                            .run_gate(ah_contracts::ci::CiGateRequest {
                                name: format!("gate{i}"),
                                command: gate.clone(),
                                cwd: Some(config.workspace.clone()),
                                timeout_ms: Some(120_000),
                            })
                            .await
                            .map_err(|e| AutoHarnessError(e.0))?;
                        if !result.passed {
                            return Ok(StageResult {
                                stage,
                                ok: false,
                                detail: format!(
                                    "gate{i} failed: {}",
                                    result.output.chars().take(200).collect::<String>()
                                ),
                                commit: None,
                            });
                        }
                        details.push(format!("gate{i} passed"));
                    }
                    StageResult {
                        stage,
                        ok: true,
                        detail: details.join(", "),
                        commit: None,
                    }
                }
            }
            StageKind::Commit => {
                // 未跟踪文件不在 git diff 中:用 status 判定是否有改动。
                let status = self
                    .git
                    .status(&config.workspace)
                    .map_err(|e| AutoHarnessError(e.0))?;
                if status.is_empty() {
                    StageResult {
                        stage,
                        ok: true,
                        detail: "no changes to commit".to_string(),
                        commit: None,
                    }
                } else {
                    self.git
                        .add(&config.workspace, &[])
                        .map_err(|e| AutoHarnessError(e.0))?;
                    let commit = self
                        .git
                        .commit(
                            &config.workspace,
                            &format!("auto-harness: {}", config.task),
                            "auto-harness",
                            "autoharness@agent-harness.local",
                        )
                        .map_err(|e| AutoHarnessError(e.0))?;
                    StageResult {
                        stage,
                        ok: true,
                        detail: format!("committed {}: {}", commit.hash, commit.subject),
                        commit: Some(commit.hash),
                    }
                }
            }
            StageKind::Publish => {
                let branch = format!("publish-{}", now_ms());
                self.git
                    .branch(&config.workspace, &branch)
                    .map_err(|e| AutoHarnessError(e.0))?;
                StageResult {
                    stage,
                    ok: true,
                    detail: format!("published on branch {branch}"),
                    commit: None,
                }
            }
        };
        Ok(result)
    }
}

impl Seam for AutoHarnessOrchestrator {}

#[async_trait]
impl AutoHarness for AutoHarnessOrchestrator {
    async fn run_cycle(&self, config: AutoHarnessConfig) -> Result<CycleResult, AutoHarnessError> {
        let mut stages = Vec::new();
        let mut failed_at: Option<StageKind> = None;
        for stage in [
            StageKind::Assess,
            StageKind::Plan,
            StageKind::Implement,
            StageKind::Verify,
            StageKind::Commit,
            StageKind::Publish,
        ] {
            let result = self.run_stage(stage, &config).await?;
            let ok = result.ok;
            stages.push(result);
            if !ok {
                failed_at = Some(stage);
                break;
            }
        }
        Ok(CycleResult { stages, failed_at })
    }
}

/// auto_harness 插件:注入 subagent/rsi/ci/git,提供 auto-harness seam。
pub struct AutoHarnessPlugin;

impl Plugin for AutoHarnessPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-autoharness"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![AUTO_HARNESS]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![SUBAGENT, RSI, CI, GIT]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let subagent = ctx
            .service::<dyn SubagentRuntime>(&SUBAGENT)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "subagent missing".into(),
            })?;
        let rsi = ctx
            .service::<dyn RsiRuntime>(&RSI)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "rsi missing".into(),
            })?;
        let ci = ctx
            .service::<dyn CiGateRunner>(&CI)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "ci missing".into(),
            })?;
        let git = ctx
            .service::<dyn GitProvider>(&GIT)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "git missing".into(),
            })?;
        let orchestrator: Arc<dyn AutoHarness> =
            Arc::new(AutoHarnessOrchestrator::new(subagent, rsi, ci, git));
        Ok(vec![ctx.register(AUTO_HARNESS, orchestrator)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::AUTO_HARNESS;
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
            StdArc::new(ah_plugins_evolving::EvolvingPlugin),
            StdArc::new(ah_plugins_rsi::RsiPlugin::new(root.join("rsi"))),
            StdArc::new(ah_plugins_git::GitPlugin),
            StdArc::new(ah_plugins_ci::CiPlugin),
            StdArc::new(AutoHarnessPlugin),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    /// 建一个已提交基线的 git 工作区。
    fn workspace(tag: &str) -> (std::path::PathBuf, Context, Vec<Effect>) {
        let root = std::env::temp_dir().join(format!("ah-ah-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(root.join("ws")).expect("mkdir ws");
        let (ctx, effects) = build_ctx(&root);
        let git = ctx
            .service::<dyn GitProvider>(&ah_contracts::keys::GIT)
            .expect("git");
        let ws = root.join("ws");
        git.init(&ws).expect("init");
        std::fs::write(ws.join("base.txt"), "base").expect("write base");
        git.add(&ws, &[]).expect("add");
        git.commit(&ws, "base", "t", "t@x").expect("commit base");
        (ws, ctx, effects)
    }

    #[tokio::test]
    async fn full_cycle_commits_real_change() {
        let (ws, ctx, effects) = workspace("cycle");
        // 模拟 implement 产生了真实改动(测试设置工作区状态,提交由编排器真实执行)。
        std::fs::write(ws.join("feature.txt"), "new feature").expect("write feature");

        let auto = ctx
            .service::<dyn AutoHarness>(&AUTO_HARNESS)
            .expect("auto-harness");
        let result = auto
            .run_cycle(AutoHarnessConfig {
                task: "add a feature".to_string(),
                workspace: ws.clone(),
                gates: vec![vec![
                    "python3".to_string(),
                    "-c".to_string(),
                    "print('ok')".to_string(),
                ]],
            })
            .await
            .expect("cycle");
        assert!(
            result.failed_at.is_none(),
            "all stages passed: {:?}",
            result.stages
        );
        assert_eq!(result.stages.len(), 6, "all six stages executed");
        let commit = result
            .stages
            .iter()
            .find(|s| s.stage == StageKind::Commit)
            .and_then(|s| s.commit.clone())
            .expect("commit hash produced");
        // 真实 git:提交存在,log 有 2 条。
        let git = ctx
            .service::<dyn GitProvider>(&ah_contracts::keys::GIT)
            .expect("git");
        let log = git.log(&ws, 5).expect("log");
        assert_eq!(log.len(), 2, "base + auto-harness commit");
        assert!(log.iter().any(|c| c.hash == commit));

        drop(effects);
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[tokio::test]
    async fn failing_gate_stops_cycle_before_commit() {
        let (ws, ctx, effects) = workspace("gatefail");
        std::fs::write(ws.join("feature.txt"), "x").expect("write");

        let auto = ctx
            .service::<dyn AutoHarness>(&AUTO_HARNESS)
            .expect("auto-harness");
        let result = auto
            .run_cycle(AutoHarnessConfig {
                task: "task".to_string(),
                workspace: ws.clone(),
                gates: vec![vec![
                    "python3".to_string(),
                    "-c".to_string(),
                    "import sys; sys.exit(1)".to_string(),
                ]],
            })
            .await
            .expect("cycle");
        assert_eq!(
            result.failed_at,
            Some(StageKind::Verify),
            "stopped at verify"
        );
        assert!(
            result.stages.iter().all(|s| s.stage != StageKind::Commit),
            "no commit stage ran"
        );
        // 工作区改动未提交。
        let git = ctx
            .service::<dyn GitProvider>(&ah_contracts::keys::GIT)
            .expect("git");
        assert_eq!(git.log(&ws, 5).expect("log").len(), 1, "base only");

        drop(effects);
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[tokio::test]
    async fn clean_workspace_reports_no_changes() {
        let (ws, ctx, effects) = workspace("clean");
        let auto = ctx
            .service::<dyn AutoHarness>(&AUTO_HARNESS)
            .expect("auto-harness");
        let result = auto
            .run_cycle(AutoHarnessConfig {
                task: "task".to_string(),
                workspace: ws.clone(),
                gates: vec![],
            })
            .await
            .expect("cycle");
        assert!(result.failed_at.is_none());
        let commit_stage = result
            .stages
            .iter()
            .find(|s| s.stage == StageKind::Commit)
            .expect("commit stage");
        assert!(commit_stage.detail.contains("no changes to commit"));

        drop(effects);
        let _ = std::fs::remove_dir_all(&ws);
    }
}
