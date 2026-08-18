//! # ah-plugins-reliability-burst
//!
//! Real error-burst detectors (aligned with openjiuwen/agent_teams/reliability/):
//! - SlidingWindowCounter: time-bucketed trailing-window event counting
//!   (deterministic; timestamps passed in by the caller);
//! - ErrorBurstDetector: sliding-window rate + consecutive-failure streak,
//!   reset_kind clears the streak, edge-triggered severity emission
//!   (interior Mutex so the Detector trait can stay &self);
//! - ToolErrorRateDetector / ModelErrorRateDetector: the two concrete
//!   configurations over the shared base.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;

use ah_contracts::keys::RELIABILITY_BURST;
use ah_contracts::prelude::Effect;
use ah_contracts::reliability_detectors::{
    Anomaly, AnomalyKind, Detector, Severity, Signal, SignalKind,
};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// Sliding-window event counter (deterministic; timestamps injected).
pub struct SlidingWindowCounter {
    window_seconds: f64,
    events: VecDeque<f64>,
}

impl SlidingWindowCounter {
    pub fn new(window_seconds: f64) -> Self {
        Self {
            window_seconds,
            events: VecDeque::new(),
        }
    }

    fn evict(&mut self, ts: f64) {
        while let Some(&first) = self.events.front() {
            if ts - first > self.window_seconds {
                self.events.pop_front();
            } else {
                break;
            }
        }
    }

    /// Record an event at ts and return the in-window count (incl. this one).
    pub fn add(&mut self, ts: f64) -> usize {
        self.events.push_back(ts);
        self.evict(ts);
        self.events.len()
    }

    /// In-window count as of ts without recording.
    pub fn count(&self, ts: f64) -> usize {
        self.events.iter().filter(|&&e| ts - e <= self.window_seconds).count()
    }

    pub fn reset(&mut self) {
        self.events.clear();
    }
}

/// Mutable burst state behind the &self trait surface.
struct BurstState {
    window: SlidingWindowCounter,
    consecutive: u64,
    fired_severity: Option<Severity>,
}

/// Error-burst detector: window rate + consecutive streak, edge-triggered.
pub struct ErrorBurstDetector {
    name: String,
    error_kind: SignalKind,
    reset_kind: SignalKind,
    anomaly_kind: AnomalyKind,
    window_seconds: f64,
    rate_threshold: u64,
    consecutive_threshold: u64,
    now: Box<dyn Fn() -> f64 + Send + Sync>,
    state: Mutex<BurstState>,
}

impl ErrorBurstDetector {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: &str,
        error_kind: SignalKind,
        reset_kind: SignalKind,
        anomaly_kind: AnomalyKind,
        window_seconds: f64,
        rate_threshold: u64,
        consecutive_threshold: u64,
        now: Box<dyn Fn() -> f64 + Send + Sync>,
    ) -> Self {
        Self {
            name: name.to_string(),
            error_kind,
            reset_kind,
            anomaly_kind,
            window_seconds,
            rate_threshold,
            consecutive_threshold,
            now,
            state: Mutex::new(BurstState {
                window: SlidingWindowCounter::new(window_seconds),
                consecutive: 0,
                fired_severity: None,
            }),
        }
    }

    fn classify(&self, consecutive: u64, window_count: u64) -> Option<Severity> {
        if consecutive >= self.consecutive_threshold * 2
            || window_count >= self.rate_threshold * 2
        {
            return Some(Severity::High);
        }
        if consecutive >= self.consecutive_threshold || window_count >= self.rate_threshold {
            return Some(Severity::Medium);
        }
        None
    }
}

impl Seam for ErrorBurstDetector {}

impl Detector for ErrorBurstDetector {
    fn name(&self) -> &str {
        &self.name
    }

