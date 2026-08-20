//! 可靠性 rail + handler + 装配决策(对齐 rail.py / handler.py / factory.py)。
//!
//! - `MemberReliabilityRail`:成员生命周期信号采集 — 每个 hook 把回调上下文
//!   归一化为 Signal,喂给成员的 ReliabilityMonitor,并对返回的异常应用可逆
//!   本地自纠偏(LocalAutoRemediator);可绑定 leader 进程内异常 sink。
//! - `LeaderReliabilityHandler`:把上报的异常按策略路由(escalate/report)并
//!   格式化为一行摘要(路由投递由协调运行时接线,本实现只做决策 + 格式)。
//! - `ReliabilityAssembly`:按配置产出检测器规格 + 修复策略视图(装配决策,
//!   真实检测器构建由各检测插件负责)。

use std::sync::Arc;

use ah_contracts::reliability_config::{DetectorsConfig, RemediationPolicyConfig};
use ah_contracts::reliability_detectors::{Anomaly, Severity, Signal};
use ah_contracts::reliability_rail::{
    DetectorSpec, LocalSink, PolicyView, ReliabilityFactory, ReliabilityHandler, ReliabilityRail,
    RouteDecision, after_model_call_signal, after_tool_call_signal, before_model_call_signal,
    before_tool_call_signal, format_anomaly, format_anomaly_event, member_detector_specs,
    model_exception_signal, route_decision, tool_exception_signal,
};
use ah_contracts::seam::Seam;
use serde_json::Value;

use crate::LocalAnomalyReporter;
use crate::LocalAutoRemediator;
use crate::ReliabilityMonitor;

/// 成员可靠性 rail:生命周期信号 → monitor → 本地纠偏。
///
/// 对齐 `ReliabilityRail`:rail 从不改写工具参数/模型调用/agent 循环,唯一
/// 副作用是非破坏性的纠偏消息。`emit` 返回本地纠偏消息;上报(REPORT_LEADER /
/// ESCALATE_USER)由 monitor 内部的 reporter 完成。
pub struct MemberReliabilityRail {
    monitor: Arc<ReliabilityMonitor>,
    member_name: String,
    auto: Option<Arc<LocalAutoRemediator>>,
    local_reporter: Option<Arc<LocalAnomalyReporter>>,
}

impl MemberReliabilityRail {
    /// 绑定成员 monitor 与可选本地纠偏/本地上报器构建 rail。
    pub fn new(
        monitor: Arc<ReliabilityMonitor>,
        member_name: impl Into<String>,
        auto: Option<Arc<LocalAutoRemediator>>,
        local_reporter: Option<Arc<LocalAnomalyReporter>>,
    ) -> Self {
        Self {
            monitor,
            member_name: member_name.into(),
            auto,
            local_reporter,
        }
    }

    /// 喂一条信号:monitor 检测 → 上报副作用已发生 → 本地纠偏消息收集。
    fn feed(&self, signal: &Signal) -> Vec<String> {
        let anomalies = self.monitor.feed(signal);
        let Some(auto) = &self.auto else {
            return Vec::new();
        };
        let mut messages = Vec::new();
        for anomaly in &anomalies {
            if let Some(message) = auto.steer_message(anomaly) {
                messages.push(message);
            }
        }
        messages
    }
}

impl Seam for MemberReliabilityRail {}

impl ReliabilityRail for MemberReliabilityRail {
    fn bind_local_sink(&self, sink: LocalSink) {
        if let Some(reporter) = &self.local_reporter {
            reporter.bind(sink);
        }
    }

    fn emit(&self, signal: &Signal) -> Vec<String> {
        self.feed(signal)
    }

    fn before_tool_call(&self, tool_name: &str, tool_args: Option<&Value>) -> Vec<String> {
        self.feed(&before_tool_call_signal(
            &self.member_name,
            tool_name,
            tool_args,
        ))
    }

    fn after_tool_call(&self, tool_name: &str, tool_result: Option<&Value>) -> Vec<String> {
        self.feed(&after_tool_call_signal(
            &self.member_name,
            tool_name,
            tool_result,
        ))
    }

