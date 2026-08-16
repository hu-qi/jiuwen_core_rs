//! # ah-plugins-team-pool
//!
//! Real team runtime pool (aligned with openjiuwen/agent_teams/runtime/pool.py):
//! in-process pool keyed by team_name with get/has/add-replace/remove/
//! teams_for_session/list_all_info read-only snapshots. The InteractGate
//! (gate.py) lives in ah-contracts as the shared concurrency primitive.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use ah_contracts::keys::TEAM_POOL;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::team_pool::{ActiveTeam, ActiveTeamInfo, PoolError, TeamRuntimePool};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 真实团队对象池:Mutex<HashMap<team_name, ActiveTeam>>。
pub struct MutexTeamPool {
    teams: Mutex<HashMap<String, ActiveTeam>>,
}

impl Default for MutexTeamPool {
    fn default() -> Self {
        Self::new()
    }
}

impl MutexTeamPool {
    pub fn new() -> Self {
        Self {
            teams: Mutex::new(HashMap::new()),
        }
    }
}

impl Seam for MutexTeamPool {}

impl TeamRuntimePool for MutexTeamPool {
    fn get(&self, team_name: &str) -> Option<ActiveTeam> {
        self.teams.lock().unwrap().get(team_name).cloned()
    }

    fn has_active(&self, team_name: &str) -> bool {
        self.teams.lock().unwrap().contains_key(team_name)
    }

    fn add(&self, entry: ActiveTeam) -> Result<(), PoolError> {
        // 同名替换(对齐 Python:add 替换既有条目)。
        self.teams
            .lock()
            .unwrap()
            .insert(entry.team_name.clone(), entry);
        Ok(())
    }

    fn remove(&self, team_name: &str) -> Option<ActiveTeam> {
        self.teams.lock().unwrap().remove(team_name)
    }

    fn list_team_names(&self) -> Vec<String> {
        self.teams.lock().unwrap().keys().cloned().collect()
    }

    fn teams_for_session(&self, session_id: &str) -> Vec<ActiveTeam> {
        self.teams
            .lock()
            .unwrap()
            .values()
            .filter(|team| team.current_session_id == session_id)
            .cloned()
            .collect()
    }

    fn list_all_info(&self) -> Vec<ActiveTeamInfo> {
        self.teams
            .lock()
            .unwrap()
            .values()
            .map(|entry| ActiveTeamInfo {
                team_name: entry.team_name.clone(),
                current_session_id: entry.current_session_id.clone(),
                state: entry.state,
                gate_closed: entry.gate.closed(),
            })
            .collect()
    }
}

/// team-pool 插件:注册 `team-pool` seam。
pub struct TeamPoolPlugin;

