//! team-pool seam:团队运行对象池 + 并发门禁(对齐 openjiuwen/agent_teams/runtime/pool.py + gate.py)。
//!
//! - `TeamRuntimePool`:以 team_name 为键的进程内对象池(get/has/add 替换/remove/
//!   list_team_names/teams_for_session/list_all_info 只读快照);
//! - `InteractGate`:Run/Interact 并发门禁(admit 票证 / consume_done / close_and_drain / reset),
//!   每个 ActiveTeam 持有一个;关闭后拒绝新 admit,close 后等 in-flight 消费完再 drain。
//!
//! 契约零实现:池存储与门禁状态机由插件提供(如 ah-plugins-team-pool)。

use std::sync::Arc;
use std::sync::Condvar;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::seam::Seam;
use crate::team_dispatch::RuntimeState;

static GATE_SEQ: AtomicU64 = AtomicU64::new(1);

/// 并发门禁内部状态。
#[derive(Debug, Default)]
struct GateState {
    closed: bool,
    inflight: u32,
    drained: bool,
}

/// Run/Interact 并发门禁:admit → 消费计数;close_and_drain 关闭后等 in-flight 清零。
#[derive(Debug)]
pub struct InteractGate {
    id: u64,
    state: Mutex<GateState>,
    drain_cv: Condvar,
}

impl Default for InteractGate {
    fn default() -> Self {
        Self::new()
    }
}

impl InteractGate {
    /// 新建门禁(唯一 id 用于票证归属)。
    pub fn new() -> Self {
        Self {
            id: GATE_SEQ.fetch_add(1, Ordering::SeqCst),
            state: Mutex::new(GateState {
                closed: false,
                inflight: 0,
                drained: true,
            }),
            drain_cv: Condvar::new(),
        }
    }

    /// 门禁唯一 id。
    pub fn id(&self) -> u64 {
        self.id
    }

    /// 是否已关闭(拒绝新 admit)。
    pub fn closed(&self) -> bool {
        self.state.lock().unwrap().closed
    }

    /// 当前 in-flight(已 admit 未 consume)计数。
    pub fn inflight(&self) -> u32 {
        self.state.lock().unwrap().inflight
    }

    /// 尝试 admit:关闭返回 None;否则 inflight+1 并返回票证。
    pub fn admit(&self) -> Option<AdmissionTicket> {
        let mut state = self.state.lock().unwrap();
        if state.closed {
            return None;
        }
        state.inflight += 1;
        state.drained = false;
        Some(AdmissionTicket { gate_id: self.id })
    }

    /// 标记一个票证已消费(票证不属于本门禁则忽略)。
    pub fn consume_done(&self, ticket: AdmissionTicket) {
        if ticket.gate_id != self.id {
            return;
        }
        let mut state = self.state.lock().unwrap();
        if state.inflight == 0 {
            return;
        }
        state.inflight -= 1;
        if state.inflight == 0 {
            state.drained = true;
            self.drain_cv.notify_all();
        }
    }

    /// 关闭门禁并等待 in-flight 清零(阻塞)。
    pub fn close_and_drain(&self) {
        let mut state = self.state.lock().unwrap();
        state.closed = true;
        if state.inflight == 0 {
            state.drained = true;
            return;
        }
        while !state.drained {
            state = self.drain_cv.wait(state).unwrap();
        }
    }

    /// 重开门禁(新 run cycle):清 closed 与 inflight。
    pub fn reset(&self) {
        let mut state = self.state.lock().unwrap();
        state.closed = false;
        state.inflight = 0;
        state.drained = true;
        self.drain_cv.notify_all();
    }
}

/// admit 票证(不透明;必须交回 consume_done)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionTicket {
    pub(crate) gate_id: u64,
}

/// 池中的活跃团队条目。
#[derive(Debug, Clone)]
pub struct ActiveTeam {
    pub team_name: String,
    pub current_session_id: String,
    pub state: RuntimeState,
    pub gate: Arc<InteractGate>,
}

/// 池条目的只读快照(不暴露 gate 引用)。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ActiveTeamInfo {
    pub team_name: String,
    pub current_session_id: String,
    pub state: RuntimeState,
    pub gate_closed: bool,
}

/// 池错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolError(pub String);

impl core::fmt::Display for PoolError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PoolError {}

/// 团队对象池 Seam(Service Definition):以 team_name 为键。
pub trait TeamRuntimePool: Seam {
    /// 取条目(共享 gate 的克隆)。
    fn get(&self, team_name: &str) -> Option<ActiveTeam>;

    /// 池中是否有该团队。
    fn has_active(&self, team_name: &str) -> bool;

    /// 注册团队(同名替换)。
    fn add(&self, entry: ActiveTeam) -> Result<(), PoolError>;

    /// 移除团队并返回条目。
    fn remove(&self, team_name: &str) -> Option<ActiveTeam>;

    /// 池中团队名快照。
    fn list_team_names(&self) -> Vec<String>;

    /// 绑定到指定 session 的团队。
    fn teams_for_session(&self, session_id: &str) -> Vec<ActiveTeam>;

    /// 全部团队只读快照。
    fn list_all_info(&self) -> Vec<ActiveTeamInfo>;
}
