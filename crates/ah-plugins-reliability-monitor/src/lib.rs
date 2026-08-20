//! # ah-plugins-reliability-monitor
//!
//! Reliability monitoring framework (1:1 with
//! openjiuwen/agent_teams/reliability/{monitor,remediation,reporter}.py):
//! - `RemediationPolicy`: severity → ordered remediation actions (default
//!   tiered, leader-first; aligned with remediation/policy.py);
//! - `LocalAutoRemediator`: reversible local self-steering, rate-limited by
//!   restart intensity — a sliding window counted on a `VecDeque<f64>`
//!   implemented in this crate (plugin isolation; no burst-crate dependency),
//!   clock injected for determinism (aligned with remediation/local.py);
//! - `ReliabilityMonitor`: fan a signal out to every detector, route each
//!   anomaly by policy (report / observe-only), tolerate a misbehaving
//!   detector so one bad detector never blocks the others (aligned with
//!   monitor.py);
//! - `AnomalyReporter` trait + `LocalAnomalyReporter`: in-process anomaly
//!   sink with bind-before-report no-op semantics (aligned with reporter.py).
//!
//! Concrete detectors live in sibling plugins (e.g. ah-plugins-reliability-burst);
//! this crate only consumes the `ah_contracts::reliability_detectors::Detector`
//! seam and never imports other ah-plugins-* crates.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;

use ah_contracts::keys::{
    RELIABILITY_FACTORY, RELIABILITY_HANDLER, RELIABILITY_MONITOR, RELIABILITY_POLICY,
    RELIABILITY_RAIL, RELIABILITY_REPORTER,
};
use ah_contracts::prelude::Effect;
use ah_contracts::reliability_config::{RemediationAction, RemediationPolicyConfig};
use ah_contracts::reliability_detectors::{Anomaly, AnomalyKind, Detector, Severity, Signal};
use ah_contracts::reliability_rail::PolicyView;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

mod rail;
pub use rail::{LeaderReliabilityHandler, MemberReliabilityRail, ReliabilityAssembly};

/// `AnomalyKind` 的 snake_case 标识(与契约 serde rename_all 一致;steer 文案使用)。
fn kind_value(kind: AnomalyKind) -> &'static str {
    match kind {
        AnomalyKind::ToolErrorRate => "tool_error_rate",
        AnomalyKind::RepeatToolCall => "repeat_tool_call",
        AnomalyKind::ToolCallLoop => "tool_call_loop",
        AnomalyKind::ModelError => "model_error",
        AnomalyKind::OutputTooLong => "output_too_long",
        AnomalyKind::ThinkingTooLong => "thinking_too_long",
        AnomalyKind::FrequentCompaction => "frequent_compaction",
        AnomalyKind::PingPong => "ping_pong",
    }
}

/// 真实系统时钟(秒),作为 `LocalAutoRemediator` 的默认 now。
fn real_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

// --- remediation policy ---

/// 严重度 → 有序修复动作(默认分层、leader-first;对齐 remediation/policy.py)。
///
/// 默认分层:LOW 仅观察 / MEDIUM 上报 leader(LLM 决策)/ HIGH 先本地自纠偏
/// 再上报 / CRITICAL 自纠偏后升级用户。LOCAL_STEER 是唯一自动化动作且严格可逆
/// (受重启强度限流);破坏性动作(停成员/取消任务/新建)刻意缺席——那是 leader
/// 通过既有团队工具的决定。
pub struct RemediationPolicy {
    severity_actions: std::collections::BTreeMap<Severity, Vec<RemediationAction>>,
}

impl RemediationPolicy {
    /// 从配置构建;`RemediationPolicyConfig::default()` 即上面的分层策略。
    pub fn new(config: &RemediationPolicyConfig) -> Self {
        Self {
            severity_actions: config.severity_actions.clone(),
        }
    }

    /// 返回 `severity` 配置的修复动作(未配置返回空列表,对齐 Python `dict.get(severity, [])`)。
    pub fn actions_for(&self, severity: Severity) -> Vec<RemediationAction> {
        self.severity_actions
            .get(&severity)
            .cloned()
            .unwrap_or_default()
    }
}

