//! reliability-rail seam:成员可靠性 rail + leader 侧 handler + 装配决策
//! (对齐 openjiuwen/agent_teams/reliability/{rail,handler,factory}.py)。
//!
//! - `ReliabilityRail`:把成员执行生命周期回调(before/after tool call、
//!   tool/model exception、before/after model call)归一化为 `Signal`,
//!   喂给成员的 `ReliabilityMonitor`,并对返回的异常应用可逆本地自纠偏
//!   (LocalAutoRemediator)与本地上报(可绑定 sink);
//! - `ReliabilityHandler`:leader 侧协调 handler——把上报的异常按策略路由进
//!   leader 循环(escalate/report),并格式化异常为一行摘要;
//! - 装配决策:`member_detector_specs`(config → 启用的检测器规格列表,
//!   对齐 factory.py `build_member_detectors` 的 enabled 决策)。
//!
//! 纯函数、无 IO、无 LLM;契约零实现(信号归一化/格式化/规格决策为确定性纯函数,
//! 事件订阅与投递由协调运行时经 seam 注入)。

use crate::reliability_config::{DetectorsConfig, RemediationAction, RemediationPolicyConfig};
use crate::reliability_detectors::{Anomaly, Severity, Signal, SignalKind};
use crate::seam::Seam;
use serde_json::Value;

/// 把异常渲染为稳定短错误串(对齐 rail.py `_error_text`)。
pub fn error_text(exc: Option<&str>) -> String {
    match exc {
        Some(text) if !text.is_empty() => text.to_string(),
        _ => "error".to_string(),
    }
}

