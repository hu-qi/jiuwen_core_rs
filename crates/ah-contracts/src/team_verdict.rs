//! team-verdict seam:审查投票判定(对齐 openjiuwen/agent_teams/agent/scheduling/verdict.py)。
//!
//! 纯函数投票数学(无 IO、无状态):
//! - quorum = ceil(threshold × reviewer_count);
//! - pass_count ≥ quorum → PASS;
//! - fail_count > reviewer_count − quorum(quorum 不可达)→ FAIL;
//! - 否则 UNDECIDED;reviewer_count ≤ 0 → UNDECIDED。
//!
//! 单 reviewer 在默认 2/3 阈值下退化为首个判定即定。

use crate::seam::Seam;

/// 投票判定结果(对齐 VERDICT_PASS/FAIL/UNDECIDED)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Pass,
    Fail,
    Undecided,
}

/// 投票判定 Seam(Service Definition):纯函数。
pub trait TeamVerdict: Seam {
    /// pass 配额(ceil(threshold × reviewer_count))。
    fn quorum(&self, reviewer_count: u32, threshold: f64) -> u32;

    /// 判定一轮投票统计。
    fn judge(
        &self,
        pass_count: u32,
        fail_count: u32,
        reviewer_count: u32,
        threshold: f64,
    ) -> Verdict;
}
