//! team-scheduler seam:团队调度器扫描决策核心(对齐 openjiuwen/agent_teams/agent/
//! scheduling/scheduler.py 的纯决策部分:F_62 / S_22)。
//!
//! 只做决策,不碰 DB:调用方负责取行(list pending / in_review)、查 busy、写结算。
//! 决策语义 1:1 对齐 Python 纯决策部分:
//! - **开工扫描**(`pick_starts`):pending 按 assignee 分组,每人取队首候选
//!   (updated_at 缺省 0 升序、同则 task_id 字典序);成员已有其它活跃任务
//!   (busy_members,即 `get_other_active_task_id` 非空)或无候选则跳过;
//! - **审阅扫描**(`review_decision`):judge 复用 team_verdict 的投票公式
//!   (quorum = ceil(threshold × reviewer_count),pass ≥ quorum → PASS,
//!   fail > reviewer_count − quorum → FAIL,否则 UNDECIDED;reviewer_count ≤ 0 →
//!   UNDECIDED);PASS → 结算通过;FAIL → 轮数达上限升级、否则结算失败;
//!   UNDECIDED → 停摆超时升级、否则首次送审;
//! - **送审去重**(`review_dispatch_key`):按 (task_id, review_round) 构造去重键,
//!   集合去重语义由调用方持有 `HashSet<ReviewDispatchKey>` 实现。
//!
//! 契约零实现:算法由插件(如 ah-plugins-team-scheduler)提供。

use std::collections::HashSet;

use crate::seam::Seam;

/// 待开工任务投影(对齐 `_reconcile_starts` 的 pending 行)。
///
/// `status` 为调用方已过滤的当前状态(如 `pending`);`assignee` 为空的任务
/// 不参与开工决策。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SchedulerTask {
    pub task_id: String,
    pub assignee: Option<String>,
    /// 最后更新时间(毫秒 epoch);None 视为 0(最优先)。
    pub updated_at: Option<u64>,
    pub status: String,
}

/// 审阅决策输入任务(对齐 `_reconcile_reviews` 的 in_review 行)。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReviewTask {
    pub task_id: String,
    pub review_round: u64,
    /// 最后更新时间(毫秒 epoch);None 视为"刚更新"(age = 0,不触发停摆)。
    pub updated_at: Option<u64>,
    /// 任务行上的轮数上限;None 时由调用方传入的 `max_rounds` 兜底。
    pub max_review_rounds: Option<u64>,
}

/// 一轮投票统计(对齐 `get_review_tally`)。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReviewTally {
    pub pass_count: u32,
    pub fail_count: u32,
    pub reviewer_count: u32,
    /// 已投票评审名(用于调用方渲染停摆升级的 pending 名单)。
    pub voted: Vec<String>,
}

/// 送审去重键 (task_id, review_round)。
///
/// 同一 (task, round) 只送审一次;去重集合由调用方持有。
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct ReviewDispatchKey {
    pub task_id: String,
    pub review_round: u64,
}

impl ReviewDispatchKey {
    /// 构造去重键。
    pub fn new(task_id: impl Into<String>, review_round: u64) -> Self {
        Self {
            task_id: task_id.into(),
            review_round,
        }
    }
}

/// 审阅决策动作(对齐 `_settle_pass` / `_settle_fail_or_escalate` /
/// `_handle_undecided`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewAction {
    /// 未决且未停摆:首次送审(向全部 reviewer 发请求)并保持 undecided。
    DispatchAndUndecided,
    /// 判定通过:结算 pass。
    SettlePass,
    /// 判定失败且轮数未达上限:结算 fail(带反馈,返回该轮返工)。
    SettleFail,
    /// 判定失败且轮数已达上限:升级给 leader(不再开新轮)。
    EscalateRounds,
    /// 未决且已停摆超时:升级给 leader。
    EscalateStall,
}

/// 团队调度器 Seam(Service Definition):纯决策核心,无 IO、无状态。
///
/// judge 投票公式与 `team_verdict` 契约一致(quorum = ceil(threshold × n),
/// fail > n − quorum 即不可达);实现方可声明依赖 team-verdict seam 或内联
/// 同样公式。
pub trait TeamScheduler: Seam {
    /// 开工扫描:按 assignee 分组取每人最早的 PENDING 候选。
    ///
    /// - 候选 = updated_at(None 视 0)最小、同则 task_id 字典序最小;
    /// - 成员在 `busy_members`(已有其它活跃任务)或无候选则跳过;
    /// - 无 assignee 的任务跳过;
    /// - 输出按 assignee 稳定顺序(字典序),每个成员至多一条。
    fn pick_starts(
        &self,
        pending: &[SchedulerTask],
        busy_members: &HashSet<String>,
    ) -> Vec<(String, SchedulerTask)>;

    /// 审阅决策:judge 一轮 tally,再按轮数上限 / 停摆超时折叠出动作。
    ///
    /// - Pass → [`ReviewAction::SettlePass`];
    /// - Fail → 轮数已达上限(任务行 `max_review_rounds` 或 `max_rounds` 兜底)
    ///   → [`ReviewAction::EscalateRounds`],否则 [`ReviewAction::SettleFail`];
    /// - Undecided → age_ms = now_ms − updated_at(None 视 now_ms,即 age 0)
    ///   ≥ stall_timeout_secs × 1000 → [`ReviewAction::EscalateStall`],
    ///   否则 [`ReviewAction::DispatchAndUndecided`](首次送审)。
    ///
    /// `max_rounds` 为调用方解析后的轮数上限(任务行无值时用 spec 默认)。
    fn review_decision(
        &self,
        task: &ReviewTask,
        tally: &ReviewTally,
        threshold: f64,
        max_rounds: u32,
        stall_timeout_secs: u64,
        now_ms: u64,
    ) -> ReviewAction;

    /// 送审去重键构造((task_id, review_round));集合去重语义由调用方做。
    fn review_dispatch_key(&self, task_id: &str, review_round: u64) -> ReviewDispatchKey;
}
