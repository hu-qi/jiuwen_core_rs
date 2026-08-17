//! # ah-plugins-team-scheduler
//!
//! Real team scheduler scan decision core (aligned with openjiuwen/agent_teams/
//! agent/scheduling/scheduler.py pure decision parts, F_62 / S_22):
//! - `pick_starts`: per-member earliest PENDING(assignee) task (updated_at asc,
//!   task_id lexicographic tie-break; None updated_at treated as 0), skipping
//!   members already busy with another active task (`get_other_active_task_id`);
//! - `review_decision`: judge the round tally with the same quorum math as
//!   team-verdict (quorum = ceil(threshold × n), fail > n − quorum → FAIL), then
//!   fold to settle pass / settle fail / escalate rounds (round ≥ ceiling) /
//!   escalate stall (age ≥ stall_timeout) / dispatch-and-undecided;
//! - `review_dispatch_key`: (task_id, review_round) dedup key constructor.
//!
//! Pure decisions only: DB reads, review dispatch delivery and settle writes
//! belong to the caller.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use ah_contracts::keys::TEAM_SCHEDULER;
use ah_contracts::prelude::Effect;
use ah_contracts::scheduler::{
    ReviewAction, ReviewDispatchKey, ReviewTally, ReviewTask, SchedulerTask, TeamScheduler,
};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::team_verdict::Verdict;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// pass 配额 ceil(threshold × reviewer_count);零评审/非法阈值 → 0。
/// 与 team-verdict 的 `TeamVerdict::quorum` 同一公式(内联,保持决策核心自足)。
fn quorum(reviewer_count: u32, threshold: f64) -> u32 {
    if reviewer_count == 0 || !threshold.is_finite() || threshold <= 0.0 {
        return 0;
    }
    (threshold * reviewer_count as f64).ceil() as u32
}

/// 一轮 tally 的投票判定(对齐 verdict.judge:pass ≥ quorum → PASS;
/// fail > reviewer_count − quorum → FAIL(quorum 不可达);否则 UNDECIDED;
/// reviewer_count ≤ 0 → UNDECIDED)。
fn judge(pass_count: u32, fail_count: u32, reviewer_count: u32, threshold: f64) -> Verdict {
    if reviewer_count == 0 {
        return Verdict::Undecided;
    }
    let q = quorum(reviewer_count, threshold);
    if pass_count >= q {
        return Verdict::Pass;
    }
    if fail_count > reviewer_count - q {
        return Verdict::Fail;
    }
    Verdict::Undecided
}

/// 真实团队调度器决策核心(纯决策,无 IO、无状态)。
pub struct SchedulerCore;

impl Seam for SchedulerCore {}

impl TeamScheduler for SchedulerCore {
    fn pick_starts(
        &self,
        pending: &[SchedulerTask],
        busy_members: &HashSet<String>,
    ) -> Vec<(String, SchedulerTask)> {
        // BTreeMap:按 assignee 字典序稳定分组(输出顺序确定)。
        let mut by_member: BTreeMap<&str, Vec<&SchedulerTask>> = BTreeMap::new();
        for task in pending {
            if let Some(assignee) = task.assignee.as_deref() {
                by_member.entry(assignee).or_default().push(task);
            }
        }
        let mut out = Vec::with_capacity(by_member.len());
        for (member, queue) in by_member {
            if busy_members.contains(member) {
                continue;
            }
            // 队首候选:updated_at(None 视 0)最小、同则 task_id 字典序最小。
            if let Some(candidate) = queue.iter().min_by(|a, b| {
                (a.updated_at.unwrap_or(0), a.task_id.as_str())
                    .cmp(&(b.updated_at.unwrap_or(0), b.task_id.as_str()))
            }) {
                out.push((member.to_string(), SchedulerTask::clone(candidate)));
            }
        }
        out
    }

