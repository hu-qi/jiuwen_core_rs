//! # ah-plugins-team-task-status
//!
//! Real team task-status state machine (aligned with openjiuwen/agent_teams/schema/
//! status.py TaskStatus + TASK_TRANSITIONS + is_valid_transition):
//! - can_transition:迁移表查找(TASK_TRANSITIONS 1:1 对齐 Python);
//! - allowed_transitions:返回迁移表行(按表顺序);
//! - is_terminal:Completed / Cancelled 为终止态。
//!
//! 纯函数、无 IO、无状态;迁移表常量定义在契约层(纯数据),判定逻辑在本插件。

use std::sync::Arc;

use ah_contracts::keys::TEAM_TASK_STATUS;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::team_task_status::{TASK_TRANSITIONS, TaskStatus, TeamTaskStatus};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 真实任务状态机实现(纯函数,查表)。
pub struct TaskStatusMachine;

impl Seam for TaskStatusMachine {}

impl TeamTaskStatus for TaskStatusMachine {
    fn can_transition(&self, current: TaskStatus, new: TaskStatus) -> bool {
        TASK_TRANSITIONS
            .iter()
            .find(|(s, _)| *s == current)
            .map(|(_, targets)| targets.contains(&new))
            .unwrap_or(false)
    }

    fn allowed_transitions(&self, current: TaskStatus) -> Vec<TaskStatus> {
        TASK_TRANSITIONS
            .iter()
            .find(|(s, _)| *s == current)
            .map(|(_, targets)| targets.to_vec())
            .unwrap_or_default()
    }

    fn is_terminal(&self, status: TaskStatus) -> bool {
        matches!(status, TaskStatus::Completed | TaskStatus::Cancelled)
    }
}

/// team-task-status 插件:注册 `team-task-status` seam。
pub struct TeamTaskStatusPlugin;

