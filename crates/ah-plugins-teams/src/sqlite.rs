//! SQLite 持久化团队运行时:与 InMemoryTeamRuntime 同一 TeamRuntime seam,
//! 状态与投票全部落在真实 SQLite 数据库(重启后完整恢复)。

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
use rusqlite::{Connection, OptionalExtension};
use serde_json::{Value, json};

fn status_to_str(status: TeamTaskStatus) -> String {
    serde_json::to_string(&status).unwrap_or_else(|_| "\"pending\"".to_string())
}

fn status_from_str(s: &str) -> Result<TeamTaskStatus, TeamError> {
    serde_json::from_str(s).map_err(|e| TeamError(format!("bad status {s}: {e}")))
}

pub struct SqliteTeamRuntime {
    conn: Mutex<Connection>,
    subagent: Arc<dyn SubagentRuntime>,
    /// 消息传输默认经 queue;注入 messager 后使用跨进程 topic。
    queue: Arc<dyn MessageQueue>,
    messager: Option<Arc<dyn Messager>>,
    messager_messages: Arc<Mutex<std::collections::HashMap<String, Vec<TeamMessage>>>>,
    ctx: Context,
}

impl SqliteTeamRuntime {
    fn channel(team: &str) -> String {
        format!("team:{team}:messages")
    }

    /// 打开(或创建)数据库并建表。
    pub fn open(
        path: &std::path::Path,
        subagent: Arc<dyn SubagentRuntime>,
        queue: Arc<dyn MessageQueue>,
        ctx: Context,
    ) -> Result<Self, TeamError> {
        let conn =
            Connection::open(path).map_err(|e| TeamError(format!("open sqlite failed: {e}")))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS teams (
                 id TEXT PRIMARY KEY, name TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS members (
                 team_id TEXT NOT NULL, member_id TEXT NOT NULL,
                 name TEXT NOT NULL, role TEXT NOT NULL,
                 PRIMARY KEY (team_id, member_id)
             );
             CREATE TABLE IF NOT EXISTS tasks (
                 team_id TEXT NOT NULL, task_id TEXT NOT NULL,
                 title TEXT NOT NULL, status TEXT NOT NULL,
                 assignee TEXT, result TEXT, deps TEXT NOT NULL,
                 review_votes TEXT NOT NULL,
                 PRIMARY KEY (team_id, task_id)
             );",
        )
        .map_err(|e| TeamError(format!("init schema failed: {e}")))?;
        Ok(Self {
            conn: Mutex::new(conn),
            subagent,
            queue,
            messager: None,
            messager_messages: Arc::new(Mutex::new(std::collections::HashMap::new())),
            ctx,
        })
    }

    pub fn with_messager(mut self, messager: Arc<dyn Messager>) -> Self {
        messager.start();
        self.messager = Some(messager);
        for team in self.list_teams() {
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

    /// 读取团队全部任务(按 task_id 排序)。
    fn load_tasks(&self, conn: &Connection, team: &str) -> Result<Vec<TeamTask>, TeamError> {
        let mut stmt = conn
            .prepare("SELECT task_id, title, status, assignee, result, deps, review_votes FROM tasks WHERE team_id = ?1 ORDER BY task_id")
            .map_err(|e| TeamError(format!("prepare tasks: {e}")))?;
        let rows = stmt
            .query_map([team], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                ))
            })
            .map_err(|e| TeamError(format!("query tasks: {e}")))?;
        let mut tasks = Vec::new();
        for row in rows {
            let (id, title, status, assignee, result, deps, votes) =
                row.map_err(|e| TeamError(format!("row: {e}")))?;
            tasks.push(TeamTask {
                id,
                title,
                status: status_from_str(&status)?,
                dependencies: serde_json::from_str(&deps)
                    .map_err(|e| TeamError(format!("deps json: {e}")))?,
                assignee,
                review_votes: serde_json::from_str(&votes)
                    .map_err(|e| TeamError(format!("votes json: {e}")))?,
                result: result
                    .map(|r| {
                        serde_json::from_str(&r).map_err(|e| TeamError(format!("result json: {e}")))
                    })
                    .transpose()?,
            });
        }
        Ok(tasks)
    }

    /// 成员校验(认领/投票均须是团队成员)。
    fn is_member(&self, conn: &Connection, team: &str, member: &str) -> Result<bool, TeamError> {
        let exists: Option<bool> = conn
            .query_row(
                "SELECT 1 FROM members WHERE team_id = ?1 AND (member_id = ?2 OR name = ?2) LIMIT 1",
                [team, member],
                |row| row.get::<_, bool>(0),
            )
            .optional()
            .map_err(|e| TeamError(format!("member check: {e}")))?;
        Ok(exists.unwrap_or(false))
    }
}

