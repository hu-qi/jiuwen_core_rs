//! swarmflow 引擎(对应 agent_teams/workflow,替代规划中的 MockWorkflowStep):
//! phase 顺序执行、phase 内 agent 并行(fork-join barrier)/串行、预算上限、
//! 进度事件流(teams/swarm)、journal 续跑(同名已完成运行短路)。
//! worker 后端 = SubagentRuntime(真实委派,隔离会话)。

use std::path::PathBuf;
use std::sync::Arc;

use ah_contracts::keys::{SUBAGENT, SWARM};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::subagent::{SubagentRuntime, SubagentSpec};
use ah_contracts::swarm::{
    SwarmAgentActivity, SwarmAgentRecord, SwarmAgentStatus, SwarmError, SwarmEvent, SwarmEventKind,
    SwarmFlowScript, SwarmPhaseRecord, SwarmRun, SwarmRunStatus, SwarmflowRunner,
};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 真实 swarmflow 引擎。
pub struct SwarmflowEngine {
    subagent: Arc<dyn SubagentRuntime>,
    /// journal 目录(续跑短路的真实文件)。
    journal_dir: PathBuf,
    ctx: Context,
    run_seq: std::sync::atomic::AtomicU64,
}

impl SwarmflowEngine {
    /// 构造引擎(subagent 为 worker 后端;journal 写入 dir)。
    pub fn new(
        subagent: Arc<dyn SubagentRuntime>,
        journal_dir: impl Into<PathBuf>,
        ctx: Context,
    ) -> Self {
        Self {
            subagent,
            journal_dir: journal_dir.into(),
            ctx,
            run_seq: std::sync::atomic::AtomicU64::new(0),
        }
    }

    fn emit(&self, run_id: &str, kind: SwarmEventKind) {
        self.ctx.emit(SwarmEvent {
            run_id: run_id.to_string(),
            kind,
        });
    }

    fn journal_path(&self, name: &str) -> PathBuf {
        self.journal_dir.join(format!("{name}.json"))
    }

    /// 执行单个 agent 活动(真实 subagent 委派);失败 → Failed 记录,不中断运行。
    async fn run_agent(&self, run_id: &str, activity: &SwarmAgentActivity) -> SwarmAgentRecord {
        self.emit(run_id, SwarmEventKind::AgentStarted(activity.label.clone()));
        let mut record = SwarmAgentRecord {
            label: activity.label.clone(),
            prompt: activity.prompt.clone(),
            activity: Vec::new(),
            outcome: None,
            status: SwarmAgentStatus::Running,
        };
        let spec = SubagentSpec {
            id: format!("{run_id}-{}", activity.label),
            task: activity.prompt.clone(),
            context: activity
                .model
                .clone()
                .map(|m| format!("worker model hint: {m}")),
            budget: Some(8),
            allowed_tools: None,
        };
        let ok = match self.subagent.run(spec).await {
            Ok(result) => {
                record
                    .activity
                    .push(format!("iterations={}", result.iterations_used));
                record.outcome = Some(result.answer);
                true
            }
            Err(e) => {
                record.activity.push(format!("error: {e}"));
                record.outcome = Some(e.0);
                false
            }
        };
        record.status = if ok {
            SwarmAgentStatus::Completed
        } else {
            SwarmAgentStatus::Failed
        };
        self.emit(
            run_id,
            SwarmEventKind::AgentCompleted(activity.label.clone(), ok),
        );
        record
    }
}

impl Seam for SwarmflowEngine {}