impl Plugin for TeamTaskStatusPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-team-task-status"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![TEAM_TASK_STATUS]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let machine: Arc<dyn TeamTaskStatus> = Arc::new(TaskStatusMachine);
        Ok(vec![ctx.register(TEAM_TASK_STATUS, machine)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TaskStatus::*;
    use ah_contracts::keys::TEAM_TASK_STATUS;
    use ah_contracts::team_task_status::TASK_TRANSITIONS;
    use ah_hub::plugin::DynPlugin;

    /// 断言 allowed_transitions 与迁移表行一致(顺序一致)。
    fn assert_allowed(machine: &TaskStatusMachine, from: TaskStatus, expected: &[TaskStatus]) {
        assert_eq!(machine.allowed_transitions(from), expected);
    }

    #[test]
    fn pending_forward_edges() {
        let machine = TaskStatusMachine;
        // claim(autonomous)/ start(scheduled)→ InProgress;plan_mode 保留 → Planning。
        assert!(machine.can_transition(Pending, Planning));
        assert!(machine.can_transition(Pending, InProgress));
        assert!(machine.can_transition(Pending, Blocked));
        assert!(machine.can_transition(Pending, Cancelled));
    }

    #[test]
    fn blocked_edges() {
        let machine = TaskStatusMachine;
        assert!(machine.can_transition(Blocked, Pending));
        assert!(machine.can_transition(Blocked, Cancelled));
        assert!(!machine.can_transition(Blocked, Planning));
        assert!(!machine.can_transition(Blocked, InProgress));
        assert!(!machine.can_transition(Blocked, InReview));
        assert!(!machine.can_transition(Blocked, Completed));
    }

    #[test]
    fn planning_self_loop_and_full_edges() {
        let machine = TaskStatusMachine;
        // submit/reject rework 自环。
        assert!(machine.can_transition(Planning, Planning));
        assert!(machine.can_transition(Planning, InProgress)); // approve_plan
        assert!(machine.can_transition(Planning, Pending)); // reset
        assert!(machine.can_transition(Planning, Blocked));
        assert!(machine.can_transition(Planning, Cancelled));
        // 计划闸门不能直接跳到验证闸门或完成。
        assert!(!machine.can_transition(Planning, InReview));
        assert!(!machine.can_transition(Planning, Completed));
    }

    #[test]
    fn in_progress_edges() {
        let machine = TaskStatusMachine;
        assert!(machine.can_transition(InProgress, InReview)); // 有 reviewer
        assert!(machine.can_transition(InProgress, Completed)); // 无 reviewer
        assert!(machine.can_transition(InProgress, Pending)); // reset
        assert!(machine.can_transition(InProgress, Blocked));
        assert!(machine.can_transition(InProgress, Cancelled));
        assert!(!machine.can_transition(InProgress, Planning));
    }

    #[test]
    fn in_review_edges() {
        let machine = TaskStatusMachine;
        assert!(machine.can_transition(InReview, Completed)); // verify pass
        assert!(machine.can_transition(InReview, InProgress)); // verify fail rework
        assert!(machine.can_transition(InReview, Pending)); // reset
        assert!(machine.can_transition(InReview, Cancelled));
        assert!(!machine.can_transition(InReview, Planning));
        assert!(!machine.can_transition(InReview, Blocked));
    }

    #[test]
    fn terminal_states_have_no_outgoing_edges() {
        let machine = TaskStatusMachine;
        let others = [
            Pending, Blocked, Planning, InProgress, InReview, Completed, Cancelled,
        ];
        for to in others {
            assert!(!machine.can_transition(Completed, to));
            assert!(!machine.can_transition(Cancelled, to));
        }
    }

    #[test]
    fn illegal_transitions_rejected() {
        let machine = TaskStatusMachine;
        // 跳闸门:直接 Pending → Completed / InReview 非法。
        assert!(!machine.can_transition(Pending, Completed));
        assert!(!machine.can_transition(Pending, InReview));
        // 完成/取消后不可复活。
        assert!(!machine.can_transition(Completed, Pending));
        assert!(!machine.can_transition(Cancelled, InProgress));
        // Blocked 只能回 Pending 或取消。
        assert!(!machine.can_transition(Blocked, Completed));
    }

    #[test]
    fn allowed_transitions_match_table() {
        let machine = TaskStatusMachine;
        assert_allowed(
            &machine,
            Pending,
            &[Planning, InProgress, Blocked, Cancelled],
        );
        assert_allowed(&machine, Blocked, &[Pending, Cancelled]);
        assert_allowed(
            &machine,
            Planning,
            &[Planning, InProgress, Pending, Blocked, Cancelled],
        );
        assert_allowed(
            &machine,
            InProgress,
            &[InReview, Completed, Pending, Blocked, Cancelled],
        );
        assert_allowed(
            &machine,
            InReview,
            &[Completed, InProgress, Pending, Cancelled],
        );
        assert_allowed(&machine, Completed, &[]);
        assert_allowed(&machine, Cancelled, &[]);
    }

    #[test]
    fn transition_table_covers_every_state_once() {
        // 7 个状态各恰好出现一次作为迁移表起点,总边数 = 4+2+5+5+4+0+0 = 20。
        let mut seen = Vec::new();
        let mut total_edges = 0usize;
        for (from, targets) in TASK_TRANSITIONS {
            assert!(!seen.contains(from), "状态 {:?} 在迁移表中重复出现", from);
            seen.push(*from);
            total_edges += targets.len();
        }
        assert_eq!(seen.len(), 7);
        assert_eq!(total_edges, 20);
        for s in [
            Pending, Blocked, Planning, InProgress, InReview, Completed, Cancelled,
        ] {
            assert!(seen.contains(&s), "状态 {:?} 缺失", s);
        }
    }

    #[test]
    fn is_terminal_statuses() {
        let machine = TaskStatusMachine;
        assert!(machine.is_terminal(Completed));
        assert!(machine.is_terminal(Cancelled));
        for s in [Pending, Blocked, Planning, InProgress, InReview] {
            assert!(!machine.is_terminal(s), "{:?} 不应是终止态", s);
        }
    }

    #[test]
    fn serde_snake_case_roundtrip() {
        // 序列化必须对齐 Python 值:pending/blocked/planning/in_progress/in_review/completed/cancelled。
        assert_eq!(serde_json::to_string(&Pending).unwrap(), "\"pending\"");
        assert_eq!(serde_json::to_string(&Blocked).unwrap(), "\"blocked\"");
        assert_eq!(serde_json::to_string(&Planning).unwrap(), "\"planning\"");
        assert_eq!(
            serde_json::to_string(&InProgress).unwrap(),
            "\"in_progress\""
        );
        assert_eq!(serde_json::to_string(&InReview).unwrap(), "\"in_review\"");
        assert_eq!(serde_json::to_string(&Completed).unwrap(), "\"completed\"");
        assert_eq!(serde_json::to_string(&Cancelled).unwrap(), "\"cancelled\"");
        let back: TaskStatus = serde_json::from_str("\"in_progress\"").unwrap();
        assert_eq!(back, InProgress);
    }

    #[test]
    fn plugin_registers_team_task_status() {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(TeamTaskStatusPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let machine = ctx
            .service::<dyn TeamTaskStatus>(&TEAM_TASK_STATUS)
            .expect("team-task-status seam");
        // 经 seam 对象断言真实行为。
        assert!(machine.can_transition(Pending, InProgress));
        assert!(machine.can_transition(InProgress, InReview));
        assert!(!machine.can_transition(Pending, Completed));
        assert!(machine.is_terminal(Completed));
        assert_eq!(
            machine.allowed_transitions(InReview),
            vec![Completed, InProgress, Pending, Cancelled]
        );
        drop(effects);
        assert!(!ctx.has_service(&TEAM_TASK_STATUS));
    }
}