impl Seam for RemediationPolicy {}

// --- anomaly reporter trait ---

/// 异常上报 Seam(对齐 reporter.py 的 `AnomalyReporter` Protocol)。
///
/// 成员侧 monitor 与(leader 进程)修复管线之间的接缝;实现方把异常送入
/// 上报管线(进程内 sink 或跨进程事件)。
pub trait AnomalyReporter: Send + Sync {
    /// 上报一条检测到的异常。
    fn report(&self, anomaly: &Anomaly);
}

// --- local in-process reporter ---

/// 进程内异常汇(leader 自监控用;对齐 reporter.py 的 `LocalAnomalyReporter`)。
///
/// leader 自身的异常不能走消息总线发布——messager 的自过滤(kernel 丢弃
/// `sender_id` 等于本地成员的事件)会把它们吞掉;因此本上报器把异常直送本地
/// sink(leader 侧 ReliabilityHandler),在 dispatcher 构建完成后绑定一次。
/// 绑定前为 no-op(Python 侧打一条 warning;本 crate 无日志依赖,以文档说明),
/// 保证接线前产出的异常有明确语义,而非悄悄丢失。
/// 本地 sink 类型:接收一条异常(绑定前为 None)。
type AnomalySink = Arc<dyn Fn(&Anomaly) + Send + Sync>;

pub struct LocalAnomalyReporter {
    sink: Mutex<Option<AnomalySink>>,
}

impl LocalAnomalyReporter {
    /// 创建未绑定的上报器(no-op 直到 `Self::bind`)。
    pub fn new() -> Self {
        Self {
            sink: Mutex::new(None),
        }
    }

    /// 绑定本地 sink(路由异常到修复 handler);重复绑定覆盖旧 sink。
    pub fn bind(&self, sink: AnomalySink) {
        *self.sink.lock().unwrap() = Some(sink);
    }
}

impl Default for LocalAnomalyReporter {
    fn default() -> Self {
        Self::new()
    }
}

impl AnomalyReporter for LocalAnomalyReporter {
    fn report(&self, anomaly: &Anomaly) {
        if let Some(sink) = self.sink.lock().unwrap().as_ref() {
            sink(anomaly);
        }
    }
}

impl Seam for LocalAnomalyReporter {}

// --- restart-intensity sliding window ---

/// 重启强度滑动窗口:记录 `period_seconds` 内的修复时刻(自实现,插件隔离)。
///
/// 事件时间戳保存在 `VecDeque<f64>`,每次操作先逐出早于 `ts - period_seconds`
/// 的事件(对齐 Python `SlidingWindowCounter._evict`);时间戳由调用方显式传入,
/// 无内部时钟,因此完全确定、可测试。
struct RestartIntensityWindow {
    period_seconds: f64,
    events: VecDeque<f64>,
}

impl RestartIntensityWindow {
    fn new(period_seconds: f64) -> Self {
        Self {
            period_seconds,
            events: VecDeque::new(),
        }
    }

    /// 记录一次发生在 `ts` 的修复并返回窗口内计数(含本次)。
    fn add(&mut self, ts: f64) -> usize {
        self.events.push_back(ts);
        self.evict(ts);
        self.events.len()
    }

    /// 清空所有记录。
    fn reset(&mut self) {
        self.events.clear();
    }

    /// 逐出早于 `ts - period_seconds` 的事件。
    fn evict(&mut self, ts: f64) {
        let cutoff = ts - self.period_seconds;
        while let Some(&first) = self.events.front() {
            if first < cutoff {
                self.events.pop_front();
            } else {
                break;
            }
        }
    }
}

// --- local automated remediation ---

/// 本地自动修复:可逆自纠偏 + 重启强度限流(对齐 remediation/local.py)。
///
/// 常驻成员进程、由 rail 驱动:当策略对某条异常要求 `LocalSteer` 时,返回一条
/// 非破坏性的纠偏消息供 rail 推入纠偏队列——但只在重启强度预算内
/// (`intensity` 次 / `period_seconds` 秒,滑动窗口计数),自动化纠偏永不变成风暴。
/// 超出预算返回 `None`,完全交给 leader / 用户路径。这里从不执行破坏性动作。
pub struct LocalAutoRemediator {
    policy: Arc<RemediationPolicy>,
    intensity: u64,
    now: Box<dyn Fn() -> f64 + Send + Sync>,
    window: Mutex<RestartIntensityWindow>,
}

