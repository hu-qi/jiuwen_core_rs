//! team-status seam:团队成员/执行状态机与状态集合(对齐 openjiuwen/agent_teams/schema/status.py)。
//!
//! - `MemberStatus` 10 态 + 迁移表(UNSTARTED→STARTING CAS 守卫,STARTING→UNSTARTED 回滚,
//!   PAUSED→READY 续跑,STOPPED→READY 重拉,SHUTDOWN→RESTARTING 复活等);
//! - `ExecutionStatus` 10 态 + 迁移表(执行生命周期,取消/完成/失败/超时收敛回 IDLE);
//! - 状态集合:departed(已释放,SHUTDOWN_REQUESTED/SHUTDOWN)、unreachable(真正消失,SHUTDOWN)、
//!   settled(可休止:READY/PAUSED/STOPPED/SHUTDOWN)。
//!
//! 契约零实现:迁移表由插件提供(如 ah-plugins-team-status)。

use crate::seam::Seam;

/// 成员状态(对齐 MemberStatus)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberStatus {
    Unstarted,
    Starting,
    Ready,
    Busy,
    Paused,
    Stopped,
    Restarting,
    ShutdownRequested,
    Shutdown,
    Error,
}

/// 执行状态(对齐 ExecutionStatus)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Idle,
    Starting,
    Running,
    CancelRequested,
    Cancelling,
    Cancelled,
    Completing,
    Completed,
    Failed,
    TimedOut,
}

/// 团队状态错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamStatusError(pub String);

impl core::fmt::Display for TeamStatusError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TeamStatusError {}

/// 团队状态 Seam(Service Definition):状态机迁移判定与状态集合。
pub trait TeamStatus: Seam {
    /// 成员迁移是否合法(迁移表查找)。
    fn member_can_transition(&self, from: MemberStatus, to: MemberStatus) -> bool;

    /// 执行迁移是否合法。
    fn execution_can_transition(&self, from: ExecutionStatus, to: ExecutionStatus) -> bool;

    /// 已释放集合(leader 已放行,工作守卫解除)。
    fn member_departed(&self, status: MemberStatus) -> bool;

    /// 不可达集合(真正消失,消息投递停止)。
    fn member_unreachable(&self, status: MemberStatus) -> bool;

    /// 可休止集合(无活跃工作时可停,团队完成检查消费)。
    fn member_settled(&self, status: MemberStatus) -> bool;

    /// 给定状态的合法后继。
    fn allowed_member_transitions(&self, from: MemberStatus) -> Vec<MemberStatus>;
}
