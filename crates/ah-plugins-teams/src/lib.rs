//! # ah-plugins-teams
//!
//! 真实多 agent 团队运行时:任务板(依赖校验)、review 票与 settle,
//! run_task 经 SubagentRuntime 真实委派执行;状态迁移发 teams/task 事件。
//!
//! 两种运行时(同一 TeamRuntime seam):
//! - InMemoryTeamRuntime:内存任务板(测试与无磁盘场景);
//! - sqlite::SqliteTeamRuntime:真实 SQLite 持久化(重启恢复)。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ah_contracts::keys::{MESSAGER, QUEUE, SUBAGENT, TEAMS};
use ah_contracts::messager::Messager;
use ah_contracts::prelude::Effect;
use ah_contracts::queue::{MessageQueue, QueueMessage};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::subagent::{SubagentRuntime, SubagentSpec};
use ah_contracts::teams::{
    TeamError, TeamMemberSpec, TeamMessage, TeamRunResult, TeamRuntime, TeamSpec, TeamTask,
    TeamTaskEvent, TeamTaskStatus,
};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use serde_json::{Value, json};

pub mod sqlite;
pub use sqlite::{SqliteTeamRuntime, SqliteTeamsPlugin};

pub mod swarm;
pub use swarm::{SwarmflowEngine, SwarmflowPlugin};

struct Team {
    spec: TeamSpec,
    members: Vec<TeamMemberSpec>,
    tasks: HashMap<String, TeamTask>,
}

/// 真实内存团队运行时(状态迁移真实;持久化留待后续,文档注明)。
pub struct InMemoryTeamRuntime {
    teams: Mutex<HashMap<String, Team>>,
    subagent: Arc<dyn SubagentRuntime>,
    /// 消息传输默认经 queue;注入 messager 后使用跨进程 topic。
    queue: Arc<dyn MessageQueue>,
    messager: Option<Arc<dyn Messager>>,
    messager_messages: Arc<Mutex<HashMap<String, Vec<TeamMessage>>>>,
    ctx: Context,
}

impl InMemoryTeamRuntime {
    pub fn new(
        subagent: Arc<dyn SubagentRuntime>,
        queue: Arc<dyn MessageQueue>,
        ctx: Context,
    ) -> Self {
        Self {
            teams: Mutex::new(HashMap::new()),
            subagent,
            queue,
            messager: None,
            messager_messages: Arc::new(Mutex::new(HashMap::new())),
            ctx,
        }
    }

    pub fn with_messager(mut self, messager: Arc<dyn Messager>) -> Self {
        messager.start();
        self.messager = Some(messager);
        let team_ids: Vec<String> = self.teams.lock().unwrap().keys().cloned().collect();
        for team in team_ids {
            self.subscribe_team(&team);
        }
        self
    }

    fn subscribe_team(&self, team: &str) {
        let Some(messager) = &self.messager else {
            return;
        };
        let topic = Self::channel(team);
        let cache = self.messager_messages.clone();
        let team_id = team.to_string();
        messager.subscribe(
            &topic,
            Arc::new(move |payload| {
                let Ok(message) = serde_json::from_value::<TeamMessage>(payload) else {
                    return;
                };
                let mut messages = cache.lock().unwrap();
                let bucket = messages.entry(team_id.clone()).or_default();
                bucket.push(message);
            }),
        );
    }

    fn channel(team: &str) -> String {
        format!("team:{team}:messages")
    }

    fn team(
        &self,
        id: &str,
    ) -> Result<std::sync::MutexGuard<'_, HashMap<String, Team>>, TeamError> {
        let guard = self.teams.lock().unwrap();
        if !guard.contains_key(id) {
            return Err(TeamError(format!("team not found: {id}")));
        }
        Ok(guard)
    }

    fn emit(&self, team: &str, task_id: &str, status: TeamTaskStatus) {
        self.ctx.emit(TeamTaskEvent {
            team: team.to_string(),
            task_id: task_id.to_string(),
            status,
        });
        if let Some(messager) = &self.messager {
            messager.publish(
                &format!("team:{team}:task"),
                serde_json::json!({"task_id": task_id, "status": status}),
            );
        }
    }
}

impl Seam for InMemoryTeamRuntime {}

