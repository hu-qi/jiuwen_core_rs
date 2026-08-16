//! # ah-plugins-team-status
//!
//! Real team member/execution status state machines (aligned with
//! openjiuwen/agent_teams/schema/status.py):
//! - MemberStatus 10-state transition table (CAS-guarded UNSTARTED->STARTING,
//!   STARTING->UNSTARTED rollback, PAUSED->READY resume, STOPPED->READY respawn,
//!   SHUTDOWN->RESTARTING revival);
//! - ExecutionStatus 10-state table (cancel/completing/failed/timeout converge to IDLE);
//! - status sets: departed / unreachable / settled.

use std::sync::Arc;

use ah_contracts::keys::TEAM_STATUS;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::team_status::{ExecutionStatus, MemberStatus, TeamStatus};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

use MemberStatus::*;

/// Member transition table (aligned with MEMBER_TRANSITIONS).
pub const MEMBER_TRANSITIONS: &[(MemberStatus, &[MemberStatus])] = &[
    (Unstarted, &[Starting, Ready, Shutdown, Error]),
    (Starting, &[Ready, Unstarted, Shutdown, Error]),
    (
        Ready,
        &[
            Ready,
            Busy,
            Paused,
            Stopped,
            ShutdownRequested,
            Shutdown,
            Error,
        ],
    ),
    (Busy, &[Ready, Paused, Stopped, ShutdownRequested, Error]),
    (
        Paused,
        &[
            Ready,
            Restarting,
            Stopped,
            ShutdownRequested,
            Shutdown,
            Error,
        ],
    ),
    (
        Stopped,
        &[Ready, Restarting, ShutdownRequested, Shutdown, Error],
    ),
    (Restarting, &[Ready, Stopped, Error, Shutdown]),
    (ShutdownRequested, &[Shutdown, Error]),
    (Shutdown, &[Restarting]),
    (
        Error,
        &[Restarting, Ready, Stopped, ShutdownRequested, Shutdown],
    ),
];

/// Execution transition table (aligned with EXECUTION_TRANSITIONS).
pub const EXECUTION_TRANSITIONS: &[(ExecutionStatus, &[ExecutionStatus])] = &[
    (ExecutionStatus::Idle, &[ExecutionStatus::Starting]),
    (
        ExecutionStatus::Starting,
        &[
            ExecutionStatus::Running,
            ExecutionStatus::CancelRequested,
            ExecutionStatus::Cancelling,
            ExecutionStatus::Failed,
            ExecutionStatus::TimedOut,
        ],
    ),
    (
        ExecutionStatus::Running,
        &[
            ExecutionStatus::CancelRequested,
            ExecutionStatus::Cancelling,
            ExecutionStatus::Completing,
            ExecutionStatus::Failed,
            ExecutionStatus::TimedOut,
        ],
    ),
    (
        ExecutionStatus::CancelRequested,
        &[
            ExecutionStatus::Cancelling,
            ExecutionStatus::Cancelled,
            ExecutionStatus::Failed,
            ExecutionStatus::TimedOut,
        ],
    ),
    (
        ExecutionStatus::Cancelling,
        &[
            ExecutionStatus::Cancelled,
            ExecutionStatus::Failed,
            ExecutionStatus::TimedOut,
        ],
    ),
    (ExecutionStatus::Cancelled, &[ExecutionStatus::Idle]),
    (
        ExecutionStatus::Completing,
        &[
            ExecutionStatus::Completed,
            ExecutionStatus::Failed,
            ExecutionStatus::TimedOut,
        ],
    ),
    (ExecutionStatus::Completed, &[ExecutionStatus::Idle]),
    (ExecutionStatus::Failed, &[ExecutionStatus::Idle]),
    (ExecutionStatus::TimedOut, &[ExecutionStatus::Idle]),
];

/// Real status machine implementation.
pub struct StatusMachine;

impl Seam for StatusMachine {}

impl TeamStatus for StatusMachine {
    fn member_can_transition(&self, from: MemberStatus, to: MemberStatus) -> bool {
        MEMBER_TRANSITIONS
            .iter()
            .find(|(s, _)| *s == from)
            .map(|(_, targets)| targets.contains(&to))
            .unwrap_or(false)
    }

    fn execution_can_transition(&self, from: ExecutionStatus, to: ExecutionStatus) -> bool {
        EXECUTION_TRANSITIONS
            .iter()
            .find(|(s, _)| *s == from)
            .map(|(_, targets)| targets.contains(&to))
            .unwrap_or(false)
    }

    fn member_departed(&self, status: MemberStatus) -> bool {
        matches!(status, ShutdownRequested | Shutdown)
    }

    fn member_unreachable(&self, status: MemberStatus) -> bool {
        matches!(status, Shutdown)
    }

    fn member_settled(&self, status: MemberStatus) -> bool {
        matches!(status, Ready | Paused | Stopped | Shutdown)
    }

    fn allowed_member_transitions(&self, from: MemberStatus) -> Vec<MemberStatus> {
        MEMBER_TRANSITIONS
            .iter()
            .find(|(s, _)| *s == from)
            .map(|(_, targets)| targets.to_vec())
            .unwrap_or_default()
    }
}

/// team-status plugin: registers the `team-status` seam.
pub struct TeamStatusPlugin;