    fn observe(&self, signal: &Signal) -> Option<Anomaly> {
        let mut state = self.state.lock().unwrap();
        if signal.kind == self.reset_kind {
            state.consecutive = 0;
            state.fired_severity = None;
            return None;
        }
        if signal.kind != self.error_kind {
            return None;
        }
        let now = (self.now)();
        state.consecutive += 1;
        let window_count = state.window.add(now) as u64;
        let severity = self.classify(state.consecutive, window_count)?;
        if let Some(previous) = state.fired_severity
            && severity.rank() <= previous.rank()
        {
            return None; // Edge-triggered: no re-fire at same/lower severity.
        }
        state.fired_severity = Some(severity);
        let mut evidence = serde_json::Map::new();
        evidence.insert("consecutive".to_string(), serde_json::json!(state.consecutive));
        evidence.insert("window_count".to_string(), serde_json::json!(window_count));
        evidence.insert("window_seconds".to_string(), serde_json::json!(self.window_seconds));
        evidence.insert(
            "last_error".to_string(),
            serde_json::json!(signal.error.clone().unwrap_or_default()),
        );
        Some(Anomaly {
            detector: self.name.clone(),
            kind: self.anomaly_kind,
            severity,
            member_name: signal.member_name.clone(),
            summary: format!(
                "{} consecutive failures ({} within {:.0}s)",
                state.consecutive, window_count, self.window_seconds
            ),
            evidence,
            peer_member: None,
        })
    }

    fn reset(&self) {
        let mut state = self.state.lock().unwrap();
        state.window.reset();
        state.consecutive = 0;
        state.fired_severity = None;
    }
}

/// Tool-call error-rate detector (default window 60s / rate 5 / consec 3).
pub struct ToolErrorRateDetector {
    inner: ErrorBurstDetector,
}

impl ToolErrorRateDetector {
    pub fn new(now: Box<dyn Fn() -> f64 + Send + Sync>) -> Self {
        Self {
            inner: ErrorBurstDetector::new(
                "tool_error_rate",
                SignalKind::ToolException,
                SignalKind::AfterToolCall,
                AnomalyKind::ToolErrorRate,
                60.0,
                5,
                3,
                now,
            ),
        }
    }
}

impl Seam for ToolErrorRateDetector {}

impl Detector for ToolErrorRateDetector {
    fn name(&self) -> &str {
        "tool_error_rate"
    }
    fn observe(&self, signal: &Signal) -> Option<Anomaly> {
        self.inner.observe(signal)
    }
    fn reset(&self) {
        self.inner.reset()
    }
}

/// Model-call error-rate detector (default window 60s / rate 5 / consec 3).
pub struct ModelErrorRateDetector {
    inner: ErrorBurstDetector,
}

impl ModelErrorRateDetector {
    pub fn new(now: Box<dyn Fn() -> f64 + Send + Sync>) -> Self {
        Self {
            inner: ErrorBurstDetector::new(
                "model_error_rate",
                SignalKind::ModelException,
                SignalKind::AfterModelCall,
                AnomalyKind::ModelError,
                60.0,
                5,
                3,
                now,
            ),
        }
    }
}

impl Seam for ModelErrorRateDetector {}

impl Detector for ModelErrorRateDetector {
    fn name(&self) -> &str {
        "model_error_rate"
    }
    fn observe(&self, signal: &Signal) -> Option<Anomaly> {
        self.inner.observe(signal)
    }
    fn reset(&self) {
        self.inner.reset()
    }
}

/// reliability-burst 插件:注册 tool_error_rate 检测器。
pub struct ReliabilityBurstPlugin;