    fn review_decision(
        &self,
        task: &ReviewTask,
        tally: &ReviewTally,
        threshold: f64,
        max_rounds: u32,
        stall_timeout_secs: u64,
        now_ms: u64,
    ) -> ReviewAction {
        match judge(
            tally.pass_count,
            tally.fail_count,
            tally.reviewer_count,
            threshold,
        ) {
            Verdict::Pass => ReviewAction::SettlePass,
            Verdict::Fail => {
                // 轮数上限:任务行 `max_review_rounds` 优先,缺省用调用方解析值。
                let ceiling = task.max_review_rounds.unwrap_or(max_rounds as u64);
                if task.review_round >= ceiling {
                    ReviewAction::EscalateRounds
                } else {
                    ReviewAction::SettleFail
                }
            }
            Verdict::Undecided => {
                // age = now − updated_at(None 视 now,即 age 0,永不因缺时间戳停摆)。
                let age_ms = now_ms.saturating_sub(task.updated_at.unwrap_or(now_ms));
                if age_ms >= stall_timeout_secs.saturating_mul(1000) {
                    ReviewAction::EscalateStall
                } else {
                    ReviewAction::DispatchAndUndecided
                }
            }
        }
    }

    fn review_dispatch_key(&self, task_id: &str, review_round: u64) -> ReviewDispatchKey {
        ReviewDispatchKey::new(task_id, review_round)
    }
}

/// team-scheduler 插件:注册 `team-scheduler` seam。
pub struct TeamSchedulerPlugin;