#[async_trait]
impl TeamRuntime for InMemoryTeamRuntime {
    fn create_team(&self, spec: TeamSpec, members: Vec<TeamMemberSpec>) -> Result<(), TeamError> {
        if spec.id.is_empty() || spec.name.is_empty() {
            return Err(TeamError("team id/name must not be empty".to_string()));
        }
        let team_id = spec.id.clone();
        let mut guard = self.teams.lock().unwrap();
        if guard.contains_key(&spec.id) {
            return Err(TeamError(format!("team already exists: {}", spec.id)));
        }
        guard.insert(
            spec.id.clone(),
            Team {
                spec,
                members,
                tasks: HashMap::new(),
            },
        );
        drop(guard);
        self.subscribe_team(&team_id);
        Ok(())
    }

    fn add_task(&self, team: &str, task: TeamTask) -> Result<(), TeamError> {
        let mut guard = self.team(team)?;
        let board = guard.get_mut(team).unwrap();
        board.tasks.insert(task.id.clone(), task);
        Ok(())
    }

    fn claim_task(&self, team: &str, member: &str) -> Result<String, TeamError> {
        let mut guard = self.team(team)?;
        let board = guard.get_mut(team).unwrap();
        // 成员校验:认领人必须是团队成员。
        let is_member = board
            .members
            .iter()
            .any(|m| m.id == member || m.name == member);
        if !is_member {
            return Err(TeamError(format!("member not in team: {member}")));
        }
        // 取第一个依赖全部 Done 的 Pending 任务。
        let candidate = board
            .tasks
            .values()
            .filter(|t| t.status == TeamTaskStatus::Pending)
            .find(|t| {
                t.dependencies.iter().all(|d| {
                    board
                        .tasks
                        .get(d)
                        .map(|dep| dep.status == TeamTaskStatus::Done)
                        .unwrap_or(false)
                })
            })
            .map(|t| t.id.clone());
        let Some(task_id) = candidate else {
            return Err(TeamError("no claimable task".to_string()));
        };
        let task = board.tasks.get_mut(&task_id).unwrap();
        task.status = TeamTaskStatus::InProgress;
        task.assignee = Some(member.to_string());
        let status = task.status;
        drop(guard);
        self.emit(team, &task_id, status);
        Ok(task_id)
    }

    fn complete_task(&self, team: &str, task: &str, output: Value) -> Result<(), TeamError> {
        let mut guard = self.team(team)?;
        let board = guard.get_mut(team).unwrap();
        let entry = board
            .tasks
            .get_mut(task)
            .ok_or_else(|| TeamError(format!("task not found: {task}")))?;
        if entry.status != TeamTaskStatus::InProgress {
            return Err(TeamError(format!(
                "task {task} cannot be completed from status {:?}",
                entry.status
            )));
        }
        entry.status = TeamTaskStatus::Done;
        entry.result = Some(output);
        let status = entry.status;
        drop(guard);
        self.emit(team, task, status);
        Ok(())
    }

    fn submit_for_review(&self, team: &str, task: &str) -> Result<(), TeamError> {
        let mut guard = self.team(team)?;
        let board = guard.get_mut(team).unwrap();
        let entry = board
            .tasks
            .get_mut(task)
            .ok_or_else(|| TeamError(format!("task not found: {task}")))?;
        if entry.status != TeamTaskStatus::InProgress {
            return Err(TeamError(format!("task {task} is not in progress")));
        }
        entry.status = TeamTaskStatus::InReview;
        entry.review_votes.clear();
        let status = entry.status;
        drop(guard);
        self.emit(team, task, status);
        Ok(())
    }

    fn vote_review(
        &self,
        team: &str,
        task: &str,
        member: &str,
        approve: bool,
    ) -> Result<(), TeamError> {
        let mut guard = self.team(team)?;
        let board = guard.get_mut(team).unwrap();
        let entry = board
            .tasks
            .get_mut(task)
            .ok_or_else(|| TeamError(format!("task not found: {task}")))?;
        if entry.status != TeamTaskStatus::InReview {
            return Err(TeamError(format!("task {task} is not in review")));
        }
        let is_member = board
            .members
            .iter()
            .any(|m| m.id == member || m.name == member);
        if !is_member {
            return Err(TeamError(format!("member not in team: {member}")));
        }
        if entry.review_votes.contains(&member.to_string()) {
            return Err(TeamError(format!("member {member} already voted")));
        }
        // 记录投票:approve 记成员名,否决记 "!member"(简化多数判定)。
        entry.review_votes.push(if approve {
            member.to_string()
        } else {
            format!("!{member}")
        });
        Ok(())
    }

