//! team-dispatch seam:团队运行派发决策(对齐 openjiuwen/agent_teams/runtime/dispatch.py)。
//!
//! 纯函数 7 路 truth table:根据 (team_in_db, team_in_session, pool_entry, team_db_state)
//! 决定 run 路径 — CREATE / NEW_TEAM_IN_SESSION / COLD_RECOVER / RESUME_FROM_PAUSE /
//! REJECT_RUNNING / REJECT_ORPHANED / REJECT_INCONSISTENT。无副作用;副作用由调用方执行。
//!
//! 前置契约:pool_entry 必须属于 target_session_id(跨 session entry 由 activate 先 stop+remove),
//! 违反即不变量错误(显式 Err)。

use crate::seam::Seam;

/// 运行派发结果种类(对齐 RunActionKind)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunActionKind {
    Create,
    NewTeamInSession,
    ColdRecover,
    ResumeFromPause,
    RejectRunning,
    RejectOrphaned,
    RejectInconsistent,
}

/// 对象池运行时状态(对齐 RuntimeState)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeState {
    Running,
    Paused,
}

/// 池条目(对齐 ActiveTeam 的派发所需字段)。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PoolEntry {
    pub current_session_id: String,
    pub state: RuntimeState,
}

/// 派发结果(对齐 RunAction)。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RunAction {
    pub kind: RunActionKind,
    pub require_spec: bool,
    pub reason: Option<String>,
}

impl RunAction {
    pub fn new(kind: RunActionKind, require_spec: bool, reason: Option<String>) -> Self {
        Self {
            kind,
            require_spec,
            reason,
        }
    }
}

/// DB 生命周期状态常量(对齐 TEAM_DB_STATE_*)。
pub const TEAM_DB_STATE_PENDING_CREATE: &str = "pending_create";
pub const TEAM_DB_STATE_CREATED: &str = "created";
pub const TEAM_DB_STATE_CLEANED: &str = "cleaned";

/// 派发错误(含不变量违例)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchError(pub String);

impl core::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DispatchError {}

/// 团队运行派发 Seam(Service Definition):纯函数决策。
pub trait TeamDispatch: Seam {
    /// 根据 (team_in_db, team_in_session, pool_entry, team_db_state) 决定 run 路径。
    fn decide(
        &self,
        team_in_db: bool,
        team_in_session: bool,
        pool_entry: Option<&PoolEntry>,
        target_session_id: &str,
        target_team_name: &str,
        team_db_state: Option<&str>,
    ) -> Result<RunAction, DispatchError>;
}