impl Plugin for TeamStatusPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-team-status"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![TEAM_STATUS]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let status: Arc<dyn TeamStatus> = Arc::new(StatusMachine);
        Ok(vec![ctx.register(TEAM_STATUS, status)])
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::TEAM_STATUS;
    use ah_contracts::team_status::ExecutionStatus;
    use ah_hub::plugin::DynPlugin;

    #[test]
    fn member_cas_guard_and_rollback() {
        let machine = StatusMachine;
        // CAS 守卫:UNSTARTED → STARTING 是唯一启动路径。
        assert!(machine.member_can_transition(Unstarted, Starting));
        assert!(
            !machine.member_can_transition(Unstarted, Busy),
            "未启动不能直接忙"
        );
        assert!(!machine.member_can_transition(Unstarted, Paused));
        // spawn 失败回滚 STARTING → UNSTARTED。
        assert!(machine.member_can_transition(Starting, Unstarted));
        // 就绪 → 忙。
        assert!(machine.member_can_transition(Ready, Busy));
        assert!(machine.member_can_transition(Busy, Ready));
    }

    #[test]
    fn member_pause_resume_stop_and_shutdown() {
        let machine = StatusMachine;
        // 自然轮末 → PAUSED;续跑回 READY。
        assert!(machine.member_can_transition(Ready, Paused));
        assert!(machine.member_can_transition(Paused, Ready));
        // 外部停 → STOPPED;recover 重拉回 READY。
        assert!(machine.member_can_transition(Ready, Stopped));
        assert!(machine.member_can_transition(Stopped, Ready));
        // 请求关停 → SHUTDOWN;不可回 RUNNING 系。
        assert!(machine.member_can_transition(Ready, ShutdownRequested));
        assert!(machine.member_can_transition(ShutdownRequested, Shutdown));
        assert!(
            !machine.member_can_transition(ShutdownRequested, Ready),
            "关停请求后不可回活"
        );
        // SHUTDOWN → RESTARTING 复活。
        assert!(machine.member_can_transition(Shutdown, Restarting));
        assert!(machine.member_can_transition(Restarting, Ready));
    }

    #[test]
    fn member_transition_table_is_exhaustive() {
        let machine = StatusMachine;
        // 每个状态至少有一条合法迁移(除自身外)。检查总表行数。
        assert_eq!(MEMBER_TRANSITIONS.len(), 10, "10 个成员状态");
        for (from, targets) in MEMBER_TRANSITIONS {
            assert!(!targets.is_empty(), "{from:?} 有后继");
            for to in *targets {
                assert!(
                    machine.member_can_transition(*from, *to),
                    "{from:?}→{to:?} 应合法"
                );
            }
        }
        // allowed_member_transitions 与表一致。
        assert_eq!(
            machine.allowed_member_transitions(Ready),
            vec![
                Ready,
                Busy,
                Paused,
                Stopped,
                ShutdownRequested,
                Shutdown,
                Error
            ]
        );
        assert!(
            machine
                .allowed_member_transitions(ShutdownRequested)
                .contains(&Shutdown)
        );
    }

    #[test]
    fn member_status_sets() {
        let machine = StatusMachine;
        // departed:关停请求/已关停(工作守卫解除)。
        assert!(machine.member_departed(ShutdownRequested));
        assert!(machine.member_departed(Shutdown));
        assert!(!machine.member_departed(Ready));
        assert!(!machine.member_departed(Paused), "PAUSED 仍属团队");
        assert!(!machine.member_departed(Stopped));
        assert!(!machine.member_departed(Error));
        assert!(!machine.member_departed(Restarting));
        // unreachable:仅 SHUTDOWN(消息投递停止)。
        assert!(machine.member_unreachable(Shutdown));
        assert!(
            !machine.member_unreachable(ShutdownRequested),
            "仅在途成员仍可投递"
        );
        assert!(!machine.member_unreachable(Stopped));
        // settled:可休止。
        for s in [Ready, Paused, Stopped, Shutdown] {
            assert!(machine.member_settled(s), "{s:?} 可休止");
        }
        assert!(!machine.member_settled(Busy));
        assert!(!machine.member_settled(Starting));
        assert!(!machine.member_settled(Error));
    }

    #[test]
    fn execution_lifecycle() {
        let machine = StatusMachine;
        use ExecutionStatus as E;
        assert!(machine.execution_can_transition(E::Idle, E::Starting));
        assert!(machine.execution_can_transition(E::Starting, E::Running));
        assert!(machine.execution_can_transition(E::Running, E::Completing));
        assert!(machine.execution_can_transition(E::Completing, E::Completed));
        assert!(machine.execution_can_transition(E::Completed, E::Idle));
        // 取消路径。
        assert!(machine.execution_can_transition(E::Running, E::CancelRequested));
        assert!(machine.execution_can_transition(E::CancelRequested, E::Cancelling));
        assert!(machine.execution_can_transition(E::Cancelling, E::Cancelled));
        assert!(machine.execution_can_transition(E::Cancelled, E::Idle));
        // 失败/超时。
        assert!(machine.execution_can_transition(E::Running, E::Failed));
        assert!(machine.execution_can_transition(E::Running, E::TimedOut));
        assert!(machine.execution_can_transition(E::TimedOut, E::Idle));
        // 非法迁移。
        assert!(
            !machine.execution_can_transition(E::Idle, E::Completed),
            "IDLE 不能直接完成"
        );
        assert!(
            !machine.execution_can_transition(E::Cancelled, E::Running),
            "已取消不能重跑"
        );
        assert!(!machine.execution_can_transition(E::Completed, E::Completed));
    }

    #[test]
    fn plugin_registers_status() {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(TeamStatusPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let status = ctx
            .service::<dyn TeamStatus>(&TEAM_STATUS)
            .expect("team-status seam");
        assert!(status.member_can_transition(Unstarted, Starting));
        assert!(status.member_settled(Paused));
        assert!(!status.member_departed(Paused));
        drop(effects);
        assert!(!ctx.has_service(&TEAM_STATUS));
    }
}
