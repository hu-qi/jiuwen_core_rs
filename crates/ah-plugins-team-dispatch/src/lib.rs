//! # ah-plugins-team-dispatch
//!
//! Real team run-dispatch decision (aligned with openjiuwen/agent_teams/runtime/dispatch.py):
//! a pure 7-way truth table over (team_in_db, team_in_session, pool_entry, team_db_state):
//! CREATE / NEW_TEAM_IN_SESSION / COLD_RECOVER / RESUME_FROM_PAUSE / REJECT_RUNNING /
//! REJECT_ORPHANED / REJECT_INCONSISTENT. No side effects; the caller executes the
//! corresponding effect. Cross-session pool entries are a contract violation -> Err.

use std::sync::Arc;

use ah_contracts::keys::TEAM_DISPATCH;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::team_dispatch::{
    DispatchError, PoolEntry, RunAction, RunActionKind, RuntimeState, TEAM_DB_STATE_CLEANED,
    TEAM_DB_STATE_PENDING_CREATE, TeamDispatch,
};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 真实派发决策器(纯函数)。
pub struct DispatchDecider;

impl Seam for DispatchDecider {}

impl TeamDispatch for DispatchDecider {
    fn decide(
        &self,
        team_in_db: bool,
        team_in_session: bool,
        pool_entry: Option<&PoolEntry>,
        target_session_id: &str,
        target_team_name: &str,
        team_db_state: Option<&str>,
    ) -> Result<RunAction, DispatchError> {
        // 可重建:checkpoint 桶描述的团队 DB 行尚未创建或已清理。
        // 必须先于 REJECT_INCONSISTENT:pending_create/cleaned + pool entry 应允许重建。
        if !team_in_db && team_in_session {
            match team_db_state {
                Some(TEAM_DB_STATE_PENDING_CREATE) | Some(TEAM_DB_STATE_CLEANED) => {
                    return Ok(RunAction::new(RunActionKind::Create, true, None));
                }
                _ => {
                    return Ok(RunAction::new(
                        RunActionKind::RejectOrphaned,
                        false,
                        Some(format!(
                            "team {target_team_name:?} not in DB but session bucket exists for {target_session_id:?}"
                        )),
                    ));
                }
            }
        }
        // 不一致:池说活跃,DB 说无行(仅当 db_state 非 pending_create/cleaned 时到达)。
        if !team_in_db && pool_entry.is_some() {
            return Ok(RunAction::new(
                RunActionKind::RejectInconsistent,
                false,
                Some(format!(
                    "team {target_team_name:?} present in pool but missing from DB"
                )),
            ));
        }
        // 全新团队:需要 spec。
        if !team_in_db {
            return Ok(RunAction::new(RunActionKind::Create, true, None));
        }
        // 冷路径(无池条目,DB 有团队)。
        if pool_entry.is_none() {
            if team_in_session {
                return Ok(RunAction::new(RunActionKind::ColdRecover, false, None));
            }
            return Ok(RunAction::new(RunActionKind::NewTeamInSession, false, None));
        }
        // 池条目存在;activate 保证其属于 target_session_id。
        let entry = pool_entry.expect("checked above");
        if entry.current_session_id != target_session_id {
            return Err(DispatchError(format!(
                "dispatch invariant violated: pool entry for {target_team_name:?} on session {:?} must be torn down before dispatching to session {target_session_id:?}",
                entry.current_session_id,
            )));
        }
        if entry.state == RuntimeState::Paused {
            return Ok(RunAction::new(RunActionKind::ResumeFromPause, false, None));
        }
        Ok(RunAction::new(
            RunActionKind::RejectRunning,
            false,
            Some(format!(
                "team {target_team_name:?} already running on session {target_session_id:?}; use interact"
            )),
        ))
    }
}

/// team-dispatch 插件:注册 `team-dispatch` seam。
pub struct TeamDispatchPlugin;