    fn on_tool_exception(&self, tool_name: &str, exc: Option<&str>) -> Vec<String> {
        self.feed(&tool_exception_signal(&self.member_name, tool_name, exc))
    }

    fn on_model_exception(&self, exc: Option<&str>) -> Vec<String> {
        self.feed(&model_exception_signal(&self.member_name, exc))
    }

    fn before_model_call(&self, message_count: Option<u64>) -> Vec<String> {
        self.feed(&before_model_call_signal(&self.member_name, message_count))
    }

    fn after_model_call(&self, text_len: Option<u64>, thinking_len: Option<u64>) -> Vec<String> {
        self.feed(&after_model_call_signal(
            &self.member_name,
            text_len,
            thinking_len,
        ))
    }
}

/// leader 侧可靠性 handler:策略路由决策 + 异常格式化。
///
/// 对齐 `ReliabilityHandler` 的确定性部分(`_route` 动作选择 + `_format` /
/// `_format_anomaly`);事件订阅与投递(把摘要送进 leader 循环)由协调运行时
/// 接线,本实现只做决策与格式。
pub struct LeaderReliabilityHandler {
    policy: PolicyView,
}

impl LeaderReliabilityHandler {
    /// 以修复策略视图构建。
    pub fn new(policy: PolicyView) -> Self {
        Self { policy }
    }

    /// 按策略选择路由目标(对齐 `_route`)。
    pub fn route_severity(&self, severity: Severity) -> Option<RouteDecision> {
        route_decision(&self.policy.actions_for(severity))
    }
}

impl Seam for LeaderReliabilityHandler {}

impl ReliabilityHandler for LeaderReliabilityHandler {
    fn format_event(
        &self,
        severity: Severity,
        member_name: &str,
        summary: &str,
        detector: &str,
    ) -> String {
        format_anomaly_event(severity, member_name, summary, detector)
    }

    fn format_anomaly(&self, anomaly: &Anomaly) -> String {
        format_anomaly(anomaly)
    }

    fn route(&self, severity: Severity) -> Option<RouteDecision> {
        self.route_severity(severity)
    }
}

/// 装配决策:config → 检测器规格 + 修复策略视图。
pub struct ReliabilityAssembly;

impl Seam for ReliabilityAssembly {}

impl ReliabilityFactory for ReliabilityAssembly {
    fn member_detector_specs(&self, config: &DetectorsConfig) -> Vec<DetectorSpec> {
        member_detector_specs(config)
    }