impl Seam for SqliteTeamRuntime {}

#[async_trait]
impl TeamRuntime for SqliteTeamRuntime {
    fn create_team(&self, spec: TeamSpec, members: Vec<TeamMemberSpec>) -> Result<(), TeamError> {
        if spec.id.is_empty() || spec.name.is_empty() {
            return Err(TeamError("team id/name must not be empty".to_string()));
        }
        let conn = self.conn.lock().unwrap();
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| TeamError(format!("begin tx: {e}")))?;
        tx.execute(
            "INSERT INTO teams (id, name) VALUES (?1, ?2)",
            [&spec.id, &spec.name],
        )
        .map_err(|e| TeamError(format!("insert team: {e}")))?;
        for member in &members {
            tx.execute(
                "INSERT INTO members (team_id, member_id, name, role) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![spec.id, member.id, member.name, member.role],
            )
            .map_err(|e| TeamError(format!("insert member: {e}")))?;
        }
        tx.commit()
            .map_err(|e| TeamError(format!("commit team: {e}")))?;
        self.subscribe_team(&spec.id);
        Ok(())
    }

    fn add_task(&self, team: &str, task: TeamTask) -> Result<(), TeamError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO tasks (team_id, task_id, title, status, assignee, result, deps, review_votes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                team,
                task.id,
                task.title,
                status_to_str(task.status),
                task.assignee,
                task.result.as_ref().map(|v| v.to_string()),
                serde_json::to_string(&task.dependencies).unwrap_or_else(|_| "[]".to_string()),
                serde_json::to_string(&task.review_votes).unwrap_or_else(|_| "[]".to_string()),
            ],
        )
        .map_err(|e| TeamError(format!("insert task: {e}")))?;
        Ok(())
    }

    fn claim_task(&self, team: &str, member: &str) -> Result<String, TeamError> {
        let conn = self.conn.lock().unwrap();
        if !self.is_member(&conn, team, member)? {
            return Err(TeamError(format!("member not in team: {member}")));
        }
        let tasks = self.load_tasks(&conn, team)?;
        let candidate = tasks
            .iter()
            .filter(|t| t.status == TeamTaskStatus::Pending)
            .find(|t| {
                t.dependencies.iter().all(|d| {
                    tasks
                        .iter()
                        .any(|x| x.id == *d && x.status == TeamTaskStatus::Done)
                })
            })
            .map(|t| t.id.clone());
        let Some(task_id) = candidate else {
            return Err(TeamError("no claimable task".to_string()));
        };
        conn.execute(
            "UPDATE tasks SET status = ?1, assignee = ?2 WHERE team_id = ?3 AND task_id = ?4",
            rusqlite::params![
                status_to_str(TeamTaskStatus::InProgress),
                member,
                team,
                task_id
            ],
        )
        .map_err(|e| TeamError(format!("claim update: {e}")))?;
        drop(conn);
        self.emit(team, &task_id, TeamTaskStatus::InProgress);
        Ok(task_id)
    }

    fn complete_task(&self, team: &str, task: &str, output: Value) -> Result<(), TeamError> {
        let conn = self.conn.lock().unwrap();
        let current = self
            .load_tasks(&conn, team)?
            .into_iter()
            .find(|entry| entry.id == task)
            .ok_or_else(|| TeamError(format!("task not found: {task}")))?;
        if current.status != TeamTaskStatus::InProgress {
            return Err(TeamError(format!(
                "task {task} cannot be completed from status {:?}",
                current.status
            )));
        }
        let updated = conn
            .execute(
                "UPDATE tasks SET status = ?1, result = ?2 WHERE team_id = ?3 AND task_id = ?4 AND status = ?5",
                rusqlite::params![
                    status_to_str(TeamTaskStatus::Done),
                    output.to_string(),
                    team,
                    task,
                    status_to_str(TeamTaskStatus::InProgress),
                ],
            )
            .map_err(|e| TeamError(format!("complete update: {e}")))?;
        if updated == 0 {
            return Err(TeamError(format!(
                "task {task} was changed before completion"
            )));
        }
        drop(conn);
        self.emit(team, task, TeamTaskStatus::Done);
        Ok(())
    }

    fn submit_for_review(&self, team: &str, task: &str) -> Result<(), TeamError> {
        let conn = self.conn.lock().unwrap();
        let current = self
            .load_tasks(&conn, team)?
            .into_iter()
            .find(|t| t.id == task)
            .ok_or_else(|| TeamError(format!("task not found: {task}")))?;
        if current.status != TeamTaskStatus::InProgress {
            return Err(TeamError(format!("task {task} is not in progress")));
        }
        conn.execute(
            "UPDATE tasks SET status = ?1, review_votes = ?2 WHERE team_id = ?3 AND task_id = ?4",
            rusqlite::params![status_to_str(TeamTaskStatus::InReview), "[]", team, task],
        )
        .map_err(|e| TeamError(format!("submit update: {e}")))?;
        drop(conn);
        self.emit(team, task, TeamTaskStatus::InReview);
        Ok(())
    }

    fn vote_review(
        &self,
        team: &str,
        task: &str,
        member: &str,
        approve: bool,
    ) -> Result<(), TeamError> {
        let conn = self.conn.lock().unwrap();
        if !self.is_member(&conn, team, member)? {
            return Err(TeamError(format!("member not in team: {member}")));
        }
        let current = self
            .load_tasks(&conn, team)?
            .into_iter()
            .find(|t| t.id == task)
            .ok_or_else(|| TeamError(format!("task not found: {task}")))?;
        if current.status != TeamTaskStatus::InReview {
            return Err(TeamError(format!("task {task} is not in review")));
        }
        if current.review_votes.contains(&member.to_string()) {
            return Err(TeamError(format!("member {member} already voted")));
        }
        let mut votes = current.review_votes;
        votes.push(if approve {
            member.to_string()
        } else {
            format!("!{member}")
        });
        conn.execute(
            "UPDATE tasks SET review_votes = ?1 WHERE team_id = ?2 AND task_id = ?3",
            rusqlite::params![
                serde_json::to_string(&votes).unwrap_or_else(|_| "[]".to_string()),
                team,
                task
            ],
        )
        .map_err(|e| TeamError(format!("vote update: {e}")))?;
        Ok(())
    }

    fn settle_review(&self, team: &str, task: &str) -> Result<TeamTaskStatus, TeamError> {
        let conn = self.conn.lock().unwrap();
        let current = self
            .load_tasks(&conn, team)?
            .into_iter()
            .find(|t| t.id == task)
            .ok_or_else(|| TeamError(format!("task not found: {task}")))?;
        if current.status != TeamTaskStatus::InReview {
            return Err(TeamError(format!("task {task} is not in review")));
        }
        let approves = current
            .review_votes
            .iter()
            .filter(|v| !v.starts_with('!'))
            .count();
        let rejects = current.review_votes.len() - approves;
        let status = if approves > rejects {
            TeamTaskStatus::Done
        } else {
            TeamTaskStatus::Failed
        };
        conn.execute(
            "UPDATE tasks SET status = ?1 WHERE team_id = ?2 AND task_id = ?3",
            rusqlite::params![status_to_str(status), team, task],
        )
        .map_err(|e| TeamError(format!("settle update: {e}")))?;
        drop(conn);
        self.emit(team, task, status);
        Ok(status)
    }

    fn tasks(&self, team: &str) -> Result<Vec<TeamTask>, TeamError> {
        let conn = self.conn.lock().unwrap();
        self.load_tasks(&conn, team)
    }

    fn list_teams(&self) -> Vec<String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id FROM teams ORDER BY id")
            .expect("prepare teams");
        let mut ids = Vec::new();
        match stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            Ok(id)
        }) {
            Ok(rows) => {
                for row in rows {
                    match row {
                        Ok(id) => ids.push(id),
                        Err(e) => eprintln!("list_teams row error: {e}"),
                    }
                }
            }
            Err(e) => eprintln!("list_teams query error: {e}"),
        }
        ids
    }

    fn send_message(
        &self,
        team: &str,
        from: &str,
        to: Option<&str>,
        content: &str,
    ) -> Result<TeamMessage, TeamError> {
        let conn = self.conn.lock().unwrap();
        let exists: Option<bool> = conn
            .query_row("SELECT 1 FROM teams WHERE id = ?1 LIMIT 1", [team], |row| {
                row.get::<_, bool>(0)
            })
            .optional()
            .map_err(|e| TeamError(format!("team check: {e}")))?;
        if !exists.unwrap_or(false) {
            return Err(TeamError(format!("team not found: {team}")));
        }
        drop(conn);
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
        // 以首位团队成员身份认领(依赖须满足)。
        let member = {
            let conn = self.conn.lock().unwrap();
            let name: Option<String> = conn
                .query_row(
                    "SELECT member_id FROM members WHERE team_id = ?1 ORDER BY member_id LIMIT 1",
                    [team],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| TeamError(format!("first member: {e}")))?;
            name.ok_or_else(|| TeamError("team has no members".to_string()))?
        };
        let claimed = self.claim_task(team, &member)?;
        if claimed != task {
            let _ = self.complete_task(team, &claimed, json!({ "skipped": true }));
            return Err(TeamError(format!(
                "task {task} not claimable (claimed {claimed})"
            )));
        }
        let title = {
            let conn = self.conn.lock().unwrap();
            self.load_tasks(&conn, team)?
                .into_iter()
                .find(|t| t.id == task)
                .map(|t| t.title)
                .unwrap_or_default()
        };
        let team_name = {
            let conn = self.conn.lock().unwrap();
            conn.query_row("SELECT name FROM teams WHERE id = ?1", [team], |row| {
                row.get::<_, String>(0)
            })
            .optional()
            .map_err(|e| TeamError(format!("team name: {e}")))?
            .unwrap_or_default()
        };

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
        let status = {
            let conn = self.conn.lock().unwrap();
            self.load_tasks(&conn, team)?
                .into_iter()
                .find(|t| t.id == task)
                .map(|t| t.status)
                .unwrap_or(TeamTaskStatus::Failed)
        };
        Ok(TeamRunResult {
            task_id: task.to_string(),
            status,
            output,
        })
    }
}