impl LocalAutoRemediator {
    /// 以注入的时钟构建(Python 默认 intensity=5 / period_seconds=60.0;now 注入保证可测)。
    pub fn new(
        policy: Arc<RemediationPolicy>,
        intensity: u64,
        period_seconds: f64,
        now: Box<dyn Fn() -> f64 + Send + Sync>,
    ) -> Self {
        Self {
            policy,
            intensity,
            now,
            window: Mutex::new(RestartIntensityWindow::new(period_seconds)),
        }
    }

    /// 以真实系统时钟 + 默认预算(5 次 / 60 秒)构建。
    pub fn with_defaults(policy: Arc<RemediationPolicy>) -> Self {
        Self::new(policy, 5, 60.0, Box::new(real_now))
    }

    /// 返回自纠偏消息,或 `None`(策略不含 `LocalSteer` / 超出强度预算)。
    ///
    /// 窗口计数在超预算时仍会记录本次(Python `add` 先记录再判断),
    /// 保证预算窗口内的实际发生次数是真实计数。
    pub fn steer_message(&self, anomaly: &Anomaly) -> Option<String> {
        if !self
            .policy
            .actions_for(anomaly.severity)
            .contains(&RemediationAction::LocalSteer)
        {
            return None;
        }
        let ts = (self.now)();
        let mut window = self.window.lock().unwrap();
        let used = window.add(ts);
        if used > self.intensity as usize {
            return None;
        }
        // i18n 键渲染(此处简化:按 `reliability.steer_self_correct` 键的
        // kind/summary 形参格式化,不引入 i18n 依赖)。
        Some(format!(
            "reliability.steer_self_correct kind={} summary={}",
            kind_value(anomaly.kind),
            anomaly.summary
        ))
    }

    /// 清空重启强度窗口(新一轮)。
    pub fn reset(&self) {
        self.window.lock().unwrap().reset();
    }
}

impl Seam for LocalAutoRemediator {}

// --- reliability monitor ---

/// 成员级检测聚合器(对齐 monitor.py 的 `ReliabilityMonitor`)。
///
/// 常驻成员进程:检测在信号产生处运行。`feed` 把信号扇出到每个检测器,并把
/// 产出的异常按策略路由——REPORT_LEADER / ESCALATE_USER → reporter;
/// OBSERVE_ONLY → 仅日志(见 `ReliabilityMonitor::route`);LOCAL_STEER →
/// 交给调用方对返回异常做可逆本地纠偏。产出的异常全部返回,供 rail 应用
/// LocalAutoRemediator。单个检测器 observe 抛错(panic)被捕获并跳过,一个
/// 坏检测器不阻塞其余检测器。
pub struct ReliabilityMonitor {
    detectors: Vec<Arc<dyn Detector>>,
    reporter: Arc<dyn AnomalyReporter>,
    policy: Arc<RemediationPolicy>,
}

impl ReliabilityMonitor {
    pub fn new(
        detectors: Vec<Arc<dyn Detector>>,
        reporter: Arc<dyn AnomalyReporter>,
        policy: Arc<RemediationPolicy>,
    ) -> Self {
        Self {
            detectors,
            reporter,
            policy,
        }
    }