    fn remediation_policy(&self, config: &RemediationPolicyConfig) -> PolicyView {
        PolicyView::from_config(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::reliability_detectors::{AnomalyKind, SignalKind};
    use serde_json::json;

    fn anomaly(severity: Severity, summary: &str) -> Anomaly {
        Anomaly {
            detector: "tool_error_rate".to_string(),
            kind: AnomalyKind::ToolErrorRate,
            severity,
            member_name: "dev-1".to_string(),
            summary: summary.to_string(),
            evidence: serde_json::Map::new(),
            peer_member: None,
        }
    }

    /// 固定产出一条异常的检测器(测试真实路径,非 mock)。
    struct FixedDetector {
        produce: Option<Anomaly>,
    }

    impl Seam for FixedDetector {}

    impl ah_contracts::reliability_detectors::Detector for FixedDetector {
        fn name(&self) -> &str {
            "fixed"
        }
        fn observe(&self, _signal: &Signal) -> Option<Anomaly> {
            self.produce.clone()
        }
        fn reset(&self) {}
    }

    fn policy() -> Arc<crate::RemediationPolicy> {
        Arc::new(crate::RemediationPolicy::new(
            &ah_contracts::reliability_config::RemediationPolicyConfig::default(),
        ))
    }

    fn rail_with_auto() -> MemberReliabilityRail {
        let detector = Arc::new(FixedDetector {
            produce: Some(anomaly(Severity::High, "too many tool errors")),
        });
        let reporter = Arc::new(LocalAnomalyReporter::new());
        let monitor = Arc::new(ReliabilityMonitor::new(vec![detector], reporter, policy()));
        let auto = Arc::new(LocalAutoRemediator::with_defaults(policy()));
        MemberReliabilityRail::new(monitor, "dev-1", Some(auto), None)
    }

    #[test]
    fn rail_hooks_feed_and_steer() {
        let rail = rail_with_auto();
        // High 异常 → 策略含 LocalSteer → 返回纠偏消息。
        let messages = rail.before_tool_call("run", Some(&json!({"cmd": "x"})));
        assert_eq!(messages.len(), 1, "High → LocalSteer 消息");
        assert!(
            messages[0].contains("kind=tool_error_rate"),
            "{:?}",
            messages
        );
    }

    #[test]
    fn rail_hook_shapes() {
        let rail = rail_with_auto();
        // 不同 hook 归一化为对应 SignalKind(检测器固定产出,数量一致)。
        assert_eq!(rail.on_tool_exception("run", Some("denied")).len(), 1);
        assert_eq!(rail.on_model_exception(Some("timeout")).len(), 1);
        assert_eq!(rail.before_model_call(Some(100)).len(), 1);
        assert_eq!(rail.after_model_call(Some(100), Some(10)).len(), 1);
        assert_eq!(
            rail.after_tool_call("run", Some(&json!({"ok": true})))
                .len(),
            1
        );
    }

    #[test]
    fn rail_bind_sink_routes_local_anomalies() {
        let reporter = Arc::new(LocalAnomalyReporter::new());
        let detector = Arc::new(FixedDetector {
            produce: Some(anomaly(Severity::High, "boom")),
        });
        let monitor = Arc::new(ReliabilityMonitor::new(
            vec![detector],
            reporter.clone(),
            policy(),
        ));
        let rail = MemberReliabilityRail::new(monitor, "dev-1", None, Some(reporter.clone()));
        let seen: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen_sink = seen.clone();
        rail.bind_local_sink(Arc::new(move |a: &Anomaly| {
            seen_sink.lock().unwrap().push(a.summary.clone());
        }));
        // 触发一次检测,本地 sink 应收到异常(经 monitor 上报路径)。
        let _ = rail.on_model_exception(Some("timeout"));
        assert_eq!(seen.lock().unwrap().len(), 1, "本地 sink 收到异常");
    }

    #[test]
    fn handler_formats_and_routes() {
        let handler = LeaderReliabilityHandler::new(PolicyView::from_config(
            &ah_contracts::reliability_config::RemediationPolicyConfig::default(),
        ));
        // 默认策略:Low → 无路由;Medium → report;Critical → escalate。
        assert_eq!(handler.route(Severity::Low), None);
        assert_eq!(
            handler.route(Severity::Medium),
            Some(RouteDecision::ReportLeader)
        );
        assert_eq!(
            handler.route(Severity::Critical),
            Some(RouteDecision::EscalateUser)
        );
        let text = handler.format_event(
            Severity::Medium,
            "dev-2",
            "repeat calls",
            "repeat_tool_call",
        );
        assert_eq!(
            text,
            "[medium] dev-2: repeat calls (detector=repeat_tool_call)"
        );
        let a = anomaly(Severity::Low, "quiet");
        assert_eq!(
            handler.format_anomaly(&a),
            "[low] dev-1: quiet (detector=tool_error_rate)"
        );
    }

    #[test]
    fn assembly_specs_and_policy() {
        let assembly = ReliabilityAssembly;
        let specs = assembly.member_detector_specs(&DetectorsConfig::default());
        assert!(!specs.is_empty());
        assert_eq!(specs[0].name, "tool_error_rate");
        let view = assembly.remediation_policy(&RemediationPolicyConfig::default());
        assert_eq!(view.actions_for(Severity::High).len(), 2);
    }

    #[test]
    fn signal_builder_json_string_args() {
        // 字符串 JSON 参数归一化为 object(rail.py `_args_as_dict`)。
        let s = before_tool_call_signal("m", "edit", Some(&json!("{\"file\": \"a.txt\"}")));
        assert_eq!(s.tool_args, Some(json!({"file": "a.txt"})));
        // 畸形字符串 → None。
        let bad = before_tool_call_signal("m", "edit", Some(&json!("not-json")));
        assert_eq!(bad.tool_args, None);
        let _ = SignalKind::Message; // 保持 import 使用
    }
}