/// SQLite 持久化团队插件:注入 SubagentRuntime,提供 teams seam(持久化版)。
pub struct SqliteTeamsPlugin {
    db_path: std::path::PathBuf,
}

impl SqliteTeamsPlugin {
    /// 以数据库文件路径创建插件。
    pub fn new(db_path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            db_path: db_path.into(),
        }
    }
}

impl Plugin for SqliteTeamsPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-teams-sqlite"
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
        let runtime = SqliteTeamRuntime::open(&self.db_path, subagent, queue, ctx.clone())
            .map_err(|e| PluginError::Apply {
                plugin: self.name(),
                message: e.0,
            })?;
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
    use ah_contracts::keys::SUBAGENT;
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
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    fn subagent(ctx: &Context) -> StdArc<dyn SubagentRuntime> {
        ctx.service::<dyn SubagentRuntime>(&SUBAGENT)
            .expect("subagent")
    }

    fn queue(ctx: &Context) -> StdArc<dyn MessageQueue> {
        ctx.service::<dyn MessageQueue>(&ah_contracts::keys::QUEUE)
            .expect("queue")
    }

    fn members() -> Vec<TeamMemberSpec> {
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
        ]
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
    async fn sqlite_lifecycle_persists_across_reopen() {
        let root = std::env::temp_dir().join(format!("ah-sqlite-life-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let db = root.join("teams.db");
        let rt =
            SqliteTeamRuntime::open(&db, subagent(&ctx), queue(&ctx), ctx.clone()).expect("open");

        let spec = TeamSpec {
            id: "t1".into(),
            name: "Alpha".into(),
        };
        rt.create_team(spec, members()).expect("create");
        rt.add_task("t1", task("a", vec![])).expect("add");
        assert!(rt.complete_task("t1", "a", json!({"ok": true})).is_err());
        let claimed = rt.claim_task("t1", "m1").expect("claim");
        assert_eq!(claimed, "a");
        rt.complete_task("t1", "a", json!({"ok": true}))
            .expect("complete");
        assert_eq!(
            rt.tasks("t1").expect("tasks")[0].status,
            TeamTaskStatus::Done
        );
        drop(rt);

        // 重启:同一数据库文件完整恢复状态。
        let rt2 =
            SqliteTeamRuntime::open(&db, subagent(&ctx), queue(&ctx), ctx.clone()).expect("reopen");
        let tasks = rt2.tasks("t1").expect("tasks");
        assert_eq!(tasks[0].status, TeamTaskStatus::Done, "status persisted");
        assert_eq!(tasks[0].assignee.as_deref(), Some("m1"));
        assert_eq!(tasks[0].result.as_ref().unwrap()["ok"], true);
        assert_eq!(rt2.list_teams(), vec!["t1".to_string()]);

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn sqlite_review_settles_and_persists() {
        let root = std::env::temp_dir().join(format!("ah-sqlite-rev-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let db = root.join("teams.db");
        let rt =
            SqliteTeamRuntime::open(&db, subagent(&ctx), queue(&ctx), ctx.clone()).expect("open");

        rt.create_team(
            TeamSpec {
                id: "t1".into(),
                name: "Alpha".into(),
            },
            members(),
        )
        .expect("create");
        rt.add_task("t1", task("a", vec![])).expect("add");
        rt.claim_task("t1", "m1").expect("claim");
        rt.submit_for_review("t1", "a").expect("submit");
        rt.vote_review("t1", "a", "m1", true).expect("vote1");
        rt.vote_review("t1", "a", "m2", false).expect("vote2");
        let status = rt.settle_review("t1", "a").expect("settle");
        assert_eq!(
            status,
            TeamTaskStatus::Failed,
            "1 approve vs 1 reject -> failed"
        );
        drop(rt);

        // 重启:review 结论持久化。
        let rt2 =
            SqliteTeamRuntime::open(&db, subagent(&ctx), queue(&ctx), ctx.clone()).expect("reopen");
        assert_eq!(
            rt2.tasks("t1").expect("tasks")[0].status,
            TeamTaskStatus::Failed
        );

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn sqlite_dependency_gate() {
        let root = std::env::temp_dir().join(format!("ah-sqlite-dep-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let db = root.join("teams.db");
        let rt =
            SqliteTeamRuntime::open(&db, subagent(&ctx), queue(&ctx), ctx.clone()).expect("open");

        rt.create_team(
            TeamSpec {
                id: "t1".into(),
                name: "Alpha".into(),
            },
            members(),
        )
        .expect("create");
        rt.add_task("t1", task("a", vec![])).expect("add a");
        rt.add_task("t1", task("b", vec!["a".to_string()]))
            .expect("add b");
        assert_eq!(rt.claim_task("t1", "m1").expect("claim a"), "a");
        assert!(rt.claim_task("t1", "m1").is_err(), "b blocked by unmet dep");
        rt.complete_task("t1", "a", json!({"ok": true}))
            .expect("complete a");
        assert_eq!(rt.claim_task("t1", "m1").expect("claim b now"), "b");

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn sqlite_run_task_delegates_for_real() {
        let root = std::env::temp_dir().join(format!("ah-sqlite-run-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let db = root.join("teams.db");
        let rt =
            SqliteTeamRuntime::open(&db, subagent(&ctx), queue(&ctx), ctx.clone()).expect("open");

        rt.create_team(
            TeamSpec {
                id: "t1".into(),
                name: "Alpha".into(),
            },
            members(),
        )
        .expect("create");
        rt.add_task("t1", task("t", vec![])).expect("add");
        let result = rt.run_task("t1", "t").await.expect("run");
        assert_eq!(result.status, TeamTaskStatus::Done);
        assert!(
            result.output["answer"]
                .as_str()
                .unwrap_or_default()
                .contains("mock final answer")
        );

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn sqlite_messaging_persists_via_queue() {
        let root = std::env::temp_dir().join(format!("ah-sqlite-msg-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let db = root.join("teams.db");
        let rt =
            SqliteTeamRuntime::open(&db, subagent(&ctx), queue(&ctx), ctx.clone()).expect("open");

        rt.create_team(
            TeamSpec {
                id: "t1".into(),
                name: "Alpha".into(),
            },
            members(),
        )
        .expect("create");
        rt.send_message("t1", "m1", Some("m2"), "persist me")
            .expect("msg");

        // 不存在的团队报错。
        assert!(rt.send_message("nope", "m1", None, "x").is_err());
        drop(rt);

        // 重启:消息经 queue 文件持久化,仍可读取。
        let rt2 =
            SqliteTeamRuntime::open(&db, subagent(&ctx), queue(&ctx), ctx.clone()).expect("reopen");
        let messages = rt2.messages("t1").expect("messages");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "persist me");

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn sqlite_messaging_can_use_inprocess_messager() {
        let root = std::env::temp_dir().join(format!("ah-sqlite-messager-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let db = root.join("teams.db");
        let messager = ah_contracts::messager::create_messager(
            ah_contracts::messager::MessagerTransportConfig {
                node_id: Some("sqlite-node".into()),
                ..Default::default()
            },
        )
        .expect("messager");
        let rt = SqliteTeamRuntime::open(&db, subagent(&ctx), queue(&ctx), ctx.clone())
            .expect("open")
            .with_messager(messager);
        rt.create_team(
            TeamSpec {
                id: "t1".into(),
                name: "Alpha".into(),
            },
            members(),
        )
        .expect("create");
        rt.send_message("t1", "m1", None, "over messager")
            .expect("send");
        rt.send_message("t1", "m1", None, "over messager")
            .expect("send duplicate");
        let messages = rt.messages("t1").expect("messages");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "over messager");
        assert_eq!(messages[1].content, "over messager");
        drop(effects);
        let _ = std::fs::remove_dir_all(root);
    }
}
