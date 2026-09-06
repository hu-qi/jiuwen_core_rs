//! # ah-plugins-team-monitor
//!
//! 真实团队监控(对齐 Python agent_teams/monitor):
//! - 只读视图:团队信息/成员/任务(含认领人)/消息(经 TeamRuntime 查询);
//! - 事件流:订阅 teams/task 事件,按 seq 追加到监控日志(可回读);
//! - 流式诊断日志:TeamStreamLogger 聚合 token 流 chunk(对齐 stream_logger.py)。

pub mod stream_logger;

use std::sync::{Arc, Mutex};

use ah_contracts::keys::{TEAM_MONITOR, TEAMS};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::team_monitor::{
    MonitorError, MonitorEvent, MonitorEventType, TaskInfo, TeamInfo, TeamMonitor, status_label,
};
use ah_contracts::teams::{TeamMessage, TeamRuntime, TeamTaskEvent};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 真实团队监控。
pub struct LocalTeamMonitor {
    teams: Arc<dyn TeamRuntime>,
    events: Mutex<Vec<MonitorEvent>>,
    next_seq: Mutex<u64>,
}

impl LocalTeamMonitor {
    pub fn new(teams: Arc<dyn TeamRuntime>) -> Self {
        Self {
            teams,
            events: Mutex::new(Vec::new()),
            next_seq: Mutex::new(0),
        }
    }

    fn record(&self, kind: MonitorEventType, summary: String) {
        let mut seq = self.next_seq.lock().unwrap();
        let event = MonitorEvent {
            seq: *seq,
            ts_ms: now_ms(),
            kind,
            summary,
        };
        *seq += 1;
        self.events.lock().unwrap().push(event);
    }
}

impl Seam for LocalTeamMonitor {}

impl TeamMonitor for LocalTeamMonitor {
    fn team_info(&self, team: &str) -> Result<TeamInfo, MonitorError> {
        if !self.teams.list_teams().contains(&team.to_string()) {
            return Err(MonitorError(format!("team not found: {team}")));
        }
        // 成员信息由 TeamRuntime 不直接暴露;只读视图以任务/消息为主。
        Ok(TeamInfo {
            id: team.to_string(),
            name: team.to_string(),
            members: vec![],
        })
    }

    fn tasks(&self, team: &str) -> Result<Vec<TaskInfo>, MonitorError> {
        let tasks = self.teams.tasks(team).map_err(|e| MonitorError(e.0))?;
        Ok(tasks
            .into_iter()
            .map(|task| TaskInfo {
                member: task.assignee.clone(),
                task,
            })
            .collect())
    }

    fn messages(&self, team: &str) -> Result<Vec<TeamMessage>, MonitorError> {
        self.teams.messages(team).map_err(|e| MonitorError(e.0))
    }

    fn events(&self) -> Vec<MonitorEvent> {
        self.events.lock().unwrap().clone()
    }
}

/// team-monitor 插件:注入 teams seam,提供 team-monitor seam;
/// 订阅 teams/task 事件记录监控日志。
pub struct TeamMonitorPlugin;

impl Plugin for TeamMonitorPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-team-monitor"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![TEAM_MONITOR]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![TEAMS]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let teams = ctx
            .service::<dyn TeamRuntime>(&TEAMS)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "teams seam not registered".to_string(),
            })?;
        let concrete = Arc::new(LocalTeamMonitor::new(teams));
        let monitor: Arc<dyn TeamMonitor> = concrete.clone();
        let mut effects = vec![ctx.register(TEAM_MONITOR, monitor)];
        // 订阅 teams/task 事件(emit)记录监控日志(具体类型闭包,可逆注册)。
        effects.push(ctx.on::<TeamTaskEvent>(move |event| {
            let summary = format!("task {} -> {}", event.task_id, status_label(event.status));
            concrete.record(MonitorEventType::TaskStatusChanged, summary);
        }));
        Ok(effects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::TEAM_MONITOR;
    use ah_contracts::team_monitor::TeamMonitor;
    use ah_contracts::teams::{TeamMemberSpec, TeamRuntime, TeamSpec, TeamTask, TeamTaskStatus};
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
            StdArc::new(ah_plugins_queue::QueuePlugin::new(root.join("queue"))),
            StdArc::new(ah_plugins_teams::TeamsPlugin),
            StdArc::new(TeamMonitorPlugin),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[test]
    fn monitor_reads_team_state() {
        let root = std::env::temp_dir().join(format!("ah-tm-state-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let teams = ctx.service::<dyn TeamRuntime>(&TEAMS).expect("teams");
        let monitor = ctx
            .service::<dyn TeamMonitor>(&TEAM_MONITOR)
            .expect("monitor");

        teams
            .create_team(
                TeamSpec {
                    id: "t1".into(),
                    name: "alpha".into(),
                },
                vec![TeamMemberSpec {
                    id: "m1".into(),
                    name: "member".into(),
                    role: "default".into(),
                }],
            )
            .expect("create");
        teams
            .add_task(
                "t1",
                TeamTask {
                    id: "task-a".into(),
                    title: "do thing".into(),
                    content: "content".into(),
                    status: TeamTaskStatus::Pending,
                    dependencies: vec![],
                    assignee: None,
                    reviewers: vec![],
                    review_votes: vec![],
                    result: None,
                },
            )
            .expect("add task");
        teams
            .send_message("t1", "m1", None, "hello team")
            .expect("send");

        let info = monitor.team_info("t1").expect("info");
        assert_eq!(info.id, "t1");
        let tasks = monitor.tasks("t1").expect("tasks");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].task.id, "task-a");
        let messages = monitor.messages("t1").expect("messages");
        assert!(messages.iter().any(|m| m.content == "hello team"));

        // 不存在的团队显式报错。
        assert!(monitor.team_info("nope").is_err());

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn monitor_records_task_events() {
        let root = std::env::temp_dir().join(format!("ah-tm-events-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let teams = ctx.service::<dyn TeamRuntime>(&TEAMS).expect("teams");
        let monitor = ctx
            .service::<dyn TeamMonitor>(&TEAM_MONITOR)
            .expect("monitor");

        teams
            .create_team(
                TeamSpec {
                    id: "t1".into(),
                    name: "a".into(),
                },
                vec![],
            )
            .expect("create");
        teams
            .add_task(
                "t1",
                TeamTask {
                    id: "t1a".into(),
                    title: "x".into(),
                    content: "content".into(),
                    status: TeamTaskStatus::InProgress,
                    dependencies: vec![],
                    assignee: Some("m1".into()),
                    reviewers: vec![],
                    review_votes: vec![],
                    result: None,
                },
            )
            .expect("add");
        // 状态迁移触发 teams/task 事件。
        teams
            .complete_task("t1", "t1a", json!({"ok": true}))
            .expect("complete");

        let events = monitor.events();
        assert!(
            events.iter().any(
                |e| e.kind == MonitorEventType::TaskStatusChanged && e.summary.contains("Done")
            ),
            "task status event recorded: {:?}",
            events.iter().map(|e| e.summary.clone()).collect::<Vec<_>>()
        );
        // seq 单调。
        let seqs: Vec<u64> = events.iter().map(|e| e.seq).collect();
        let mut sorted = seqs.clone();
        sorted.sort_unstable();
        assert_eq!(seqs, sorted, "monotonic seq");

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
}