    /// 把所有检测器跑一遍,返回产出的异常(含策略路由副作用)。
    pub fn feed(&self, signal: &Signal) -> Vec<Anomaly> {
        let mut produced = Vec::new();
        for detector in &self.detectors {
            // 容忍坏检测器:observe 的 panic 被捕获并跳过(对齐 Python 的
            // try/except + continue),一个坏检测器不阻塞其余检测器。
            let anomaly =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| detector.observe(signal)))
                    .ok()
                    .flatten();
            let Some(anomaly) = anomaly else {
                continue;
            };
            self.route(&anomaly);
            produced.push(anomaly);
        }
        produced
    }

    /// 按策略路由一条异常(上报,或 OBSERVE_ONLY 仅日志)。
    fn route(&self, anomaly: &Anomaly) {
        let actions = self.policy.actions_for(anomaly.severity);
        // REPORT_LEADER / ESCALATE_USER → 上报(leader / 用户管线)。
        if actions.contains(&RemediationAction::ReportLeader)
            || actions.contains(&RemediationAction::EscalateUser)
        {
            self.reporter.report(anomaly);
        }
        // 若命中上报分支,副作用已产生;其余情形(仅 OBSERVE_ONLY / LOCAL_STEER /
        // 未配置)无上报副作用:
        // - OBSERVE_ONLY → Python 打一条 info 日志;本 crate 无日志依赖,
        //   注释说明即可、不落日志库(行为由测试锁定,非静默吞掉)。
        // - 仅 LOCAL_STEER / 未配置 → 无副作用;返回的异常由调用方
        //   经 LocalAutoRemediator 做可逆本地纠偏。
    }

    /// 重置所有检测器(新一轮)。
    pub fn reset(&self) {
        for detector in &self.detectors {
            detector.reset();
        }
    }
}

impl Seam for ReliabilityMonitor {}

// --- plugin registration ---

/// reliability-monitor 插件:注册默认策略 + 本地上报器 + 空检测器 monitor(可查)
/// + rail / handler / factory 装配 seam。
///
/// 检测器由各检测插件提供、monitor 由团队装配侧(rail/factory)按成员构建;
/// 此处注册框架级默认服务,消费方按契约键取回。
pub struct ReliabilityMonitorPlugin;