impl Plugin for TeamPoolPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-team-pool"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![TEAM_POOL]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let pool: Arc<dyn TeamRuntimePool> = Arc::new(MutexTeamPool::new());
        Ok(vec![ctx.register(TEAM_POOL, pool)])
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::TEAM_POOL;
    use ah_contracts::team_dispatch::RuntimeState;
    use ah_contracts::team_pool::{InteractGate, TeamRuntimePool};
    use ah_hub::plugin::DynPlugin;
    use std::thread;
    use std::time::Duration;

    fn team(name: &str, session: &str, state: RuntimeState) -> ActiveTeam {
        ActiveTeam {
            team_name: name.to_string(),
            current_session_id: session.to_string(),
            state,
            gate: Arc::new(InteractGate::new()),
        }
    }

    #[test]
    fn pool_add_get_has_remove_and_replace() {
        let pool = MutexTeamPool::new();
        assert!(!pool.has_active("team-a"));
        assert!(pool.get("team-a").is_none());
        pool.add(team("team-a", "s1", RuntimeState::Running))
            .expect("add");
        assert!(pool.has_active("team-a"));
        let entry = pool.get("team-a").expect("get");
        assert_eq!(entry.current_session_id, "s1");
        assert_eq!(entry.state, RuntimeState::Running);
        // 同名替换。
        pool.add(team("team-a", "s2", RuntimeState::Paused))
            .expect("replace");
        let replaced = pool.get("team-a").expect("get");
        assert_eq!(replaced.current_session_id, "s2");
        assert_eq!(replaced.state, RuntimeState::Paused);
        // remove。
        let removed = pool.remove("team-a").expect("remove");
        assert_eq!(removed.team_name, "team-a");
        assert!(!pool.has_active("team-a"));
        assert!(pool.remove("missing").is_none());
    }

    #[test]
    fn pool_lists_names_and_teams_for_session() {
        let pool = MutexTeamPool::new();
        pool.add(team("t1", "sess-x", RuntimeState::Running))
            .unwrap();
        pool.add(team("t2", "sess-x", RuntimeState::Running))
            .unwrap();
        pool.add(team("t3", "sess-y", RuntimeState::Paused))
            .unwrap();
        let mut names = pool.list_team_names();
        names.sort();
        assert_eq!(
            names,
            vec!["t1".to_string(), "t2".to_string(), "t3".to_string()]
        );
        let for_x = pool.teams_for_session("sess-x");
        assert_eq!(for_x.len(), 2);
        let info = pool.list_all_info();
        assert_eq!(info.len(), 3);
        let info_t3 = info.iter().find(|i| i.team_name == "t3").expect("t3");
        assert_eq!(info_t3.state, RuntimeState::Paused);
        assert!(!info_t3.gate_closed);
    }

    #[test]
    fn gate_admit_consume_and_closed_rejection() {
        let gate = InteractGate::new();
        assert!(!gate.closed());
        assert_eq!(gate.inflight(), 0);
        let ticket = gate.admit().expect("admit");
        assert_eq!(gate.inflight(), 1);
        let ticket2 = gate.admit().expect("admit 2");
        assert_eq!(gate.inflight(), 2);
        gate.consume_done(ticket);
        assert_eq!(gate.inflight(), 1);
        // 全部消费后关闭不阻塞。
        gate.consume_done(ticket2);
        gate.close_and_drain();
        assert!(gate.closed());
        assert_eq!(gate.inflight(), 0);
        assert!(gate.admit().is_none(), "关闭后拒绝 admit");
    }

    #[test]
    fn gate_ignores_foreign_ticket_and_resets() {
        let gate_a = InteractGate::new();
        let gate_b = InteractGate::new();
        let ticket_a = gate_a.admit().expect("admit a");
        let foreign = gate_b.admit().expect("admit b");
        gate_a.consume_done(foreign.clone());
        assert_eq!(gate_a.inflight(), 1, "外门禁票证忽略");
        gate_a.consume_done(ticket_a);
        assert_eq!(gate_a.inflight(), 0);
        // 归还 foreign 票证到 gate_b,避免残留 in-flight。
        gate_b.consume_done(foreign);
        assert_eq!(gate_b.inflight(), 0);
        // reset 重开。
        let ticket_b = gate_b.admit().expect("admit b");
        gate_b.consume_done(ticket_b);
        gate_b.close_and_drain();
        assert!(gate_b.closed());
        assert!(gate_b.admit().is_none());
        gate_b.reset();
        assert!(!gate_b.closed());
        assert_eq!(gate_b.inflight(), 0);
        assert!(gate_b.admit().is_some(), "reset 后重新放行");
    }

    #[test]
    fn gate_close_and_drain_waits_for_inflight_consumption() {
        let gate = Arc::new(InteractGate::new());
        let ticket = gate.admit().expect("admit");
        // 在另一线程消费,主线程 close_and_drain 应阻塞直到消费完成。
        let gate_consumer = gate.clone();
        let consumer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            gate_consumer.consume_done(ticket);
        });
        gate.close_and_drain();
        consumer.join().expect("join");
        assert!(gate.closed());
        assert_eq!(gate.inflight(), 0);
    }

    #[test]
    fn plugin_registers_pool() {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(TeamPoolPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let pool = ctx
            .service::<dyn TeamRuntimePool>(&TEAM_POOL)
            .expect("team-pool seam");
        pool.add(team("t", "s", RuntimeState::Running))
            .expect("add");
        assert!(pool.has_active("t"));
        drop(effects);
        assert!(!ctx.has_service(&TEAM_POOL));
    }
}
