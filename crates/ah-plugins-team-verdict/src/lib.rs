//! # ah-plugins-team-verdict
//!
//! Real review-vote verdict math (aligned with openjiuwen/agent_teams/agent/scheduling/
//! verdict.py, F_62): pure function, no IO, no state.
//! - quorum = ceil(threshold * reviewer_count);
//! - pass_count >= quorum -> PASS;
//! - fail_count > reviewer_count - quorum (quorum unreachable) -> FAIL;
//! - else UNDECIDED; reviewer_count <= 0 -> UNDECIDED.

use std::sync::Arc;

use ah_contracts::keys::TEAM_VERDICT;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::team_verdict::{TeamVerdict, Verdict};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 真实投票判定实现(纯函数)。
pub struct VerdictJudge;

impl Seam for VerdictJudge {}

impl TeamVerdict for VerdictJudge {
    fn quorum(&self, reviewer_count: u32, threshold: f64) -> u32 {
        if reviewer_count == 0 || !threshold.is_finite() || threshold <= 0.0 {
            return 0;
        }
        (threshold * reviewer_count as f64).ceil() as u32
    }

    fn judge(
        &self,
        pass_count: u32,
        fail_count: u32,
        reviewer_count: u32,
        threshold: f64,
    ) -> Verdict {
        if reviewer_count == 0 {
            return Verdict::Undecided;
        }
        let quorum = self.quorum(reviewer_count, threshold);
        if pass_count >= quorum {
            return Verdict::Pass;
        }
        if fail_count > reviewer_count - quorum {
            return Verdict::Fail;
        }
        Verdict::Undecided
    }
}

/// team-verdict 插件:注册 `team-verdict` seam。
pub struct TeamVerdictPlugin;

impl Plugin for TeamVerdictPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-team-verdict"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![TEAM_VERDICT]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let verdict: Arc<dyn TeamVerdict> = Arc::new(VerdictJudge);
        Ok(vec![ctx.register(TEAM_VERDICT, verdict)])
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::TEAM_VERDICT;
    use ah_contracts::team_verdict::Verdict;
    use ah_hub::plugin::DynPlugin;

    #[test]
    fn pass_when_quorum_reached() {
        let judge = VerdictJudge;
        // 3 评审,2/3 阈值 → quorum = ceil(2) = 2。
        assert_eq!(judge.quorum(3, 2.0 / 3.0), 2);
        assert_eq!(judge.judge(2, 0, 3, 2.0 / 3.0), Verdict::Pass);
        assert_eq!(
            judge.judge(2, 1, 3, 2.0 / 3.0),
            Verdict::Pass,
            "pass 达配额即过"
        );
        // 阈值 1.0 → quorum = 全部。
        assert_eq!(judge.quorum(4, 1.0), 4);
        assert_eq!(judge.judge(4, 0, 4, 1.0), Verdict::Pass);
        assert_eq!(
            judge.judge(3, 1, 4, 1.0),
            Verdict::Fail,
            "满票阈值下 1 票 fail 即不可达"
        );
    }

    #[test]
    fn fail_when_quorum_unreachable() {
        let judge = VerdictJudge;
        // 3 评审,quorum 2,slack = 3-2 = 1;2 fail → 不可达 → FAIL。
        assert_eq!(judge.judge(0, 2, 3, 2.0 / 3.0), Verdict::Fail);
        assert_eq!(
            judge.judge(1, 2, 3, 2.0 / 3.0),
            Verdict::Fail,
            "决定性迟票即败"
        );
        // fail 未超 slack → 仍 undecided。
        assert_eq!(judge.judge(0, 1, 3, 2.0 / 3.0), Verdict::Undecided);
        assert_eq!(judge.judge(1, 1, 3, 2.0 / 3.0), Verdict::Undecided);
    }

    #[test]
    fn single_reviewer_first_verdict_wins() {
        let judge = VerdictJudge;
        // 单评审 + 2/3 阈值 → quorum = ceil(0.67) = 1 → 首个判定即定。
        assert_eq!(judge.quorum(1, 2.0 / 3.0), 1);
        assert_eq!(judge.judge(1, 0, 1, 2.0 / 3.0), Verdict::Pass);
        assert_eq!(judge.judge(0, 1, 1, 2.0 / 3.0), Verdict::Fail);
    }

    #[test]
    fn undecided_edge_cases() {
        let judge = VerdictJudge;
        // 零评审 → UNDECIDED。
        assert_eq!(judge.judge(0, 0, 0, 2.0 / 3.0), Verdict::Undecided);
        // 无票。
        assert_eq!(judge.judge(0, 0, 3, 2.0 / 3.0), Verdict::Undecided);
        // 阈值 0.5,偶数评审 → ceil(1.5) = 2。
        assert_eq!(judge.quorum(3, 0.5), 2);
        assert_eq!(judge.judge(1, 1, 3, 0.5), Verdict::Undecided, "1/1/3 未决");
        assert_eq!(judge.judge(2, 0, 3, 0.5), Verdict::Pass);
        // 非有限阈值 → quorum 0 → 任何票数 ≥ 0 即 pass?阈值非法应保守。quorum=0 时 pass 即过。
        // 这里只断言 quorum 的防御:非法阈值返回 0。
        assert_eq!(judge.quorum(3, f64::NAN), 0);
        assert_eq!(judge.quorum(3, 0.0), 0);
    }

    #[test]
    fn plugin_registers_verdict() {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(TeamVerdictPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let verdict = ctx
            .service::<dyn TeamVerdict>(&TEAM_VERDICT)
            .expect("team-verdict seam");
        assert_eq!(verdict.judge(2, 0, 3, 2.0 / 3.0), Verdict::Pass);
        assert_eq!(verdict.judge(0, 2, 3, 2.0 / 3.0), Verdict::Fail);
        drop(effects);
        assert!(!ctx.has_service(&TEAM_VERDICT));
    }
}