impl Plugin for TeamSchedulerPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-team-scheduler"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![TEAM_SCHEDULER]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let scheduler: Arc<dyn TeamScheduler> = Arc::new(SchedulerCore);
        Ok(vec![ctx.register(TEAM_SCHEDULER, scheduler)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_hub::plugin::DynPlugin;

    fn task(task_id: &str, assignee: Option<&str>, updated_at: Option<u64>) -> SchedulerTask {
        SchedulerTask {
            task_id: task_id.to_string(),
            assignee: assignee.map(|s| s.to_string()),
            updated_at,
            status: "pending".to_string(),
        }
    }

    fn tally(pass: u32, fail: u32, total: u32) -> ReviewTally {
        ReviewTally {
            pass_count: pass,
            fail_count: fail,
            reviewer_count: total,
            voted: Vec::new(),
        }
    }

    fn review_task(round: u64, updated_at: Option<u64>, max_rounds: Option<u64>) -> ReviewTask {
        ReviewTask {
            task_id: "t-review".to_string(),
            review_round: round,
            updated_at,
            max_review_rounds: max_rounds,
        }
    }

    #[test]
    fn pick_starts_picks_earliest_per_member_in_stable_order() {
        let core = SchedulerCore;
        let pending = vec![
            task("t3", Some("alice"), Some(30)),
            task("t1", Some("alice"), Some(10)),
            task("t2", Some("alice"), Some(20)),
            task("t4", Some("bob"), Some(50)),
            task("t5", Some("bob"), Some(5)),
        ];
        let busy = HashSet::new();
        let starts = core.pick_starts(&pending, &busy);
        // 每人只取最早(alice→t1@10,bob→t5@5);输出按 assignee 字典序。
        let members: Vec<&str> = starts.iter().map(|(m, _)| m.as_str()).collect();
        let picked: Vec<&str> = starts.iter().map(|(_, t)| t.task_id.as_str()).collect();
        assert_eq!(members, vec!["alice", "bob"]);
        assert_eq!(picked, vec!["t1", "t5"]);
    }

    #[test]
    fn pick_starts_tie_break_by_task_id_lexicographic() {
        let core = SchedulerCore;
        // 同 updated_at → task_id 字典序小者优先;None 与 None 同 0 亦按 task_id。
        let pending = vec![
            task("tz", Some("alice"), Some(10)),
            task("ta", Some("alice"), Some(10)),
            task("tb", Some("bob"), None),
            task("ba", Some("bob"), None),
        ];
        let busy = HashSet::new();
        let starts = core.pick_starts(&pending, &busy);
        assert_eq!(starts[0].1.task_id, "ta");
        assert_eq!(starts[1].1.task_id, "ba");
    }

    #[test]
    fn pick_starts_skips_busy_members() {
        let core = SchedulerCore;
        let pending = vec![
            task("t1", Some("alice"), Some(1)),
            task("t2", Some("bob"), Some(1)),
        ];
        // busy = 成员已有其它活跃任务(get_other_active_task_id 非空)。
        let busy: HashSet<String> = ["alice".to_string()].into_iter().collect();
        let starts = core.pick_starts(&pending, &busy);
        assert_eq!(starts.len(), 1);
        assert_eq!(starts[0].0, "bob");
        // 全部 busy → 空。
        let all_busy: HashSet<String> = ["alice".to_string(), "bob".to_string()]
            .into_iter()
            .collect();
        assert!(core.pick_starts(&pending, &all_busy).is_empty());
    }

    #[test]
    fn pick_starts_skips_unassigned_and_empty_input() {
        let core = SchedulerCore;
        let pending = vec![
            task("t1", None, Some(1)),
            task("t2", None, None),
            task("t3", Some("alice"), Some(3)),
        ];
        let busy = HashSet::new();
        let starts = core.pick_starts(&pending, &busy);
        assert_eq!(starts.len(), 1);
        assert_eq!(starts[0].0, "alice");
        assert!(core.pick_starts(&[], &busy).is_empty());
    }

    #[test]
    fn pick_starts_none_updated_at_counts_as_zero() {
        let core = SchedulerCore;
        // None updated_at 视 0 → 比任何显式时间戳都早,即"最早"。
        let pending = vec![
            task("t-old", Some("alice"), Some(100)),
            task("t-new", Some("alice"), None),
        ];
        let busy = HashSet::new();
        let starts = core.pick_starts(&pending, &busy);
        assert_eq!(starts[0].1.task_id, "t-new");
    }

    #[test]
    fn review_decision_pass_settles() {
        let core = SchedulerCore;
        // 3 评审 2/3 阈值 → quorum = 2;2 pass → PASS → SettlePass。
        let task = review_task(1, Some(1_000), None);
        assert_eq!(
            core.review_decision(&task, &tally(2, 0, 3), 2.0 / 3.0, 3, 1800, 1_000_000),
            ReviewAction::SettlePass
        );
        // 单评审退化为首个判定即定。
        assert_eq!(
            core.review_decision(&task, &tally(1, 0, 1), 2.0 / 3.0, 3, 1800, 1_000_000),
            ReviewAction::SettlePass
        );
    }

    #[test]
    fn review_decision_fail_below_ceiling_settles_fail() {
        let core = SchedulerCore;
        // 3 评审 quorum 2、slack 1;2 fail → quorum 不可达 → FAIL。
        let tally = tally(0, 2, 3);
        // round 1 < 默认上限 3 → SettleFail(返工)。
        let task = review_task(1, Some(1_000), None);
        assert_eq!(
            core.review_decision(&task, &tally, 2.0 / 3.0, 3, 1800, 1_000_000),
            ReviewAction::SettleFail
        );
        // round 2 < 任务行上限 3 → SettleFail。
        let task = review_task(2, Some(1_000), Some(3));
        assert_eq!(
            core.review_decision(&task, &tally, 2.0 / 3.0, 3, 1800, 1_000_000),
            ReviewAction::SettleFail
        );
    }

    #[test]
    fn review_decision_fail_at_ceiling_escalates_rounds() {
        let core = SchedulerCore;
        let tally = tally(0, 2, 3);
        // round 3 >= 默认上限 3 → EscalateRounds。
        let task = review_task(3, Some(1_000), None);
        assert_eq!(
            core.review_decision(&task, &tally, 2.0 / 3.0, 3, 1800, 1_000_000),
            ReviewAction::EscalateRounds
        );
        // 任务行 max_review_rounds=2 优先于入参 3:round 2 已达上限 → EscalateRounds。
        let task = review_task(2, Some(1_000), Some(2));
        assert_eq!(
            core.review_decision(&task, &tally, 2.0 / 3.0, 3, 1800, 1_000_000),
            ReviewAction::EscalateRounds
        );
    }

    #[test]
    fn review_decision_undecided_stalled_escalates_stall() {
        let core = SchedulerCore;
        // 1 pass / 1 fail / 3 评审 → 未决(不达 quorum 也未不可达)。
        let tally = tally(1, 1, 3);
        let stall = 1800u64;
        // 恰好 age == stall × 1000 → 停摆升级。
        let updated = 1_000_000u64;
        let task = review_task(1, Some(updated), None);
        assert_eq!(
            core.review_decision(&task, &tally, 2.0 / 3.0, 3, stall, updated + stall * 1000),
            ReviewAction::EscalateStall
        );
        // 超过 → 停摆升级。
        assert_eq!(
            core.review_decision(
                &task,
                &tally,
                2.0 / 3.0,
                3,
                stall,
                updated + stall * 1000 + 1
            ),
            ReviewAction::EscalateStall
        );
    }

    #[test]
    fn review_decision_undecided_fresh_dispatches() {
        let core = SchedulerCore;
        let tally = tally(1, 1, 3);
        let stall = 1800u64;
        let now = 1_000_000u64;
        // 未超时 → 首次送审(保持 undecided)。
        let task = review_task(1, Some(now - 1000), None);
        assert_eq!(
            core.review_decision(&task, &tally, 2.0 / 3.0, 3, stall, now),
            ReviewAction::DispatchAndUndecided
        );
        // updated_at None → age 0 → 首次送审。
        let task = review_task(1, None, None);
        assert_eq!(
            core.review_decision(&task, &tally, 2.0 / 3.0, 3, stall, now),
            ReviewAction::DispatchAndUndecided
        );
    }

    #[test]
    fn review_decision_zero_reviewers_stays_undecided() {
        let core = SchedulerCore;
        // reviewer_count = 0 → judge 返回 Undecided(照 team-verdict)。
        let tally = tally(0, 0, 0);
        let now = 1_000_000u64;
        // 无评审也无停摆 → 首次送审语义(调用方上游通常跳过无评审任务)。
        let task = review_task(1, Some(now - 1000), None);
        assert_eq!(
            core.review_decision(&task, &tally, 2.0 / 3.0, 3, 1800, now),
            ReviewAction::DispatchAndUndecided
        );
        // 时间戳极旧(now 100_000_000ms ≈ 27.8h > 1800s 停摆线)→ 停摆升级分支
        // (仍不误判 pass/fail)。
        let stale = review_task(1, Some(0), None);
        assert_eq!(
            core.review_decision(&stale, &tally, 2.0 / 3.0, 3, 1800, 100_000_000),
            ReviewAction::EscalateStall
        );
    }

    #[test]
    fn review_dispatch_key_identity_and_dedup() {
        let core = SchedulerCore;
        let k1 = core.review_dispatch_key("t1", 1);
        // 同一 (task_id, round) 相等;不同 round / 不同 task 不等。
        assert_eq!(k1, ReviewDispatchKey::new("t1", 1));
        assert_ne!(k1, core.review_dispatch_key("t1", 2));
        assert_ne!(k1, core.review_dispatch_key("t2", 1));
        // 调用方去重语义:HashSet 按键收敛。
        let mut dispatched = HashSet::new();
        dispatched.insert(k1.clone());
        dispatched.insert(core.review_dispatch_key("t1", 1));
        dispatched.insert(core.review_dispatch_key("t1", 2));
        assert_eq!(dispatched.len(), 2, "同 (task, round) 只送审一次");
        assert!(dispatched.contains(&k1));
    }

    #[test]
    fn plugin_registers_scheduler() {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(TeamSchedulerPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let scheduler = ctx
            .service::<dyn TeamScheduler>(&TEAM_SCHEDULER)
            .expect("team-scheduler seam");
        // 经 trait 对象走真实决策路径。
        let pending = vec![task("t1", Some("alice"), Some(1))];
        let starts = scheduler.pick_starts(&pending, &HashSet::new());
        assert_eq!(starts.len(), 1);
        assert_eq!(starts[0].1.task_id, "t1");
        let t = review_task(3, Some(1), None);
        assert_eq!(
            scheduler.review_decision(&t, &tally(0, 2, 3), 2.0 / 3.0, 3, 1800, 2_000_000),
            ReviewAction::EscalateRounds
        );
        drop(effects);
        assert!(!ctx.has_service(&TEAM_SCHEDULER));
    }
}