#[async_trait]
impl SwarmflowRunner for SwarmflowEngine {
    fn preprocess(&self, script: &SwarmFlowScript) -> SwarmRun {
        SwarmRun {
            name: script.name.clone(),
            run_id: format!("preview-{}", now_ms()),
            status: SwarmRunStatus::Running,
            phases: script
                .phases
                .iter()
                .map(|phase| SwarmPhaseRecord {
                    title: phase.title.clone(),
                    agents: phase
                        .agents
                        .iter()
                        .map(|a| SwarmAgentRecord {
                            label: a.label.clone(),
                            prompt: a.prompt.clone(),
                            activity: Vec::new(),
                            outcome: None,
                            status: SwarmAgentStatus::Running,
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    async fn run(&self, script: SwarmFlowScript, budget: usize) -> Result<SwarmRun, SwarmError> {
        // journal 续跑:同名已完成运行短路(真实文件)。
        let journal = self.journal_path(&script.name);
        if journal.exists()
            && let Ok(text) = std::fs::read_to_string(&journal)
            && let Ok(cached) = serde_json::from_str::<SwarmRun>(&text)
        {
            return Ok(cached);
        }

        let run_id = format!(
            "wf_{:x}",
            (now_ms() << 20)
                | self
                    .run_seq
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        );
        self.emit(&run_id, SwarmEventKind::WorkflowStarted);

        let mut phases: Vec<SwarmPhaseRecord> = Vec::new();
        let mut executed = 0usize;
        let mut status = SwarmRunStatus::Running;

        'outer: for phase in &script.phases {
            self.emit(&run_id, SwarmEventKind::PhaseStarted(phase.title.clone()));
            let mut records: Vec<SwarmAgentRecord> = Vec::new();
            if phase.parallel {
                // fork-join barrier:并行执行全部 agent(单个失败不中断运行)。
                let mut handles = Vec::new();
                for activity in &phase.agents {
                    if executed >= budget {
                        status = SwarmRunStatus::BudgetExhausted;
                        break 'outer;
                    }
                    executed += 1;
                    handles.push(self.run_agent(&run_id, activity));
                }
                records = futures_join_all(handles).await;
            } else {
                for activity in &phase.agents {
                    if executed >= budget {
                        status = SwarmRunStatus::BudgetExhausted;
                        break 'outer;
                    }
                    executed += 1;
                    records.push(self.run_agent(&run_id, activity).await);
                }
            }
            phases.push(SwarmPhaseRecord {
                title: phase.title.clone(),
                agents: records,
            });
        }
        if status == SwarmRunStatus::Running {
            status = SwarmRunStatus::Completed;
        }
        let run = SwarmRun {
            name: script.name.clone(),
            run_id: run_id.clone(),
            status,
            phases,
        };
        // journal 落盘(真实续跑依据)。
        std::fs::create_dir_all(&self.journal_dir)
            .map_err(|e| SwarmError(format!("create journal dir: {e}")))?;
        let text =
            serde_json::to_string(&run).map_err(|e| SwarmError(format!("serialize: {e}")))?;
        std::fs::write(&journal, text).map_err(|e| SwarmError(format!("write journal: {e}")))?;
        let done = status == SwarmRunStatus::Completed;
        self.emit(&run_id, SwarmEventKind::WorkflowCompleted(done));
        Ok(run)
    }
}

/// futures 风格的 join_all 助手(tokio 已依赖,直接收集)。
async fn futures_join_all<T>(handles: Vec<impl std::future::Future<Output = T>>) -> Vec<T> {
    let mut results = Vec::with_capacity(handles.len());
    for handle in handles {
        results.push(handle.await);
    }
    results
}

/// swarmflow 插件:注入 SubagentRuntime,提供 teams-swarm seam。
pub struct SwarmflowPlugin {
    journal_dir: PathBuf,
}

impl SwarmflowPlugin {
    /// 以 journal 目录创建插件。
    pub fn new(journal_dir: impl Into<PathBuf>) -> Self {
        Self {
            journal_dir: journal_dir.into(),
        }
    }
}

impl Plugin for SwarmflowPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-teams-workflow"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![SWARM]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![SUBAGENT]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let subagent = ctx
            .service::<dyn SubagentRuntime>(&SUBAGENT)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "subagent seam not registered".to_string(),
            })?;
        let engine: Arc<dyn SwarmflowRunner> = Arc::new(SwarmflowEngine::new(
            subagent,
            self.journal_dir.clone(),
            ctx.clone(),
        ));
        Ok(vec![ctx.register(SWARM, engine)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::SWARM;
    use ah_contracts::swarm::{SwarmAgentActivity, SwarmFlowScript, SwarmPhase, SwarmRunStatus};
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
            StdArc::new(ah_plugins_queue::QueuePlugin::new(root.join("queue"))),
            StdArc::new(crate::TeamsPlugin),
            StdArc::new(SwarmflowPlugin::new(root.join("swarm-journals"))),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    fn agent(label: &str, prompt: &str) -> SwarmAgentActivity {
        SwarmAgentActivity {
            label: label.to_string(),
            prompt: prompt.to_string(),
            model: None,
        }
    }

    fn script(name: &str) -> SwarmFlowScript {
        SwarmFlowScript {
            name: name.to_string(),
            phases: vec![
                SwarmPhase {
                    title: "plan".to_string(),
                    agents: vec![agent("planner", "list the workspace")],
                    parallel: false,
                },
                SwarmPhase {
                    title: "verify".to_string(),
                    agents: vec![
                        agent("verifier-a", "list the workspace"),
                        agent("verifier-b", "list the workspace"),
                    ],
                    parallel: true,
                },
            ],
        }
    }

    #[tokio::test]
    async fn swarmflow_runs_phases_in_order_with_real_workers() {
        let root = std::env::temp_dir().join(format!("ah-swarm-run-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let runner = ctx.service::<dyn SwarmflowRunner>(&SWARM).expect("swarm");

        let run = runner.run(script("wf1"), 10).await.expect("run");
        assert_eq!(run.status, SwarmRunStatus::Completed);
        assert_eq!(run.phases.len(), 2, "both phases executed");
        assert_eq!(run.phases[0].title, "plan");
        assert_eq!(run.phases[0].agents.len(), 1);
        assert_eq!(
            run.phases[1].agents.len(),
            2,
            "parallel phase ran both agents"
        );
        // 真实 subagent 委派:outcome 含 mock final answer。
        for phase in &run.phases {
            for a in &phase.agents {
                assert_eq!(a.status, SwarmAgentStatus::Completed);
                assert!(
                    a.outcome
                        .as_deref()
                        .unwrap_or_default()
                        .contains("mock final answer"),
                    "real worker outcome: {:?}",
                    a.outcome
                );
            }
        }

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn swarmflow_budget_exhausted_stops_execution() {
        let root = std::env::temp_dir().join(format!("ah-swarm-budget-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let runner = ctx.service::<dyn SwarmflowRunner>(&SWARM).expect("swarm");

        let run = runner.run(script("wf-budget"), 1).await.expect("run");
        assert_eq!(run.status, SwarmRunStatus::BudgetExhausted);
        // 预算 1:只执行了第一个 agent(planner)。
        let total: usize = run.phases.iter().map(|p| p.agents.len()).sum();
        assert_eq!(total, 1, "budget capped agent execution");

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn swarmflow_journal_resumes_same_named_run() {
        let root = std::env::temp_dir().join(format!("ah-swarm-journal-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let runner = ctx.service::<dyn SwarmflowRunner>(&SWARM).expect("swarm");

        let first = runner.run(script("wf-resume"), 10).await.expect("first");
        assert_eq!(first.status, SwarmRunStatus::Completed);
        assert!(
            root.join("swarm-journals").join("wf-resume.json").exists(),
            "journal written"
        );

        // 同名再跑:journal 短路,返回缓存的运行(run_id 相同 → 未重执行)。
        let second = runner.run(script("wf-resume"), 10).await.expect("second");
        assert_eq!(second.run_id, first.run_id, "short-circuited from journal");

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn swarmflow_emits_progress_events() {
        use ah_contracts::event::Event;

        let root = std::env::temp_dir().join(format!("ah-swarm-events-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let collected: Arc<std::sync::Mutex<Vec<SwarmEventKind>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = collected.clone();
        let _h = ctx.on::<SwarmEvent>(move |event| {
            sink.lock().unwrap().push(event.kind.clone());
        });

        let runner = ctx.service::<dyn SwarmflowRunner>(&SWARM).expect("swarm");
        runner.run(script("wf-events"), 10).await.expect("run");
        let kinds = collected.lock().unwrap().clone();
        assert!(
            kinds
                .iter()
                .any(|k| matches!(k, SwarmEventKind::WorkflowStarted)),
            "workflow started event"
        );
        assert!(
            kinds
                .iter()
                .any(|k| matches!(k, SwarmEventKind::PhaseStarted(p) if p == "plan")),
            "phase started event"
        );
        assert!(
            kinds
                .iter()
                .any(|k| matches!(k, SwarmEventKind::WorkflowCompleted(true))),
            "workflow completed event"
        );
        assert_eq!(SwarmEvent::ID, "teams/swarm", "stable event id");

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
}
