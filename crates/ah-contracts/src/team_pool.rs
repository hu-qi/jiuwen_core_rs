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

// ---------------------------------------------------------------------------
// 后台任务控制器(对齐 runtime/background_task_controller.py)
// ---------------------------------------------------------------------------

/// 一次存活 swarmflow 运行的控制句柄(对齐 SwarmflowRunHandle)。
///
/// 引擎副作用(abort_sessions/cancel/relaunch)由持有方经闭包注入,控制器只做
/// 注册表 + 暂停/恢复编排 —— 对齐 Python 的 handle 携带 backend/native/relaunch。
#[derive(Clone)]
pub struct SwarmflowRunHandle {
    pub task_id: String,
    /// 设置引擎 abort_event(经闭包执行)。
    pub abort: std::sync::Arc<dyn Fn() + Send + Sync>,
    /// 中止该 run 的会话(经闭包执行;best-effort)。
    pub abort_sessions: std::sync::Arc<dyn Fn() + Send + Sync>,
    /// 取消顶层 run task(best-effort)。
    pub cancel: std::sync::Arc<dyn Fn() + Send + Sync>,
    /// 以相同输入重新启动 run_background。
    pub relaunch: std::sync::Arc<dyn Fn() + Send + Sync>,
}

impl SwarmflowRunHandle {
    /// 以闭包构造句柄。
    pub fn new(
        task_id: impl Into<String>,
        abort: std::sync::Arc<dyn Fn() + Send + Sync>,
        abort_sessions: std::sync::Arc<dyn Fn() + Send + Sync>,
        cancel: std::sync::Arc<dyn Fn() + Send + Sync>,
        relaunch: std::sync::Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self {
            task_id: task_id.into(),
            abort,
            abort_sessions,
            cancel,
            relaunch,
        }
    }
}

/// 后台任务控制面(对齐 BackgroundTaskController)。
///
/// 注册表 + 控制面:run 启动时注册句柄、完成时注销;`pause`/`resume` 作用于
/// 已注册句柄。无匹配 run 时 pause/resume 返回 false(no-op)。
#[derive(Default)]
pub struct BackgroundTaskController {
    state: Mutex<BackgroundTaskState>,
}

#[derive(Default)]
struct BackgroundTaskState {
    active: std::collections::HashMap<String, SwarmflowRunHandle>,
    paused: std::collections::HashMap<String, std::sync::Arc<dyn Fn() + Send + Sync>>,
}

impl BackgroundTaskController {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册一次运行的控制句柄(启动时调用)。
    pub fn register(&self, handle: SwarmflowRunHandle) {
        self.state
            .lock()
            .unwrap()
            .active
            .insert(handle.task_id.clone(), handle);
    }

    /// 注销句柄(launcher finally 调用;幂等)。
    pub fn deregister(&self, task_id: &str) {
        self.state.lock().unwrap().active.remove(task_id);
    }

    /// 暂停全部活跃运行;无活跃返回 false(对齐 pause 三步:abort → abort_sessions
    /// → cancel,再登记 relaunch 到 paused 并移出 active)。
    pub fn pause(&self) -> bool {
        let mut state = self.state.lock().unwrap();
        if state.active.is_empty() {
            return false;
        }
        let handles: Vec<SwarmflowRunHandle> = state.active.values().cloned().collect();
        for handle in handles {
            (handle.abort)();
            (handle.abort_sessions)();
            (handle.cancel)();
            state.paused.insert(handle.task_id.clone(), handle.relaunch);
            state.active.remove(&handle.task_id);
        }
        true
    }

    /// 恢复全部暂停运行(重放 relaunch);无暂停返回 false。
    pub fn resume(&self) -> bool {
        let mut state = self.state.lock().unwrap();
        if state.paused.is_empty() {
            return false;
        }
        let relaunches: Vec<(String, std::sync::Arc<dyn Fn() + Send + Sync>)> =
            state.paused.drain().collect();
        for (task_id, relaunch) in relaunches {
            (relaunch)();
            state.active.remove(&task_id);
        }
        true
    }

    /// 是否处于暂停态(有待恢复的 run)。
    pub fn is_paused(&self) -> bool {
        !self.state.lock().unwrap().paused.is_empty()
    }

    /// 活跃 run 数。
    pub fn active_count(&self) -> usize {
        self.state.lock().unwrap().active.len()
    }

    /// 暂停 run 数。
    pub fn paused_count(&self) -> usize {
        self.state.lock().unwrap().paused.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn handle(
        task_id: &str,
        abort: std::sync::Arc<dyn Fn() + Send + Sync>,
        relaunch: std::sync::Arc<dyn Fn() + Send + Sync>,
    ) -> SwarmflowRunHandle {
        let noop = std::sync::Arc::new(|| {}) as std::sync::Arc<dyn Fn() + Send + Sync>;
        SwarmflowRunHandle::new(task_id, abort, noop.clone(), noop, relaunch)
    }

    #[test]
    fn controller_register_pause_resume() {
        let ctl = BackgroundTaskController::new();
        let aborted = Arc::new(AtomicUsize::new(0));
        let resumed = Arc::new(AtomicUsize::new(0));
        let a = aborted.clone();
        let r = resumed.clone();
        ctl.register(handle(
            "run1",
            Arc::new(move || {
                a.fetch_add(1, Ordering::SeqCst);
            }),
            Arc::new(move || {
                r.fetch_add(1, Ordering::SeqCst);
            }),
        ));
        assert_eq!(ctl.active_count(), 1);
        assert!(!ctl.is_paused());

        // pause:三步执行(abort 至少一次),active → paused。
        assert!(ctl.pause());
        assert_eq!(aborted.load(Ordering::SeqCst), 1);
        assert_eq!(ctl.active_count(), 0);
        assert!(ctl.is_paused());
        assert_eq!(ctl.paused_count(), 1);

        // 无活跃 → pause false;无暂停 → resume false。
        assert!(!ctl.pause());
        // resume:重放 relaunch。
        assert!(ctl.resume());
        assert_eq!(resumed.load(Ordering::SeqCst), 1);
        assert!(!ctl.is_paused());
        assert!(!ctl.resume());
    }

    #[test]
    fn controller_deregister_is_idempotent() {
        let ctl = BackgroundTaskController::new();
        ctl.register(handle("run2", Arc::new(|| {}), Arc::new(|| {})));
        ctl.deregister("run2");
        ctl.deregister("run2"); // 幂等。
        assert_eq!(ctl.active_count(), 0);
        assert!(!ctl.pause(), "no active → pause no-op");
    }

    #[test]
    fn gate_admit_consume_close_reset() {
        let gate = InteractGate::new();
        let ticket = gate.admit().expect("admit open");
        assert_eq!(gate.inflight(), 1);
        gate.consume_done(ticket);
        assert_eq!(gate.inflight(), 0);

        gate.close_and_drain();
        assert!(gate.closed());
        assert!(gate.admit().is_none(), "closed gate rejects admit");

        gate.reset();
        assert!(!gate.closed());
        let _ = gate.admit().expect("admit after reset");
    }
}