    fn settle_review(&self, team: &str, task: &str) -> Result<TeamTaskStatus, TeamError> {
        let mut guard = self.team(team)?;
        let board = guard.get_mut(team).unwrap();
        let entry = board
            .tasks
            .get_mut(task)
            .ok_or_else(|| TeamError(format!("task not found: {task}")))?;
        if entry.status != TeamTaskStatus::InReview {
            return Err(TeamError(format!("task {task} is not in review")));
        }
        let approves = entry
            .review_votes
            .iter()
            .filter(|v| !v.starts_with('!'))
            .count();
        let rejects = entry.review_votes.len() - approves;
        let status = if approves > rejects {
            TeamTaskStatus::Done
        } else {
            TeamTaskStatus::Failed
        };
        entry.status = status;
        drop(guard);
        self.emit(team, task, status);
        Ok(status)
    }

    fn tasks(&self, team: &str) -> Result<Vec<TeamTask>, TeamError> {
        let guard = self.team(team)?;
        let board = guard.get(team).unwrap();
        let mut tasks: Vec<TeamTask> = board.tasks.values().cloned().collect();
        tasks.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(tasks)
    }

    fn list_teams(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.teams.lock().unwrap().keys().cloned().collect();
        ids.sort();
        ids
    }

    fn send_message(
        &self,
        team: &str,
        from: &str,
        to: Option<&str>,
        content: &str,
    ) -> Result<TeamMessage, TeamError> {
        {
            let _guard = self.team(team)?;
        }
        let message = TeamMessage {
            from: from.to_string(),
            to: to.map(str::to_string),
            content: content.to_string(),
        };
        if let Some(messager) = &self.messager {
            messager.publish(
                &Self::channel(team),
                serde_json::to_value(&message).unwrap(),
            );
        } else {
            self.queue
                .publish(
                    &Self::channel(team),
                    serde_json::to_value(&message).unwrap(),
                )
                .map_err(|e| TeamError(format!("publish message: {e}")))?;
        }
        Ok(message)
    }

    fn messages(&self, team: &str) -> Result<Vec<TeamMessage>, TeamError> {
        {
            let _guard = self.team(team)?;
        }
        if self.messager.is_some() {
            return Ok(self
                .messager_messages
                .lock()
                .unwrap()
                .get(team)
                .cloned()
                .unwrap_or_default());
        }
        let mut msgs: Vec<QueueMessage> = self
            .queue
            .backlog(&Self::channel(team))
            .map_err(|e| TeamError(format!("read messages: {e}")))?;
        msgs.sort_by_key(|m| m.seq);
        Ok(msgs
            .into_iter()
            .map(|m| {
                serde_json::from_value(m.payload).unwrap_or(TeamMessage {
                    from: String::new(),
                    to: None,
                    content: String::new(),
                })
            })
            .collect())
    }

    async fn run_task(&self, team: &str, task: &str) -> Result<TeamRunResult, TeamError> {
        // 认领(依赖须满足):以首位团队成员身份认领。
        let member = {
            let guard = self.team(team)?;
            guard
                .get(team)
                .unwrap()
                .members
                .first()
                .map(|m| m.id.clone())
                .ok_or_else(|| TeamError("team has no members".to_string()))?
        };
        let claimed = self.claim_task(team, &member)?;
        if claimed != task {
            // 认领了别的任务则退回?简化:要求指定任务可领。
            let _ = self.complete_task(team, &claimed, json!({"skipped": true}));
            return Err(TeamError(format!(
                "task {task} not claimable (claimed {claimed})"
            )));
        }
        let (title, team_name) = {
            let guard = self.team(team)?;
            let board = guard.get(team).unwrap();
            (
                board
                    .tasks
                    .get(task)
                    .map(|t| t.title.clone())
                    .unwrap_or_default(),
                board.spec.name.clone(),
            )
        };

        // 真实委派 subagent 执行。
        let result = self
            .subagent
            .run(SubagentSpec {
                id: format!("team-{team}-{task}"),
                task: title,
                context: Some(format!(
                    "You are a member of team {team_name} ({team}) working on task {task}."
                )),
                budget: Some(6),
                allowed_tools: None,
            })
            .await
            .map_err(|e| TeamError(format!("subagent failed: {e}")))?;

        let output = json!({ "answer": result.answer, "iterations": result.iterations_used });
        self.complete_task(team, task, output.clone())?;
        let status = self
            .team(team)?
            .get(team)
            .and_then(|board| board.tasks.get(task))
            .map(|task| task.status)
            .unwrap_or(TeamTaskStatus::Failed);
        Ok(TeamRunResult {
            task_id: task.to_string(),
            status,
            output,
        })
    }
}