impl Plugin for ReliabilityBurstPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-reliability-burst"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![RELIABILITY_BURST]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let detector: Arc<dyn Detector> = Arc::new(ToolErrorRateDetector::new(Box::new(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0)
        })));
        Ok(vec![ctx.register(RELIABILITY_BURST, detector)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_hub::plugin::DynPlugin;
    use ah_contracts::reliability_detectors::{
        Anomaly, AnomalyKind, Detector, Severity, Signal, SignalKind,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn now_seq(times: Vec<f64>) -> Box<dyn Fn() -> f64 + Send + Sync> {
        let idx = AtomicUsize::new(0);
        Box::new(move || {
            let i = idx.fetch_add(1, Ordering::SeqCst);
            if i < times.len() { times[i] } else { *times.last().unwrap_or(&0.0) }
        })
    }

    fn tool_err(member: &str, msg: &str) -> Signal {
        let mut s = Signal::new(SignalKind::ToolException, member);
        s.error = Some(msg.to_string());
        s
    }

    fn tool_ok(member: &str) -> Signal {
        Signal::new(SignalKind::AfterToolCall, member)
    }

    /// 收集所有触发(边沿触发下返回的是各严重度上升点)。
    fn feed(det: &dyn Detector, signals: Vec<Signal>) -> Vec<Anomaly> {
        signals.into_iter().filter_map(|s| det.observe(&s)).collect()
    }

    #[test]
    fn sliding_window_counts_and_evicts() {
        let mut w = SlidingWindowCounter::new(10.0);
        assert_eq!(w.add(1.0), 1);
        assert_eq!(w.add(5.0), 2);
        assert_eq!(w.add(11.0), 3, "11-1=10 不逐出");
        assert_eq!(w.add(12.0), 3, "12-1=11 > 10 逐出 1.0");
        assert_eq!(w.count(15.0), 3, "count 不记录");
        w.reset();
        assert_eq!(w.count(15.0), 0);
    }

    #[test]
    fn burst_reaches_medium_then_high_on_escalation() {
        let detector = ToolErrorRateDetector::new(now_seq(vec![0.0, 10.0, 20.0, 30.0, 40.0, 45.0, 50.0]));
        let anomalies = feed(&detector, (0..7).map(|i| tool_err("alice", &format!("e{i}"))).collect());
        // 第 3 个(consecutive=3)→ Medium;第 6 个(consecutive=6 ≥ 6)→ High。
        assert_eq!(anomalies.len(), 2, "edge-triggered 只发上升点");
        assert_eq!(anomalies[0].severity, Severity::Medium);
        assert_eq!(anomalies[1].severity, Severity::High);
        assert_eq!(anomalies[0].kind, AnomalyKind::ToolErrorRate);
        assert_eq!(anomalies[0].member_name, "alice");
        assert!(anomalies[0].summary.contains("3 consecutive failures"));
        assert!(anomalies[1].summary.contains("6 consecutive failures"));
    }

    #[test]
    fn burst_no_refire_at_same_severity() {
        let detector = ToolErrorRateDetector::new(now_seq(vec![0.0, 1.0, 2.0, 3.0, 4.0]));
        let anomalies = feed(&detector, (0..5).map(|i| tool_err("alice", &format!("e{i}"))).collect());
        assert_eq!(anomalies.len(), 1, "5 个错误只触发一次 Medium");
        assert_eq!(anomalies[0].severity, Severity::Medium);
    }

    #[test]
    fn success_resets_streak_and_window_expiry_blocks_fire() {
        let detector = ToolErrorRateDetector::new(now_seq(vec![
            0.0, 1.0, 100.0, 101.0,
        ]));
        feed(&detector, vec![tool_err("bob", "e0"), tool_ok("bob")]);
        // 成功重置后:2 个窗口外错误(consecutive=2 < 3,window 逐出)→ 不触发。
        let anomalies = feed(&detector, vec![tool_err("bob", "n0"), tool_err("bob", "n1")]);
        assert!(anomalies.is_empty(), "窗口外且 streak 不足不触发");
    }

    #[test]
    fn unrelated_kind_ignored() {
        let detector = ToolErrorRateDetector::new(now_seq(vec![0.0, 1.0]));
        assert!(detector.observe(&Signal::new(SignalKind::AfterModelCall, "alice")).is_none());
    }

    #[test]
    fn model_detector_same_shape() {
        let detector = ModelErrorRateDetector::new(now_seq(vec![0.0, 10.0, 20.0, 30.0, 40.0, 45.0]));
        let anomalies = feed(&detector, (0..6).map(|i| {
            let mut s = Signal::new(SignalKind::ModelException, "carol");
            s.error = Some(format!("m{i}"));
            s
        }).collect());
        assert_eq!(anomalies.len(), 2);
        assert_eq!(anomalies[0].severity, Severity::Medium);
        assert_eq!(anomalies[1].severity, Severity::High);
        assert_eq!(anomalies[0].kind, AnomalyKind::ModelError);
        // AfterModelCall 清 streak。
        feed(&detector, vec![Signal::new(SignalKind::AfterModelCall, "carol")]);
        let base = ErrorBurstDetector::new(
            "base",
            SignalKind::ToolException,
            SignalKind::AfterToolCall,
            AnomalyKind::ToolErrorRate,
            60.0,
            5,
            3,
            Box::new(|| 0.0),
        );
        assert_eq!(base.name(), "base");
        base.reset();
    }

    #[test]
    fn plugin_registers_burst_detector() {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(ReliabilityBurstPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let detector = ctx.service::<dyn Detector>(&RELIABILITY_BURST).expect("burst detector");
        assert_eq!(detector.name(), "tool_error_rate");
        drop(effects);
        assert!(!ctx.has_service(&RELIABILITY_BURST));
    }
}