impl Plugin for ReliabilityMonitorPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-reliability-monitor"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![
            RELIABILITY_MONITOR,
            RELIABILITY_POLICY,
            RELIABILITY_REPORTER,
            RELIABILITY_RAIL,
            RELIABILITY_HANDLER,
            RELIABILITY_FACTORY,
        ]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let policy = Arc::new(RemediationPolicy::new(&RemediationPolicyConfig::default()));
        let reporter = Arc::new(LocalAnomalyReporter::new());
        // monitor 持 `Arc<dyn AnomalyReporter>`(unsized 强转),reporter 以
        // 具体类型注册,消费方可取回具体实例调用 `bind` 绑定本地 sink。
        let monitor = Arc::new(ReliabilityMonitor::new(
            Vec::new(),
            reporter.clone(),
            policy.clone(),
        ));
        // rail/handler/factory:默认装配(空检测器 monitor + 默认策略),供
        // 团队装配侧取回按成员扩展;handler 持策略视图做路由决策 + 格式化。
        let rail = Arc::new(MemberReliabilityRail::new(
            monitor.clone(),
            "",
            None,
            Some(reporter.clone()),
        ));
        let handler = Arc::new(LeaderReliabilityHandler::new(PolicyView::from_config(
            &RemediationPolicyConfig::default(),
        )));
        let factory = Arc::new(ReliabilityAssembly);
        Ok(vec![
            ctx.register(RELIABILITY_POLICY, policy),
            ctx.register(RELIABILITY_REPORTER, reporter),
            ctx.register(RELIABILITY_MONITOR, monitor),
            ctx.register(RELIABILITY_RAIL, rail),
            ctx.register(RELIABILITY_HANDLER, handler),
            ctx.register(RELIABILITY_FACTORY, factory),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::reliability_config::RemediationAction::*;
    use ah_contracts::reliability_detectors::SignalKind;
    use ah_hub::plugin::DynPlugin;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn anomaly(detector: &str, severity: Severity, summary: &str) -> Anomaly {
        Anomaly {
            detector: detector.to_string(),
            kind: AnomalyKind::ModelError,
            severity,
            member_name: "m".to_string(),
            summary: summary.to_string(),
            evidence: serde_json::Map::new(),
            peer_member: None,
        }
    }

    fn signal(member: &str) -> Signal {
        Signal::new(SignalKind::ToolException, member)
    }

    fn policy() -> Arc<RemediationPolicy> {
        Arc::new(RemediationPolicy::new(&RemediationPolicyConfig::default()))
    }

    /// 可配置假检测器:固定产出指定异常(或 None),统计 observe/reset,可注入 panic。
    struct FakeDetector {
        name: String,
        produce: Option<Anomaly>,
        panic_on_observe: bool,
        observe_count: AtomicUsize,
        reset_count: AtomicUsize,
    }

    impl FakeDetector {
        fn firing(name: &str, severity: Severity, summary: &str) -> Self {
            Self {
                name: name.to_string(),
                produce: Some(anomaly(name, severity, summary)),
                panic_on_observe: false,
                observe_count: AtomicUsize::new(0),
                reset_count: AtomicUsize::new(0),
            }
        }

        fn silent(name: &str) -> Self {
            Self {
                name: name.to_string(),
                produce: None,
                panic_on_observe: false,
                observe_count: AtomicUsize::new(0),
                reset_count: AtomicUsize::new(0),
            }
        }

        fn panicking(name: &str) -> Self {
            Self {
                name: name.to_string(),
                produce: None,
                panic_on_observe: true,
                observe_count: AtomicUsize::new(0),
                reset_count: AtomicUsize::new(0),
            }
        }
    }

    impl Seam for FakeDetector {}

    impl Detector for FakeDetector {
        fn name(&self) -> &str {
            &self.name
        }

        fn observe(&self, _signal: &Signal) -> Option<Anomaly> {
            self.observe_count.fetch_add(1, Ordering::SeqCst);
            if self.panic_on_observe {
                panic!("{} observe exploded", self.name);
            }
            self.produce.clone()
        }

        fn reset(&self) {
            self.reset_count.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// 计数上报器:统计 report 次数并保留最近一条异常。
    struct CountingReporter {
        calls: AtomicUsize,
        last: Mutex<Option<Anomaly>>,
    }

    impl CountingReporter {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
                last: Mutex::new(None),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl Seam for CountingReporter {}

    impl AnomalyReporter for CountingReporter {
        fn report(&self, anomaly: &Anomaly) {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.last.lock().unwrap() = Some(anomaly.clone());
        }
    }

    // (a) policy 默认分层:4 个 severity 各映射到预期动作。
    #[test]
    fn policy_default_tiered() {
        let policy = RemediationPolicy::new(&RemediationPolicyConfig::default());
        assert_eq!(policy.actions_for(Severity::Low), vec![ObserveOnly]);
        assert_eq!(policy.actions_for(Severity::Medium), vec![ReportLeader]);
        assert_eq!(
            policy.actions_for(Severity::High),
            vec![LocalSteer, ReportLeader]
        );
        assert_eq!(
            policy.actions_for(Severity::Critical),
            vec![LocalSteer, EscalateUser]
        );
    }

    // (e) LocalAnomalyReporter:未绑定 no-op,绑定后直送 sink(保留骨架测试并扩展)。
    #[test]
    fn local_reporter_binds_and_routes() {
        let reporter = LocalAnomalyReporter::new();
        let anomaly = anomaly("d", Severity::Medium, "s");
        // unbound -> no-op(不 panic、不计数)
        reporter.report(&anomaly);
        let seen = Arc::new(AtomicUsize::new(0));
        let captured = Arc::new(Mutex::new(None::<Anomaly>));
        let seen2 = seen.clone();
        let captured2 = captured.clone();
        reporter.bind(Arc::new(move |a| {
            seen2.fetch_add(1, Ordering::SeqCst);
            *captured2.lock().unwrap() = Some(a.clone());
        }));
        // bound -> 每次 report 都直送 sink,且携带同一条异常。
        reporter.report(&anomaly);
        reporter.report(&anomaly);
        assert_eq!(seen.load(Ordering::SeqCst), 2);
        let guard = captured.lock().unwrap();
        let last = guard.as_ref().expect("sink 收到异常");
        assert_eq!(last.detector, "d");
        assert_eq!(last.severity, Severity::Medium);
    }

    // (b) monitor.feed 把所有检测器跑一遍并返回全部异常。
    #[test]
    fn monitor_feed_runs_all_detectors_and_returns_all_anomalies() {
        let d1 = Arc::new(FakeDetector::firing("d1", Severity::Low, "s1"));
        let d2 = Arc::new(FakeDetector::firing("d2", Severity::Medium, "s2"));
        let d3 = Arc::new(FakeDetector::firing("d3", Severity::High, "s3"));
        let reporter = Arc::new(CountingReporter::new());
        let monitor = ReliabilityMonitor::new(
            vec![d1.clone(), d2.clone(), d3.clone()],
            reporter.clone(),
            policy(),
        );
        let out = monitor.feed(&signal("m"));
        assert_eq!(out.len(), 3, "全部异常都被返回");
        assert_eq!(out[0].detector, "d1");
        assert_eq!(out[1].detector, "d2");
        assert_eq!(out[2].detector, "d3");
        assert_eq!(
            d1.observe_count.load(Ordering::SeqCst),
            1,
            "每个检测器都被跑一遍"
        );
        assert_eq!(d2.observe_count.load(Ordering::SeqCst), 1);
        assert_eq!(d3.observe_count.load(Ordering::SeqCst), 1);
        // Medium/High 路由上报,Low(OBSERVE_ONLY)不上报。
        assert_eq!(reporter.calls(), 2);
    }

    // (c) monitor 路由:Medium → reporter 收到 1 次;Low(OBSERVE_ONLY) → 不收。
    #[test]
    fn monitor_routes_medium_to_reporter_low_observe_only() {
        let reporter = Arc::new(CountingReporter::new());
        let medium = Arc::new(FakeDetector::firing("medium", Severity::Medium, "m"));
        let monitor = ReliabilityMonitor::new(vec![medium], reporter.clone(), policy());
        let out = monitor.feed(&signal("m"));
        assert_eq!(out.len(), 1);
        assert_eq!(reporter.calls(), 1, "Medium → reporter 收到 1 次");
        assert_eq!(
            reporter
                .last
                .lock()
                .unwrap()
                .as_ref()
                .expect("收到异常")
                .detector,
            "medium"
        );

        let reporter = Arc::new(CountingReporter::new());
        let low = Arc::new(FakeDetector::firing("low", Severity::Low, "l"));
        let monitor = ReliabilityMonitor::new(vec![low], reporter.clone(), policy());
        let out = monitor.feed(&signal("m"));
        assert_eq!(out.len(), 1, "OBSERVE_ONLY 异常仍返回给调用方");
        assert_eq!(reporter.calls(), 0, "Low(OBSERVE_ONLY)→ reporter 不收");

        // Critical([LocalSteer, EscalateUser])→ 同样上报。
        let reporter = Arc::new(CountingReporter::new());
        let critical = Arc::new(FakeDetector::firing("critical", Severity::Critical, "c"));
        let monitor = ReliabilityMonitor::new(vec![critical], reporter.clone(), policy());
        let out = monitor.feed(&signal("m"));
        assert_eq!(out.len(), 1);
        assert_eq!(
            reporter.calls(),
            1,
            "Critical(ESCALATE_USER)→ reporter 收到 1 次"
        );
    }

    // 容忍坏检测器:observe panic 被跳过,其余检测器照常产出与上报。
    #[test]
    fn monitor_skips_panicking_detector_and_continues() {
        let bad = Arc::new(FakeDetector::panicking("bad"));
        let good = Arc::new(FakeDetector::firing("good", Severity::Medium, "g"));
        let reporter = Arc::new(CountingReporter::new());
        let monitor =
            ReliabilityMonitor::new(vec![bad.clone(), good.clone()], reporter.clone(), policy());
        let out = monitor.feed(&signal("m"));
        assert_eq!(out.len(), 1, "坏检测器被跳过,其余照常产出");
        assert_eq!(out[0].detector, "good");
        assert_eq!(
            bad.observe_count.load(Ordering::SeqCst),
            1,
            "坏检测器仍被调用了一次"
        );
        assert_eq!(good.observe_count.load(Ordering::SeqCst), 1);
        assert_eq!(reporter.calls(), 1, "good 的 Medium 正常上报");
    }

    // (f) monitor.reset 调用所有检测器 reset。
    #[test]
    fn monitor_reset_calls_all_detectors() {
        let d1 = Arc::new(FakeDetector::silent("d1"));
        let d2 = Arc::new(FakeDetector::silent("d2"));
        let monitor = ReliabilityMonitor::new(
            vec![d1.clone(), d2.clone()],
            Arc::new(CountingReporter::new()),
            policy(),
        );
        monitor.reset();
        assert_eq!(d1.reset_count.load(Ordering::SeqCst), 1);
        assert_eq!(d2.reset_count.load(Ordering::SeqCst), 1);
        monitor.reset();
        assert_eq!(d1.reset_count.load(Ordering::SeqCst), 2);
        assert_eq!(d2.reset_count.load(Ordering::SeqCst), 2);
    }

    // (d) LocalAutoRemediator:High 含 LocalSteer → 返回消息;连续 6 次(> intensity 5)
    //     → None;窗口过期后恢复;Low(无 LocalSteer)→ None;reset 清空窗口。
    #[test]
    fn remediator_steers_high_and_respects_intensity_budget() {
        let clock = Arc::new(Mutex::new(0.0));
        let now = {
            let clock = clock.clone();
            Box::new(move || *clock.lock().unwrap())
        };
        let remediator = LocalAutoRemediator::new(policy(), 5, 60.0, now);
        let high = anomaly("d", Severity::High, "s");

        // 策略含 LocalSteer → 返回纠偏消息(按 i18n 键形参格式化)。
        let msg = remediator
            .steer_message(&high)
            .expect("High 应返回纠偏消息");
        assert_eq!(
            msg,
            format!(
                "reliability.steer_self_correct kind={} summary={}",
                kind_value(high.kind),
                high.summary
            )
        );

        // 连续 5 次在预算内,第 6 次(used=6 > intensity=5)→ None。
        for _ in 0..4 {
            assert!(
                remediator.steer_message(&high).is_some(),
                "前 5 次应在预算内"
            );
        }
        assert!(
            remediator.steer_message(&high).is_none(),
            "第 6 次超过 intensity=5"
        );

        // 窗口过期(period_seconds=60)后恢复。
        *clock.lock().unwrap() = 100.0;
        assert!(remediator.steer_message(&high).is_some(), "窗口过期后恢复");

        // reset 清空窗口 → 立即可再次纠偏。
        remediator.reset();
        assert!(
            remediator.steer_message(&high).is_some(),
            "reset 后恢复预算"
        );

        // Low(OBSERVE_ONLY,无 LocalSteer)→ None(且不记录窗口事件)。
        let low = anomaly("d", Severity::Low, "s");
        assert!(remediator.steer_message(&low).is_none());
        assert!(
            remediator.steer_message(&high).is_some(),
            "无 LocalSteer 的异常不消耗预算"
        );
    }

    // 插件注册:默认策略 / 本地上报器 / monitor 均可按键取回,撤销后移除。
    #[test]
    fn plugin_registers_policy_reporter_monitor() {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(ReliabilityMonitorPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        assert!(ctx.has_service(&RELIABILITY_POLICY));
        assert!(ctx.has_service(&RELIABILITY_REPORTER));
        assert!(ctx.has_service(&RELIABILITY_MONITOR));

        // 上报器可取回具体实例并绑定 sink。
        let reporter = ctx
            .service::<LocalAnomalyReporter>(&RELIABILITY_REPORTER)
            .expect("reporter 可查");
        let seen = Arc::new(AtomicUsize::new(0));
        let seen2 = seen.clone();
        reporter.bind(Arc::new(move |_| {
            seen2.fetch_add(1, Ordering::SeqCst);
        }));
        reporter.report(&anomaly("d", Severity::Medium, "s"));
        assert_eq!(seen.load(Ordering::SeqCst), 1, "注册的上报器绑定后生效");

        // monitor 可查,插件默认无检测器 → feed 空扇出。
        let monitor = ctx
            .service::<ReliabilityMonitor>(&RELIABILITY_MONITOR)
            .expect("monitor 可查");
        assert!(
            monitor.feed(&signal("m")).is_empty(),
            "默认无检测器,feed 空扇出"
        );

        drop(effects);
        assert!(!ctx.has_service(&RELIABILITY_MONITOR), "撤销注册后移除");
    }
}