impl Plugin for TeamDispatchPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-team-dispatch"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![TEAM_DISPATCH]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let dispatch: Arc<dyn TeamDispatch> = Arc::new(DispatchDecider);
        Ok(vec![ctx.register(TEAM_DISPATCH, dispatch)])
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::TEAM_DISPATCH;
    use ah_contracts::team_dispatch::{
        RunActionKind, RuntimeState, TEAM_DB_STATE_CLEANED, TEAM_DB_STATE_CREATED,
        TEAM_DB_STATE_PENDING_CREATE,
    };
    use ah_hub::plugin::DynPlugin;

    fn entry(session: &str, state: RuntimeState) -> PoolEntry {
        PoolEntry {
            current_session_id: session.to_string(),
            state,
        }
    }

    fn decide(
        team_in_db: bool,
        team_in_session: bool,
        pool: Option<&PoolEntry>,
        db_state: Option<&str>,
    ) -> Result<RunAction, DispatchError> {
        DispatchDecider.decide(
            team_in_db,
            team_in_session,
            pool,
            "sess-1",
            "team-a",
            db_state,
        )
    }

    #[test]
    fn create_when_nothing_exists() {
        let action = decide(false, false, None, None).expect("decide");
        assert_eq!(action.kind, RunActionKind::Create);
        assert!(action.require_spec, "全新团队必须带 spec");
    }

    #[test]
    fn recreate_for_pending_or_cleaned_db_state() {
        // DB 无行但 session 桶存在,pending_create → 可重建 CREATE。
        let action = decide(false, true, None, Some(TEAM_DB_STATE_PENDING_CREATE)).expect("decide");
        assert_eq!(action.kind, RunActionKind::Create);
        assert!(action.require_spec);
        // cleaned 同理。
        let action = decide(false, true, None, Some(TEAM_DB_STATE_CLEANED)).expect("decide");
        assert_eq!(action.kind, RunActionKind::Create);
        // 即使池里有条目也允许重建(先于不一致检查)。
        let action = decide(
            false,
            true,
            Some(&entry("sess-1", RuntimeState::Running)),
            Some(TEAM_DB_STATE_PENDING_CREATE),
        )
        .expect("decide");
        assert_eq!(action.kind, RunActionKind::Create);
    }

    #[test]
    fn reject_orphaned_when_bucket_without_db_row() {
        let action = decide(false, true, None, Some(TEAM_DB_STATE_CREATED)).expect("decide");
        assert_eq!(action.kind, RunActionKind::RejectOrphaned);
        assert!(!action.require_spec);
        assert!(action.reason.as_deref().unwrap_or("").contains("not in DB"));
        // 无 db_state 同样孤儿。
        let action = decide(false, true, None, None).expect("decide");
        assert_eq!(action.kind, RunActionKind::RejectOrphaned);
    }

    #[test]
    fn reject_inconsistent_when_pool_without_db_row() {
        let action = decide(
            false,
            false,
            Some(&entry("sess-1", RuntimeState::Running)),
            Some(TEAM_DB_STATE_CREATED),
        )
        .expect("decide");
        assert_eq!(action.kind, RunActionKind::RejectInconsistent);
        assert!(
            action
                .reason
                .as_deref()
                .unwrap_or("")
                .contains("present in pool")
        );
    }

    #[test]
    fn new_team_in_session_and_cold_recover() {
        // DB 有团队、session 无桶、无池 → 新团队进 session。
        let action = decide(true, false, None, None).expect("decide");
        assert_eq!(action.kind, RunActionKind::NewTeamInSession);
        assert!(!action.require_spec);
        // DB 有团队、session 有桶、无池 → 冷恢复。
        let action = decide(true, true, None, None).expect("decide");
        assert_eq!(action.kind, RunActionKind::ColdRecover);
        assert!(!action.require_spec);
    }

    #[test]
    fn resume_from_pause_and_reject_running() {
        let paused = decide(
            true,
            true,
            Some(&entry("sess-1", RuntimeState::Paused)),
            None,
        )
        .expect("decide");
        assert_eq!(paused.kind, RunActionKind::ResumeFromPause);
        let running = decide(
            true,
            true,
            Some(&entry("sess-1", RuntimeState::Running)),
            None,
        )
        .expect("decide");
        assert_eq!(running.kind, RunActionKind::RejectRunning);
        assert!(
            running
                .reason
                .as_deref()
                .unwrap_or("")
                .contains("already running")
        );
    }

    #[test]
    fn invariant_violation_cross_session_pool_entry() {
        // 池条目属于另一 session → 显式 Err(不变量违例)。
        let err = decide(
            true,
            true,
            Some(&entry("sess-other", RuntimeState::Running)),
            None,
        )
        .unwrap_err();
        assert!(err.0.contains("invariant violated"), "err: {}", err.0);
    }

    #[test]
    fn plugin_registers_dispatch() {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(TeamDispatchPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let dispatch = ctx
            .service::<dyn TeamDispatch>(&TEAM_DISPATCH)
            .expect("team-dispatch seam");
        let action = dispatch
            .decide(false, false, None, "s", "t", None)
            .expect("decide");
        assert_eq!(action.kind, RunActionKind::Create);
        drop(effects);
        assert!(!ctx.has_service(&TEAM_DISPATCH));
    }
}
