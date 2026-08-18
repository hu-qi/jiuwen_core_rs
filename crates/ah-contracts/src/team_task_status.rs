//! team-task-status seam:团队任务状态机(对齐 openjiuwen/agent_teams/schema/status.py 的 TaskStatus)。
//!
//! - TaskStatus 7 态:两个可选闸门(PLANNING 计划闸门 / IN_REVIEW 验证闸门)互为镜像,
//!   各自带 rework 自环/回退边;
//! - TASK_TRANSITIONS:迁移表常量(纯数据),1:1 对齐 Python TASK_TRANSITIONS;
//! - TeamTaskStatus:Service Definition —— 迁移判定 / 合法后继 / 终止态。
//!
//! 契约零实现:判定逻辑(查表)由插件提供(ah-plugins-team-task-status)。

use crate::seam::Seam;

/// 团队任务状态(对齐 Python TaskStatus)。
///
/// 状态命名任务的*所处状态*(the task IS ...),驱动它的 claim/start/submit/approve/
/// verify 是迁移事件而非状态。PENDING 同时覆盖 autonomous(待 claim)与 scheduled
/// (已分配未启动,带 assignee);PLANNING 与 IN_REVIEW 是两个可选闸门。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// 待办:等待 claim(autonomous)或已分配未启动(scheduled)。
    Pending,
    /// 依赖未满足。
    Blocked,
    /// 计划闸门:成员准备计划等待 leader 批准(PLAN_MODE 可选)。
    Planning,
    /// 执行中(合并 CLAIMED/STARTED/PLAN_APPROVED 节点)。
    InProgress,
    /// 验证闸门:成员提交结果,reviewer 正在验证。
    InReview,
    /// 终止态:已通过验证/接受。
    Completed,
    /// 终止态:已取消。
    Cancelled,
}

/// TaskStatus 迁移表(1:1 对齐 Python TASK_TRANSITIONS)。
///
/// 单一超集覆盖两种派发模式与两个可选闸门;任务实际走哪条路径由调用它的方法决定,
/// 而非按派发模式分支。PLANNING / IN_REVIEW 是两个镜像闸门,各带自环/回退边(rework)。
pub const TASK_TRANSITIONS: &[(TaskStatus, &[TaskStatus])] = &[
    // pending:plan_mode 保留待规划 / claim(autonomous)/ start(scheduled)/ 阻塞 / 取消。
    (
        TaskStatus::Pending,
        &[
            TaskStatus::Planning,
            TaskStatus::InProgress,
            TaskStatus::Blocked,
            TaskStatus::Cancelled,
        ],
    ),
    // blocked:解除阻塞回 pending,或取消。
    (
        TaskStatus::Blocked,
        &[TaskStatus::Pending, TaskStatus::Cancelled],
    ),
    // planning:submit/reject rework 自环、approve_plan 进执行、reset 回 pending、阻塞、取消。
    (
        TaskStatus::Planning,
        &[
            TaskStatus::Planning,
            TaskStatus::InProgress,
            TaskStatus::Pending,
            TaskStatus::Blocked,
            TaskStatus::Cancelled,
        ],
    ),
    // in_progress:有 reviewer 进验证闸门 / 无 reviewer 直接完成 / reset / 阻塞 / 取消。
    (
        TaskStatus::InProgress,
        &[
            TaskStatus::InReview,
            TaskStatus::Completed,
            TaskStatus::Pending,
            TaskStatus::Blocked,
            TaskStatus::Cancelled,
        ],
    ),
    // in_review:verify pass 完成 / verify fail rework 回执行 / reset / 取消。
    (
        TaskStatus::InReview,
        &[
            TaskStatus::Completed,
            TaskStatus::InProgress,
            TaskStatus::Pending,
            TaskStatus::Cancelled,
        ],
    ),
    // 终止态:无出边。
    (TaskStatus::Completed, &[]),
    (TaskStatus::Cancelled, &[]),
];

/// 团队任务状态 Seam(Service Definition):状态机迁移判定与状态集合。
pub trait TeamTaskStatus: Seam {
    /// 迁移是否合法(迁移表查找)。
    fn can_transition(&self, current: TaskStatus, new: TaskStatus) -> bool;

    /// 给定状态的合法后继(按迁移表顺序)。
    fn allowed_transitions(&self, current: TaskStatus) -> Vec<TaskStatus>;

    /// 终止态(Completed / Cancelled)。
    fn is_terminal(&self, status: TaskStatus) -> bool;
}