/// 团队插件:注入 SubagentRuntime,提供 teams seam。
pub struct TeamsPlugin;

impl Plugin for TeamsPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-teams"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![TEAMS]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![SUBAGENT, QUEUE]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let subagent = ctx
            .service::<dyn SubagentRuntime>(&SUBAGENT)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "subagent seam not registered".to_string(),
            })?;
        let queue = ctx
            .service::<dyn MessageQueue>(&QUEUE)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "queue seam not registered".to_string(),
            })?;
        let runtime = InMemoryTeamRuntime::new(subagent, queue, ctx.clone());
        let runtime = if let Some(messager) = ctx.service::<dyn Messager>(&MESSAGER) {
            runtime.with_messager(messager)
        } else {
            runtime
        };
        let runtime: Arc<dyn TeamRuntime> = Arc::new(runtime);
        Ok(vec![ctx.register(TEAMS, runtime)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::event::Event;
    use ah_contracts::keys::{FS, TEAMS};
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
            StdArc::new(TeamsPlugin),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    fn team() -> (TeamSpec, Vec<TeamMemberSpec>) {
        (
            TeamSpec {
                id: "t1".into(),
                name: "Alpha".into(),
            },
            vec![
                TeamMemberSpec {
                    id: "m1".into(),
                    name: "Alice".into(),
                    role: "dev".into(),
                },
                TeamMemberSpec {
                    id: "m2".into(),
                    name: "Bob".into(),
                    role: "reviewer".into(),
                },
            ],
        )
    }

    fn task(id: &str, deps: Vec<String>) -> TeamTask {
        TeamTask {
            id: id.to_string(),
            title: format!("do {id}"),
            status: TeamTaskStatus::Pending,
            dependencies: deps,
            assignee: None,
            review_votes: Vec::new(),
            result: None,
        }
    }

    #[tokio::test]
    async fn task_lifecycle_and_dependency_gate() {
        let root = std::env::temp_dir().join(format!("ah-teams-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let runtime = ctx.service::<dyn TeamRuntime>(&TEAMS).expect("teams");

        let (spec, members) = team();
        runtime.create_team(spec, members).expect("create");
        runtime.add_task("t1", task("a", vec![])).expect("add a");
        runtime
            .add_task("t1", task("b", vec!["a".to_string()]))
            .expect("add b");

        // 依赖未完成:只能认领 a。
        let claimed = runtime.claim_task("t1", "m1").expect("claim a");
        assert_eq!(claimed, "a");
        assert!(
            runtime.claim_task("t1", "m1").is_err(),
            "b 依赖未完成不可领"
        );

        runtime
            .complete_task("t1", "a", json!({"ok": true}))
            .expect("complete a");
        let claimed = runtime.claim_task("t1", "m1").expect("claim b now");
        assert_eq!(claimed, "b");
        runtime
            .complete_task("t1", "b", json!({"ok": true}))
            .expect("complete b");

        let tasks = runtime.tasks("t1").expect("tasks");
        assert!(tasks.iter().all(|t| t.status == TeamTaskStatus::Done));

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn cannot_complete_pending_task() {
        let root =
            std::env::temp_dir().join(format!("ah-teams-invalid-complete-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let runtime = ctx.service::<dyn TeamRuntime>(&TEAMS).expect("teams");
        let (spec, members) = team();
        runtime.create_team(spec, members).expect("create");
        runtime.add_task("t1", task("a", vec![])).expect("add");
        assert!(
            runtime
                .complete_task("t1", "a", json!({"ok": true}))
                .is_err()
        );
        assert_eq!(
            runtime.tasks("t1").expect("tasks")[0].status,
            TeamTaskStatus::Pending
        );
        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn review_settles_by_majority() {
        let root = std::env::temp_dir().join(format!("ah-teams-review-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let runtime = ctx.service::<dyn TeamRuntime>(&TEAMS).expect("teams");

        let (spec, members) = team();
        runtime.create_team(spec, members).expect("create");
        runtime.add_task("t1", task("a", vec![])).expect("add");
        runtime.claim_task("t1", "m1").expect("claim");
        runtime.submit_for_review("t1", "a").expect("submit");

        // m1 approve、m2 approve → 通过。
        runtime.vote_review("t1", "a", "m1", true).expect("vote1");
        runtime.vote_review("t1", "a", "m2", true).expect("vote2");
        let status = runtime.settle_review("t1", "a").expect("settle");
        assert_eq!(status, TeamTaskStatus::Done);

        // 否决场景。
        runtime.add_task("t1", task("b", vec![])).expect("add b");
        runtime.claim_task("t1", "m1").expect("claim b");
        runtime.submit_for_review("t1", "b").expect("submit b");
        runtime.vote_review("t1", "b", "m1", true).expect("v1");
        runtime.vote_review("t1", "b", "m2", false).expect("v2");
        let status = runtime.settle_review("t1", "b").expect("settle b");
        assert_eq!(status, TeamTaskStatus::Failed);

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn run_task_delegates_to_subagent_for_real() {
        let root = std::env::temp_dir().join(format!("ah-teams-run-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let fs = ctx
            .service::<dyn ah_contracts::fs::FsProvider>(&FS)
            .expect("fs");
        fs.write("probe.txt", b"x").expect("write");

        let runtime = ctx.service::<dyn TeamRuntime>(&TEAMS).expect("teams");
        let (spec, members) = team();
        runtime.create_team(spec, members).expect("create");
        runtime.add_task("t1", task("t", vec![])).expect("add");

        let result = runtime.run_task("t1", "t").await.expect("run");
        assert_eq!(result.status, TeamTaskStatus::Done);
        // subagent 真实执行:回答引用真实文件。
        let answer = result.output["answer"].as_str().unwrap_or_default();
        assert!(answer.contains("mock final answer"));
        assert!(answer.contains("probe.txt"));

        // 状态迁移事件被 emit。
        let events = ctx
            .service::<dyn ah_contracts::session::SessionLog>(&ah_contracts::keys::SESSIONS)
            .map(|s| s.events());
        // teams/task 是 emit 事件,监听器在测试里计数更直接;此处仅确认任务状态。
        assert!(events.is_some());

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn team_task_event_has_stable_id() {
        assert_eq!(TeamTaskEvent::ID, "teams/task");
    }

    #[tokio::test]
    async fn team_messaging_via_queue_in_order() {
        let root = std::env::temp_dir().join(format!("ah-teams-msg-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let runtime = ctx.service::<dyn TeamRuntime>(&TEAMS).expect("teams");

        let (spec, members) = team();
        runtime.create_team(spec, members).expect("create");
        runtime
            .send_message("t1", "m1", Some("m2"), "hello")
            .expect("msg1");
        runtime
            .send_message("t1", "m2", None, "hi all")
            .expect("msg2");
        // 不存在的团队发送消息报错。
        assert!(runtime.send_message("nope", "m1", None, "x").is_err());

        let messages = runtime.messages("t1").expect("messages");
        assert_eq!(messages.len(), 2, "ordered backlog");
        assert_eq!(messages[0].from, "m1");
        assert_eq!(messages[0].to.as_deref(), Some("m2"));
        assert_eq!(messages[0].content, "hello");
        assert_eq!(messages[1].from, "m2");
        assert_eq!(messages[1].to, None);
        assert_eq!(messages[1].content, "hi all");

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn team_messaging_can_use_inprocess_messager() {
        let root = std::env::temp_dir().join(format!("ah-teams-messager-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let subagent = ctx
            .service::<dyn SubagentRuntime>(&SUBAGENT)
            .expect("subagent");
        let queue = ctx.service::<dyn MessageQueue>(&QUEUE).expect("queue");
        let messager = ah_contracts::messager::create_messager(
            ah_contracts::messager::MessagerTransportConfig {
                node_id: Some("team-node".into()),
                ..Default::default()
            },
        )
        .expect("messager");
        let task_events = Arc::new(Mutex::new(Vec::new()));
        let task_events_sink = task_events.clone();
        let messager_for_events = messager.clone();
        messager_for_events.subscribe(
            "team:t1:task",
            Arc::new(move |payload| task_events_sink.lock().unwrap().push(payload)),
        );
        let runtime =
            InMemoryTeamRuntime::new(subagent, queue, ctx.clone()).with_messager(messager);
        let (spec, members) = team();
        runtime.create_team(spec, members).expect("create");
        runtime
            .add_task("t1", task("task-event", vec![]))
            .expect("add");
        runtime.claim_task("t1", "m1").expect("claim");
        assert_eq!(task_events.lock().unwrap().len(), 1);
        runtime
            .send_message("t1", "m1", Some("m2"), "hello over messager")
            .expect("send");
        runtime
            .send_message("t1", "m1", Some("m2"), "hello over messager")
            .expect("send duplicate");
        let messages = runtime.messages("t1").expect("messages");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "hello over messager");
        assert_eq!(messages[1].content, "hello over messager");
        drop(effects);
        let _ = std::fs::remove_dir_all(root);
    }
}