/// 归一化 dict / JSON-object 工具参数用于稳定哈希(对齐 rail.py `_args_as_dict`)。
///
/// - dict → 原样;
/// - JSON 字符串且解析为 object → 解析值;
/// - 其余(非 dict / 解析失败 / 非 object)→ None。
pub fn args_as_dict(tool_args: Option<&Value>) -> Option<Value> {
    match tool_args {
        Some(Value::Object(_)) => tool_args.cloned(),
        Some(Value::String(raw)) => {
            let parsed: Value = serde_json::from_str(raw).ok()?;
            if parsed.is_object() {
                Some(parsed)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// 从响应提取输出/思考长度(对齐 rail.py `_measure_response`)。
///
/// 返回 `(text_len, thinking_len)`,字段缺失或非字符串 → None。`thinking`
/// 优先取 reasoning_content,其次 thinking(由调用方判定字段名后传入)。
pub fn measure_response(
    content: Option<&str>,
    thinking: Option<&str>,
) -> (Option<u64>, Option<u64>) {
    let text_len = content.map(|s| s.chars().count() as u64);
    let thinking_len = thinking.map(|s| s.chars().count() as u64);
    (text_len, thinking_len)
}

/// 组装 before_tool_call 信号(对齐 rail.py `before_tool_call`)。
pub fn before_tool_call_signal(
    member_name: &str,
    tool_name: &str,
    tool_args: Option<&Value>,
) -> Signal {
    let mut s = Signal::new(SignalKind::BeforeToolCall, member_name);
    s.tool_name = if tool_name.is_empty() {
        None
    } else {
        Some(tool_name.to_string())
    };
    s.tool_args = args_as_dict(tool_args);
    s
}

/// 组装 after_tool_call 信号(对齐 rail.py `after_tool_call`)。
pub fn after_tool_call_signal(
    member_name: &str,
    tool_name: &str,
    tool_result: Option<&Value>,
) -> Signal {
    let mut s = Signal::new(SignalKind::AfterToolCall, member_name);
    s.tool_name = if tool_name.is_empty() {
        None
    } else {
        Some(tool_name.to_string())
    };
    s.tool_result = tool_result.cloned();
    s
}

/// 组装 tool exception 信号(对齐 rail.py `on_tool_exception`)。
pub fn tool_exception_signal(member_name: &str, tool_name: &str, exc: Option<&str>) -> Signal {
    let mut s = Signal::new(SignalKind::ToolException, member_name);
    s.tool_name = if tool_name.is_empty() {
        None
    } else {
        Some(tool_name.to_string())
    };
    s.error = Some(error_text(exc));
    s
}

/// 组装 model exception 信号(对齐 rail.py `on_model_exception`)。
pub fn model_exception_signal(member_name: &str, exc: Option<&str>) -> Signal {
    let mut s = Signal::new(SignalKind::ModelException, member_name);
    s.error = Some(error_text(exc));
    s
}

/// 组装 before_model_call 信号(对齐 rail.py `before_model_call`)。
pub fn before_model_call_signal(member_name: &str, message_count: Option<u64>) -> Signal {
    let mut s = Signal::new(SignalKind::BeforeModelCall, member_name);
    s.message_count = message_count;
    s
}

/// 组装 after_model_call 信号(对齐 rail.py `after_model_call`)。
pub fn after_model_call_signal(
    member_name: &str,
    text_len: Option<u64>,
    thinking_len: Option<u64>,
) -> Signal {
    let mut s = Signal::new(SignalKind::AfterModelCall, member_name);
    s.text_len = text_len;
    s.thinking_len = thinking_len;
    s
}

/// 把 AnomalyDetectedEvent 渲染为一行摘要(对齐 handler.py `_format`)。
pub fn format_anomaly_event(
    severity: Severity,
    member_name: &str,
    summary: &str,
    detector: &str,
) -> String {
    format!(
        "[{}] {member_name}: {summary} (detector={detector})",
        severity_value(severity)
    )
}

/// 把本地 Anomaly 渲染为一行摘要(对齐 handler.py `_format_anomaly`)。
pub fn format_anomaly(anomaly: &Anomaly) -> String {
    format_anomaly_event(
        anomaly.severity,
        &anomaly.member_name,
        &anomaly.summary,
        &anomaly.detector,
    )
}

/// Severity 的 snake_case 值(与契约 serde 一致)。
pub fn severity_value(severity: Severity) -> &'static str {
    match severity {
        Severity::Low => "low",
        Severity::Medium => "medium",
        Severity::High => "high",
        Severity::Critical => "critical",
    }
}

/// leader 路由决策(对齐 handler.py `_route` 的动作选择)。
///
/// 按策略动作列表选择投递目标:含 ESCALATE_USER → EscalateUser;
/// 否则含 REPORT_LEADER → ReportLeader;两者皆无 → None(仅观察/本地纠偏)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteDecision {
    EscalateUser,
    ReportLeader,
}

/// 按策略动作选择路由目标(对齐 handler.py `_route`)。
pub fn route_decision(actions: &[RemediationAction]) -> Option<RouteDecision> {
    if actions.contains(&RemediationAction::EscalateUser) {
        Some(RouteDecision::EscalateUser)
    } else if actions.contains(&RemediationAction::ReportLeader) {
        Some(RouteDecision::ReportLeader)
    } else {
        None
    }
}

/// 检测器规格(装配决策的产物;构建真实检测器由各检测插件负责)。
#[derive(Debug, Clone, PartialEq)]
pub struct DetectorSpec {
    /// 稳定检测器名(如 "tool_error_rate")。
    pub name: &'static str,
}

/// 按配置产出启用的成员检测器规格(对齐 factory.py `build_member_detectors`
/// 的 enabled 决策;顺序即构建顺序)。
pub fn member_detector_specs(config: &DetectorsConfig) -> Vec<DetectorSpec> {
    let mut specs = Vec::new();
    if config.tool_error.enabled {
        specs.push(DetectorSpec {
            name: "tool_error_rate",
        });
    }
    if config.repeat_tool.enabled {
        specs.push(DetectorSpec {
            name: "repeat_tool_call",
        });
    }
    if config.model_error.enabled {
        specs.push(DetectorSpec {
            name: "model_error",
        });
    }
    if config.output_length.enabled {
        specs.push(DetectorSpec {
            name: "output_length",
        });
    }
    if config.compaction.enabled {
        specs.push(DetectorSpec {
            name: "frequent_compaction",
        });
    }
    specs
}

/// 可靠性 rail Seam(Service Definition):成员生命周期信号采集 + 本地纠偏。
///
/// 对齐 `ReliabilityRail`:每个 hook 把回调上下文归一化为 Signal,喂给成员的
/// monitor(检测并上报异常),并对返回的异常应用可逆本地自纠偏。rail 从不改写
/// 工具参数/模型调用/agent 循环——唯一副作用是非破坏性的纠偏消息。
pub trait ReliabilityRail: Seam {
    /// 绑定 leader 进程内异常 sink(对齐 `bind_local_sink`;非 leader 为 no-op)。
    fn bind_local_sink(&self, sink: crate::reliability_rail::LocalSink);

    /// 喂一条信号并返回本地纠偏消息(对齐 `_emit` 的本地部分)。
    ///
    /// 调用方(agent 循环)把返回的每条消息推入纠偏队列。
    fn emit(&self, signal: &Signal) -> Vec<String>;

    /// before_tool_call 生命周期点。
    fn before_tool_call(&self, tool_name: &str, tool_args: Option<&Value>) -> Vec<String>;

    /// after_tool_call 生命周期点。
    fn after_tool_call(&self, tool_name: &str, tool_result: Option<&Value>) -> Vec<String>;

    /// tool exception 生命周期点。
    fn on_tool_exception(&self, tool_name: &str, exc: Option<&str>) -> Vec<String>;

    /// model exception 生命周期点。
    fn on_model_exception(&self, exc: Option<&str>) -> Vec<String>;

    /// before_model_call 生命周期点(上下文消息数,压缩推断用)。
    fn before_model_call(&self, message_count: Option<u64>) -> Vec<String>;

    /// after_model_call 生命周期点(输出/思考长度)。
    fn after_model_call(&self, text_len: Option<u64>, thinking_len: Option<u64>) -> Vec<String>;
}

/// 本地异常 sink 类型(对齐 `Callable[[Anomaly], Awaitable[None]]` 的进程内形态)。
pub type LocalSink = std::sync::Arc<dyn Fn(&Anomaly) + Send + Sync>;

/// 可靠性 handler Seam(Service Definition):leader 侧路由 + 格式。
///
/// 对齐 `ReliabilityHandler` 的确定性部分:异常事件/本地异常 → 一行摘要;
/// 按策略路由决策(escalate/report)。事件订阅与投递由协调运行时接线。
pub trait ReliabilityHandler: Seam {
    /// 渲染异常事件为一行摘要(对齐 `_format`)。
    fn format_event(
        &self,
        severity: Severity,
        member_name: &str,
        summary: &str,
        detector: &str,
    ) -> String;

    /// 渲染本地异常为一行摘要(对齐 `_format_anomaly`)。
    fn format_anomaly(&self, anomaly: &Anomaly) -> String;

    /// 按策略选择路由目标(对齐 `_route` 的动作选择)。
    fn route(&self, severity: Severity) -> Option<RouteDecision>;
}

/// 可靠性装配决策 Seam(Service Definition):config → 组件规格。
///
/// 对齐 factory.py 的确定性装配部分(`build_member_detectors` 的 enabled
/// 决策 + `build_remediation_policy`)。构建真实检测器/修复器由各插件负责。
pub trait ReliabilityFactory: Seam {
    /// 按配置产出启用的检测器规格(对齐 `build_member_detectors`)。
    fn member_detector_specs(&self, config: &DetectorsConfig) -> Vec<DetectorSpec>;

    /// 从配置构建修复策略(对齐 `build_remediation_policy`)。
    fn remediation_policy(
        &self,
        config: &RemediationPolicyConfig,
    ) -> crate::reliability_rail::PolicyView;
}

/// 修复策略视图(供消费方构造/持有具体策略;对齐 `RemediationPolicy`)。
#[derive(Debug, Clone, PartialEq)]
pub struct PolicyView {
    /// 严重度 → 动作列表。
    pub severity_actions: std::collections::BTreeMap<Severity, Vec<RemediationAction>>,
}

impl PolicyView {
    /// 从配置构建。
    pub fn from_config(config: &RemediationPolicyConfig) -> Self {
        Self {
            severity_actions: config.severity_actions.clone(),
        }
    }

    /// 返回 severity 配置的修复动作(未配置返回空列表)。
    pub fn actions_for(&self, severity: Severity) -> Vec<RemediationAction> {
        self.severity_actions
            .get(&severity)
            .cloned()
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn error_text_falls_back() {
        assert_eq!(error_text(None), "error");
        assert_eq!(error_text(Some("")), "error");
        assert_eq!(error_text(Some("boom")), "boom");
    }

    #[test]
    fn args_as_dict_normalizes() {
        // dict 原样。
        let obj = json!({"a": 1});
        assert_eq!(args_as_dict(Some(&obj)), Some(obj));
        // JSON 字符串解析为 object。
        let raw = json!("{\"a\": 1}");
        assert_eq!(args_as_dict(Some(&raw)), Some(json!({"a": 1})));
        // 非 object / 解析失败 / None → None。
        let arr = json!([1, 2]);
        assert_eq!(args_as_dict(Some(&arr)), None);
        let bad = json!("not json");
        assert_eq!(args_as_dict(Some(&bad)), None);
        let num = json!(42);
        assert_eq!(args_as_dict(Some(&num)), None);
        assert_eq!(args_as_dict(None), None);
    }

    #[test]
    fn measure_response_extracts_lengths() {
        assert_eq!(measure_response(None, None), (None, None));
        assert_eq!(measure_response(Some("hello"), None), (Some(5), None));
        assert_eq!(
            measure_response(Some("hi"), Some("think")),
            (Some(2), Some(5))
        );
        // 中文按字符计(对齐 Python len)。
        assert_eq!(measure_response(Some("你好"), None), (Some(2), None));
    }

    #[test]
    fn signal_builders_populate_fields() {
        let before = before_tool_call_signal("m", "read_file", Some(&json!({"p": "a"})));
        assert_eq!(before.kind, SignalKind::BeforeToolCall);
        assert_eq!(before.tool_name.as_deref(), Some("read_file"));
        assert_eq!(before.tool_args, Some(json!({"p": "a"})));
        // 空工具名 → None。
        let empty = before_tool_call_signal("m", "", None);
        assert_eq!(empty.tool_name, None);

        let exc = tool_exception_signal("m", "run", Some("denied"));
        assert_eq!(exc.kind, SignalKind::ToolException);
        assert_eq!(exc.error.as_deref(), Some("denied"));

        let model_exc = model_exception_signal("m", None);
        assert_eq!(model_exc.error.as_deref(), Some("error"));

        let before_model = before_model_call_signal("m", Some(42));
        assert_eq!(before_model.message_count, Some(42));

        let after_model = after_model_call_signal("m", Some(100), Some(20));
        assert_eq!(after_model.text_len, Some(100));
        assert_eq!(after_model.thinking_len, Some(20));
    }

    #[test]
    fn formatting_matches_python_shape() {
        let text = format_anomaly_event(Severity::High, "dev-1", "tool errors", "tool_error_rate");
        assert_eq!(text, "[high] dev-1: tool errors (detector=tool_error_rate)");
        let anomaly = Anomaly {
            detector: "model_error".to_string(),
            kind: crate::reliability_detectors::AnomalyKind::ModelError,
            severity: Severity::Critical,
            member_name: "m".to_string(),
            summary: "stream broke".to_string(),
            evidence: serde_json::Map::new(),
            peer_member: None,
        };
        assert_eq!(
            format_anomaly(&anomaly),
            "[critical] m: stream broke (detector=model_error)"
        );
    }

    #[test]
    fn route_decision_selects_by_policy() {
        use RemediationAction::*;
        assert_eq!(route_decision(&[ObserveOnly]), None, "仅观察 → 无路由");
        assert_eq!(
            route_decision(&[ReportLeader]),
            Some(RouteDecision::ReportLeader)
        );
        // escalate 优先于 report。
        assert_eq!(
            route_decision(&[LocalSteer, ReportLeader, EscalateUser]),
            Some(RouteDecision::EscalateUser)
        );
        assert_eq!(route_decision(&[]), None);
    }

    #[test]
    fn member_detector_specs_respects_enabled() {
        let mut config = DetectorsConfig::default();
        let specs = member_detector_specs(&config);
        let names: Vec<&str> = specs.iter().map(|s| s.name).collect();
        assert_eq!(
            names,
            vec![
                "tool_error_rate",
                "repeat_tool_call",
                "model_error",
                "output_length",
                "frequent_compaction"
            ]
        );
        // 逐个关闭。
        config.tool_error.enabled = false;
        config.output_length.enabled = false;
        let names2: Vec<&str> = member_detector_specs(&config)
            .iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(
            names2,
            vec!["repeat_tool_call", "model_error", "frequent_compaction"]
        );
        // 全关 → 空。
        config.repeat_tool.enabled = false;
        config.model_error.enabled = false;
        config.compaction.enabled = false;
        assert!(member_detector_specs(&config).is_empty());
    }

    #[test]
    fn policy_view_actions_for_defaults() {
        let view = PolicyView::from_config(&RemediationPolicyConfig::default());
        use RemediationAction::*;
        assert_eq!(view.actions_for(Severity::Low), vec![ObserveOnly]);
        assert_eq!(view.actions_for(Severity::Medium), vec![ReportLeader]);
        assert_eq!(
            view.actions_for(Severity::High),
            vec![LocalSteer, ReportLeader]
        );
        assert_eq!(
            view.actions_for(Severity::Critical),
            vec![LocalSteer, EscalateUser]
        );
    }
}
