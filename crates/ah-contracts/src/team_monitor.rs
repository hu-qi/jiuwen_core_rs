//! team_monitor seam:团队只读监控(对齐 Python agent_teams/monitor)。
//!
//! 观察团队的只读状态(信息/成员/任务/消息)与实时事件流:
//! - snapshot:从 TeamRuntime 查询,聚合为只读视图;
//! - 事件:订阅 teams/task 与 teams/swarm,按时间追加到监控日志(可读回)。

use crate::seam::Seam;
use crate::teams::{TeamMessage, TeamTask, TeamTaskStatus};

/// 团队成员只读视图。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MemberInfo {
    pub id: String,
    pub name: String,
    pub role: String,
}

/// 团队信息只读视图。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TeamInfo {
    pub id: String,
    pub name: String,
    pub members: Vec<MemberInfo>,
}

/// 任务只读视图。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TaskInfo {
    pub task: TeamTask,
    pub member: Option<String>,
}

/// 监控事件类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitorEventType {
    TaskStatusChanged,
    SwarmPhase,
    Message,
}

/// 一条监控事件(可回读)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MonitorEvent {
    pub seq: u64,
    pub ts_ms: u64,
    pub kind: MonitorEventType,
    /// 事件负载摘要(如 "task t1 -> Done")。
    pub summary: String,
}

/// team_monitor 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorError(pub String);

impl core::fmt::Display for MonitorError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MonitorError {}

/// team_monitor Seam(Service Definition):只读监控。
pub trait TeamMonitor: Seam {
    /// 团队信息 + 成员(只读)。
    fn team_info(&self, team: &str) -> Result<TeamInfo, MonitorError>;

    /// 任务视图(含认领人,只读)。
    fn tasks(&self, team: &str) -> Result<Vec<TaskInfo>, MonitorError>;

    /// 消息(只读,按发送顺序)。
    fn messages(&self, team: &str) -> Result<Vec<TeamMessage>, MonitorError>;

    /// 已记录监控事件(按 seq 升序)。
    fn events(&self) -> Vec<MonitorEvent>;
}

/// 从 TeamTaskStatus 的辅助显示。
pub fn status_label(status: TeamTaskStatus) -> &'static str {
    match status {
        TeamTaskStatus::Pending => "Pending",
        TeamTaskStatus::InProgress => "InProgress",
        TeamTaskStatus::InReview => "InReview",
        TeamTaskStatus::Done => "Done",
        TeamTaskStatus::Failed => "Failed",
    }
}
