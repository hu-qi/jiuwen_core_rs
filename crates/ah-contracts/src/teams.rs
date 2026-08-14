//! teams seam:多 agent 团队任务协作(任务板 + 依赖 + review + settle)。

use async_trait::async_trait;
use serde_json::Value;

use crate::seam::Seam;

/// 团队规格。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TeamSpec {
    pub id: String,
    pub name: String,
}

/// 团队成员规格。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TeamMemberSpec {
    pub id: String,
    pub name: String,
    pub role: String,
}

/// 任务状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamTaskStatus {
    Pending,
    InProgress,
    InReview,
    Done,
    Failed,
}

/// 团队任务。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TeamTask {
    pub id: String,
    pub title: String,
    pub status: TeamTaskStatus,
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub assignee: Option<String>,
    #[serde(default)]
    pub review_votes: Vec<String>,
    pub result: Option<Value>,
}

/// 团队成员间消息。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TeamMessage {
    pub from: String,
    pub to: Option<String>,
    pub content: String,
}

/// 任务执行结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TeamRunResult {
    pub task_id: String,
    pub status: TeamTaskStatus,
    pub output: Value,
}

/// teams/task 事件(状态迁移时 emit)。
#[derive(Clone, Debug)]
pub struct TeamTaskEvent {
    pub team: String,
    pub task_id: String,
    pub status: TeamTaskStatus,
}

impl crate::event::Event for TeamTaskEvent {
    const ID: &'static str = "teams/task";
}

/// 团队错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamError(pub String);

impl core::fmt::Display for TeamError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TeamError {}

/// teams Seam(Service Definition):任务板与执行。
#[async_trait]
pub trait TeamRuntime: Seam {
    fn create_team(&self, spec: TeamSpec, members: Vec<TeamMemberSpec>) -> Result<(), TeamError>;
    fn add_task(&self, team: &str, task: TeamTask) -> Result<(), TeamError>;
    /// 认领下一个可领任务(依赖须全部 Done);返回任务 id。
    fn claim_task(&self, team: &str, member: &str) -> Result<String, TeamError>;
    fn complete_task(&self, team: &str, task: &str, output: Value) -> Result<(), TeamError>;
    fn submit_for_review(&self, team: &str, task: &str) -> Result<(), TeamError>;
    fn vote_review(
        &self,
        team: &str,
        task: &str,
        member: &str,
        approve: bool,
    ) -> Result<(), TeamError>;
    /// 结算 review:过半数通过→Done,否则 Failed。
    fn settle_review(&self, team: &str, task: &str) -> Result<TeamTaskStatus, TeamError>;
    fn tasks(&self, team: &str) -> Result<Vec<TeamTask>, TeamError>;
    fn list_teams(&self) -> Vec<String>;
    /// 认领任务并经 SubagentRuntime 真实执行,输出写入任务板。
    async fn run_task(&self, team: &str, task: &str) -> Result<TeamRunResult, TeamError>;
    /// 向团队发送消息(经 queue seam 传输,channel 为 team:{team}:messages)。
    fn send_message(
        &self,
        team: &str,
        from: &str,
        to: Option<&str>,
        content: &str,
    ) -> Result<TeamMessage, TeamError>;
    /// 读取团队消息(按发送顺序)。
    fn messages(&self, team: &str) -> Result<Vec<TeamMessage>, TeamError>;
}
