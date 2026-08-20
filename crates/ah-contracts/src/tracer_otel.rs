//! tracer-otel seam:OpenTelemetry 追踪扩展(对齐
//! `openjiuwen/extensions/tracer_otel/` 的确定性部分)。
//!
//! - `OtelTracerConfig` + `validate_sample_rate`(对齐 config.py);
//! - redaction:`truncate` / `hash_value`(sha256: 前缀 + 16 hex)/ `should_redact`
//!   / `redact`(对齐 redaction.py);
//! - semconv 属性键常量(对齐 semconv.py);
//! - span managers:`OtelAgentSpanManager` / `OtelWorkflowSpanManager`
//!   (invoke_id → span 状态 + 增量数据缓冲,对齐 span_manager.py);
//! - setup 决策:`resolve_exporter`(exporter_type / protocol 校验 + http
//!   endpoint 补 `/v1/traces`,对齐 setup.py);
//! - 属性映射纯函数:序列化 / 耗时格式化 / agent·llm·workflow 属性构建
//!   / 父上下文解析决策 / 组件分类(对齐 handler.py 确定性部分)。
//!
//! 纯函数、无 IO、无 LLM;契约零实现。OTel SDK span 对象与导出器经 seam
//! 注入(插件持有真实 tracer)。

use crate::seam::Seam;
use serde_json::{Map, Value};
use std::collections::HashMap;

/// SHA-256 脱敏前缀(对齐 `_REDACTED_PREFIX`)。
pub const REDACTED_PREFIX: &str = "sha256:";
/// 截断后缀(对齐 `_TRUNCATED_SUFFIX`)。
pub const TRUNCATED_SUFFIX: &str = "...<truncated>";

/// OTel 追踪扩展配置(对齐 OtelTracerConfig;不可变)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OtelTracerConfig {
    #[serde(default = "default_tracer_name")]
    pub tracer_name: String,
    /// otlp / console。
    #[serde(default = "default_exporter_type")]
    pub exporter_type: String,
    #[serde(default)]
    pub exporter_endpoint: Option<String>,
    /// grpc / http — OTLP 传输协议。
    #[serde(default = "default_protocol")]
    pub protocol: String,
    #[serde(default)]
    pub headers: std::collections::BTreeMap<String, String>,
    #[serde(default = "default_service_name")]
    pub service_name: String,
    #[serde(default)]
    pub service_version: Option<String>,
    /// 0.0 ~ 1.0 采样概率。
    #[serde(default = "default_sample_rate")]
    pub sample_rate: f64,
    #[serde(default = "default_schedule_delay")]
    pub schedule_delay_millis: u64,
    #[serde(default = "default_export_timeout")]
    pub export_timeout_ms: u64,
    #[serde(default = "default_max_batch")]
    pub max_export_batch_size: u64,
    /// True → SHA-256 哈希(向后兼容;redact_prompts / redact_completions 覆盖)。
    #[serde(default = "default_true")]
    pub redaction_enabled: bool,
    /// None → 回退 redaction_enabled;True/False 覆盖。
    #[serde(default)]
    pub redact_prompts: Option<bool>,
    #[serde(default)]
    pub redact_completions: Option<bool>,
    /// 属性值截断上限。
    #[serde(default = "default_max_attr_length")]
    pub max_attr_length: usize,
}

fn default_tracer_name() -> String {
    "openjiuwen.tracer.otel".to_string()
}
fn default_exporter_type() -> String {
    "otlp".to_string()
}
fn default_protocol() -> String {
    "grpc".to_string()
}
fn default_service_name() -> String {
    "openjiuwen".to_string()
}
fn default_sample_rate() -> f64 {
    1.0
}
fn default_schedule_delay() -> u64 {
    5000
}
fn default_export_timeout() -> u64 {
    30000
}
fn default_max_batch() -> u64 {
    512
}
fn default_true() -> bool {
    true
}
fn default_max_attr_length() -> usize {
    4096
}

impl Default for OtelTracerConfig {
    fn default() -> Self {
        Self {
            tracer_name: default_tracer_name(),
            exporter_type: default_exporter_type(),
            exporter_endpoint: None,
            protocol: default_protocol(),
            headers: std::collections::BTreeMap::new(),
            service_name: default_service_name(),
            service_version: None,
            sample_rate: default_sample_rate(),
            schedule_delay_millis: default_schedule_delay(),
            export_timeout_ms: default_export_timeout(),
            max_export_batch_size: default_max_batch(),
            redaction_enabled: default_true(),
            redact_prompts: None,
            redact_completions: None,
            max_attr_length: default_max_attr_length(),
        }
    }
}

/// 校验 sample_rate ∈ [0.0, 1.0](对齐 `__post_init__`)。
pub fn validate_sample_rate(sample_rate: f64) -> Result<(), OtelTracerError> {
    if (0.0..=1.0).contains(&sample_rate) {
        Ok(())
    } else {
        Err(OtelTracerError(format!(
            "sample_rate must be between 0.0 and 1.0, got {sample_rate}"
        )))
    }
}

/// OTel 追踪错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtelTracerError(pub String);

impl core::fmt::Display for OtelTracerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for OtelTracerError {}

// ---------------------------------------------------------------------------
// redaction
// ---------------------------------------------------------------------------

/// 硬截断字符串并标记截断(对齐 `truncate`)。
pub fn truncate(value: &str, max_length: usize) -> String {
    if max_length == 0 || value.chars().count() <= max_length {
        return value.to_string();
    }
    let truncated: String = value.chars().take(max_length).collect();
    format!("{truncated}{TRUNCATED_SUFFIX}")
}

/// 以短内容哈希替换值(对齐 `hash_value`;sha256: + 前 16 hex)。
pub fn hash_value(value: &str) -> String {
    let digest = crate::worktree::sha256(value.as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("{REDACTED_PREFIX}{hex}")
}

/// 解析是否应用脱敏(对齐 `_should_redact`)。
///
/// `field` 为 "prompts" / "completions" 时用细粒度覆盖(非 None);
/// 否则用旧版 `redaction_enabled` 标志。
pub fn should_redact(config: &OtelTracerConfig, field: Option<&str>) -> bool {
    match field {
        Some("prompts") => config.redact_prompts.unwrap_or(config.redaction_enabled),
        Some("completions") => config
            .redact_completions
            .unwrap_or(config.redaction_enabled),
        _ => config.redaction_enabled,
    }
}

/// 应用脱敏策略(对齐 `redact`):启用 → SHA-256 哈希;禁用 → 仅截断。
/// 恒返回字符串。
pub fn redact(value: Option<&Value>, config: &OtelTracerConfig, field: Option<&str>) -> String {
    let text = value.map(serialize_value).unwrap_or_default();
    if should_redact(config, field) {
        hash_value(&text)
    } else {
        truncate(&text, config.max_attr_length)
    }
}

/// 序列化值为 JSON 字符串(对齐 `_serialize`;dict/list → JSON,其余 → str)。
pub fn serialize_value(value: &Value) -> String {
    match value {
        Value::Object(_) | Value::Array(_) => {
            serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
        }
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------------------
// semconv — 属性键常量(对齐 semconv.py)
// ---------------------------------------------------------------------------

pub const GEN_AI_SYSTEM: &str = "gen_ai.system";
pub const GEN_AI_SYSTEM_VALUE: &str = "openjiuwen";
pub const GEN_AI_REQUEST_MODEL: &str = "gen_ai.request.model";
pub const GEN_AI_OPERATION_NAME: &str = "gen_ai.operation.name";
pub const GEN_AI_PROMPT: &str = "gen_ai.prompt";
pub const GEN_AI_COMPLETION: &str = "gen_ai.completion";
pub const GEN_AI_USAGE_PROMPT_TOKENS: &str = "gen_ai.usage.prompt_tokens";
pub const GEN_AI_USAGE_COMPLETION_TOKENS: &str = "gen_ai.usage.completion_tokens";
pub const GEN_AI_TOOL_NAME: &str = "gen_ai.tool.name";

pub const OJ_WORKFLOW_ID: &str = "openjiuwen.workflow.id";
pub const OJ_WORKFLOW_NAME: &str = "openjiuwen.workflow.name";
pub const OJ_WORKFLOW_VERSION: &str = "openjiuwen.workflow.version";
pub const OJ_WORKFLOW_COMPONENT_ID: &str = "openjiuwen.workflow.component.id";
pub const OJ_WORKFLOW_COMPONENT_TYPE: &str = "openjiuwen.workflow.component.type";
pub const OJ_WORKFLOW_COMPONENT_NAME: &str = "openjiuwen.workflow.component.name";
pub const OJ_WORKFLOW_EXECUTION_ID: &str = "openjiuwen.workflow.execution_id";
pub const OJ_WORKFLOW_LOOP_NODE_ID: &str = "openjiuwen.workflow.loop.node_id";
pub const OJ_WORKFLOW_LOOP_INDEX: &str = "openjiuwen.workflow.loop.index";

pub const OJ_AGENT_INVOKE_TYPE: &str = "openjiuwen.agent.invoke_type";
pub const OJ_AGENT_NAME: &str = "openjiuwen.agent.name";
pub const OJ_AGENT_INPUTS: &str = "openjiuwen.agent.inputs";
pub const OJ_AGENT_OUTPUTS: &str = "openjiuwen.agent.outputs";
pub const OJ_AGENT_ERROR_MESSAGE: &str = "openjiuwen.agent.error_message";

pub const OJ_TRACE_ID: &str = "openjiuwen.trace.id";
pub const OJ_SESSION_ID: &str = "openjiuwen.session_id";

pub const OJ_INVOKE_ID: &str = "openjiuwen.invoke_id";
pub const OJ_PARENT_INVOKE_ID: &str = "openjiuwen.parent_invoke_id";
pub const OJ_START_TIME: &str = "openjiuwen.start_time";
pub const OJ_END_TIME: &str = "openjiuwen.end_time";
pub const OJ_ELAPSED_TIME: &str = "openjiuwen.elapsed_time";
pub const OJ_STATUS: &str = "openjiuwen.status";
pub const OJ_ERROR: &str = "openjiuwen.error";
pub const OJ_CHILD_INVOKE_IDS: &str = "openjiuwen.child_invoke_ids";
pub const OJ_META_DATA: &str = "openjiuwen.meta_data";

pub const OJ_PARENT_NODE_ID: &str = "openjiuwen.parent_node_id";
pub const OJ_SOURCE_IDS: &str = "openjiuwen.source_ids";
pub const OJ_INNER_ERROR: &str = "openjiuwen.inner_error";
pub const OJ_STREAM_INPUTS: &str = "openjiuwen.stream_inputs";
pub const OJ_STREAM_OUTPUTS: &str = "openjiuwen.stream_outputs";
pub const OJ_INTERACTIVE_INPUTS: &str = "openjiuwen.interactive_inputs";
pub const OJ_WORKFLOW_INPUTS: &str = "openjiuwen.workflow.inputs";
pub const OJ_WORKFLOW_OUTPUTS: &str = "openjiuwen.workflow.outputs";
pub const OJ_WORKFLOW_ERROR_MESSAGE: &str = "openjiuwen.workflow.error_message";
pub const OJ_WORKFLOW_INVOKE_DATA: &str = "openjiuwen.workflow.invoke_data";

// ---------------------------------------------------------------------------
// span managers(对齐 span_manager.py)
// ---------------------------------------------------------------------------

/// 一个 OTel span 的可确定性状态视图(span 本体由插件持有)。
#[derive(Debug, Clone, PartialEq)]
pub struct OtelSpanState {
    pub invoke_id: String,
    /// 缓存的开始时间(毫秒 UTC;用于耗时计算)。
    pub start_time_ms: Option<i64>,
}

/// 代理 span 管理器:invoke_id → OtelSpanState(对齐 OtelAgentSpanManager)。
#[derive(Debug, Clone, Default)]
pub struct OtelAgentSpanManager {
    spans: HashMap<String, OtelSpanState>,
}

impl OtelAgentSpanManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, invoke_id: String, state: OtelSpanState) {
        self.spans.insert(invoke_id, state);
    }

    pub fn pop(&mut self, invoke_id: &str) -> Option<OtelSpanState> {
        self.spans.remove(invoke_id)
    }

    pub fn get(&self, invoke_id: &str) -> Option<&OtelSpanState> {
        self.spans.get(invoke_id)
    }
}

/// 工作流 span 管理器:span 状态 + 增量数据缓冲(对齐 OtelWorkflowSpanManager)。
#[derive(Debug, Clone, Default)]
pub struct OtelWorkflowSpanManager {
    spans: HashMap<String, OtelSpanState>,
    on_invoke_data: HashMap<String, Vec<Value>>,
    stream_inputs: HashMap<String, Vec<Value>>,
    stream_outputs: HashMap<String, Vec<Value>>,
}

impl OtelWorkflowSpanManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, invoke_id: String, state: OtelSpanState) {
        self.spans.insert(invoke_id.clone(), state);
        self.on_invoke_data.insert(invoke_id.clone(), Vec::new());
        self.stream_inputs.insert(invoke_id.clone(), Vec::new());
        self.stream_outputs.insert(invoke_id, Vec::new());
    }

    pub fn pop(&mut self, invoke_id: &str) -> Option<OtelSpanState> {
        self.on_invoke_data.remove(invoke_id);
        self.stream_inputs.remove(invoke_id);
        self.stream_outputs.remove(invoke_id);
        self.spans.remove(invoke_id)
    }

    pub fn get(&self, invoke_id: &str) -> Option<&OtelSpanState> {
        self.spans.get(invoke_id)
    }

    pub fn append_on_invoke_data(&mut self, invoke_id: &str, data: Value) {
        if let Some(buf) = self.on_invoke_data.get_mut(invoke_id) {
            buf.push(data);
        }
    }

    pub fn get_on_invoke_data(&self, invoke_id: &str) -> &[Value] {
        self.on_invoke_data
            .get(invoke_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn append_stream_input(&mut self, invoke_id: &str, chunk: Value) {
        if let Some(buf) = self.stream_inputs.get_mut(invoke_id) {
            buf.push(chunk);
        }
    }

    pub fn get_stream_inputs(&self, invoke_id: &str) -> &[Value] {
        self.stream_inputs
            .get(invoke_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn append_stream_output(&mut self, invoke_id: &str, chunk: Value) {
        if let Some(buf) = self.stream_outputs.get_mut(invoke_id) {
            buf.push(chunk);
        }
    }

    pub fn get_stream_outputs(&self, invoke_id: &str) -> &[Value] {
        self.stream_outputs
            .get(invoke_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

// ---------------------------------------------------------------------------
// setup 决策(对齐 setup.py)
// ---------------------------------------------------------------------------

/// 导出器决策。
#[derive(Debug, Clone, PartialEq)]
pub enum ExporterPlan {
    /// ConsoleSpanExporter + SimpleSpanProcessor。
    Console,
    /// OTLPSpanExporter + BatchSpanProcessor(带参数)。
    Otlp {
        endpoint: Option<String>,
        protocol: String,
        headers: std::collections::BTreeMap<String, String>,
        schedule_delay_millis: u64,
        export_timeout_ms: u64,
        max_export_batch_size: u64,
    },
}

/// 解析导出器计划(对齐 `init_otel_tracer` 的 exporter/protocol 决策)。
///
/// - exporter_type: console → Console;otlp → Otlp(需校验 protocol);
///   未知 → Err;
/// - protocol: grpc / http;未知 → Err;
/// - http:endpoint 存在且不以 `/v1/traces` 结尾 → 补后缀。
pub fn resolve_exporter(config: &OtelTracerConfig) -> Result<ExporterPlan, OtelTracerError> {
    match config.exporter_type.as_str() {
        "console" => Ok(ExporterPlan::Console),
        "otlp" => {
            let endpoint = config.exporter_endpoint.clone();
            let endpoint = match config.protocol.as_str() {
                "http" => endpoint.map(|e| {
                    if !e.ends_with("/v1/traces") {
                        format!("{e}/v1/traces")
                    } else {
                        e
                    }
                }),
                "grpc" => endpoint,
                other => {
                    return Err(OtelTracerError(format!(
                        "unknown otlp protocol '{other}', supported: grpc, http"
                    )));
                }
            };
            Ok(ExporterPlan::Otlp {
                endpoint,
                protocol: config.protocol.clone(),
                headers: config.headers.clone(),
                schedule_delay_millis: config.schedule_delay_millis,
                export_timeout_ms: config.export_timeout_ms,
                max_export_batch_size: config.max_export_batch_size,
            })
        }
        other => Err(OtelTracerError(format!(
            "unknown exporter_type '{other}', supported: console, otlp"
        ))),
    }
}

// ---------------------------------------------------------------------------
// handler 确定性属性映射(对齐 handler.py)
// ---------------------------------------------------------------------------

/// LLM 相关组件类型子串(对齐 `_LLM_SUBSTRINGS`)。
pub const LLM_SUBSTRINGS: &[&str] = &["LLM", "IntentDetection", "Questioner"];
/// 工具组件类型子串(对齐 `_TOOL_SUBSTRINGS`)。
pub const TOOL_SUBSTRINGS: &[&str] = &["Tool"];

/// 是否 LLM 相关组件(子串匹配;对齐 handler 的 `any(s in component_type ...)`)。
pub fn is_llm_component(component_type: &str) -> bool {
    LLM_SUBSTRINGS.iter().any(|s| component_type.contains(s))
}

/// 是否工具组件。
pub fn is_tool_component(component_type: &str) -> bool {
    TOOL_SUBSTRINGS.iter().any(|s| component_type.contains(s))
}

/// 是否为工作流根(metadata 含 workflow_id 且无 component_id;对齐
/// `is_workflow_root = "workflow_id" in metadata and "component_id" not in metadata`)。
pub fn is_workflow_root(metadata: &Map<String, Value>) -> bool {
    metadata.contains_key("workflow_id") && !metadata.contains_key("component_id")
}

/// 耗时格式化(对齐 `_set_end_attrs` / `_set_workflow_end_attrs`):
/// <1000ms → "Nms",否则 "N.NNs"。
pub fn format_elapsed(elapsed_ms: f64) -> String {
    if elapsed_ms < 1000.0 {
        format!("{elapsed_ms:.0}ms")
    } else {
        format!("{:.2}s", elapsed_ms / 1000.0)
    }
}

/// 工作流 / 组件属性构建(对齐 `_set_workflow_attrs`;返回键→值)。
pub fn workflow_attrs(
    metadata: Option<&Map<String, Value>>,
    invoke_id: &str,
) -> Map<String, Value> {
    let mut attrs = Map::new();
    let Some(metadata) = metadata else {
        return attrs;
    };
    if is_workflow_root(metadata) {
        insert_str(&mut attrs, OJ_WORKFLOW_ID, metadata, "workflow_id", "");
        insert_str(&mut attrs, OJ_WORKFLOW_NAME, metadata, "workflow_name", "");
        insert_str(
            &mut attrs,
            OJ_WORKFLOW_VERSION,
            metadata,
            "workflow_version",
            "",
        );
        // execution_id = workflow_id or invoke_id。
        let execution_id = metadata
            .get("workflow_id")
            .and_then(Value::as_str)
            .unwrap_or(invoke_id);
        attrs.insert(
            OJ_WORKFLOW_EXECUTION_ID.to_string(),
            Value::String(execution_id.to_string()),
        );
    } else {
        insert_str(
            &mut attrs,
            OJ_WORKFLOW_COMPONENT_ID,
            metadata,
            "component_id",
            "",
        );
        insert_str(
            &mut attrs,
            OJ_WORKFLOW_COMPONENT_TYPE,
            metadata,
            "component_type",
            "",
        );
        insert_str(
            &mut attrs,
            OJ_WORKFLOW_COMPONENT_NAME,
            metadata,
            "component_name",
            "",
        );
        insert_str(&mut attrs, OJ_WORKFLOW_ID, metadata, "workflow_id", "");
        if let Some(loop_node_id) = metadata.get("loop_node_id").and_then(Value::as_str) {
            attrs.insert(
                OJ_WORKFLOW_LOOP_NODE_ID.to_string(),
                Value::String(loop_node_id.to_string()),
            );
        }
        if let Some(loop_index) = metadata.get("loop_index") {
            attrs.insert(
                OJ_WORKFLOW_LOOP_INDEX.to_string(),
                Value::String(loop_index.to_string()),
            );
        }
    }
    attrs
}

fn insert_str(
    attrs: &mut Map<String, Value>,
    key: &str,
    metadata: &Map<String, Value>,
    field: &str,
    default: &str,
) {
    let value = metadata
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or(default);
    attrs.insert(key.to_string(), Value::String(value.to_string()));
}

/// 工作流调用开始时的基础属性(对齐 `on_call_start` 的公共属性)。
#[allow(clippy::too_many_arguments)]
pub fn workflow_call_start_attrs(
    trace_id: &str,
    session_id: &str,
    invoke_id: &str,
    parent_node_id: &str,
    start_time_str: &str,
    source_ids: Option<&Value>,
    component_type: &str,
    metadata: Option<&Map<String, Value>>,
) -> Map<String, Value> {
    let mut attrs = Map::new();
    attrs.insert(
        GEN_AI_SYSTEM.to_string(),
        Value::String(GEN_AI_SYSTEM_VALUE.to_string()),
    );
    attrs.insert(OJ_TRACE_ID.to_string(), Value::String(trace_id.to_string()));
    if !session_id.is_empty() {
        attrs.insert(
            OJ_SESSION_ID.to_string(),
            Value::String(session_id.to_string()),
        );
    }
    attrs.insert(
        OJ_INVOKE_ID.to_string(),
        Value::String(invoke_id.to_string()),
    );
    attrs.insert(
        OJ_PARENT_NODE_ID.to_string(),
        Value::String(parent_node_id.to_string()),
    );
    attrs.insert(
        OJ_START_TIME.to_string(),
        Value::String(start_time_str.to_string()),
    );
    if let Some(source_ids) = source_ids {
        attrs.insert(
            OJ_SOURCE_IDS.to_string(),
            Value::String(serialize_value(source_ids)),
        );
    }
    if is_llm_component(component_type) {
        attrs.insert(
            GEN_AI_OPERATION_NAME.to_string(),
            Value::String("chat".to_string()),
        );
    } else if is_tool_component(component_type) {
        attrs.insert(
            GEN_AI_OPERATION_NAME.to_string(),
            Value::String("execute_tool".to_string()),
        );
    }
    let wf = workflow_attrs(metadata, invoke_id);
    for (k, v) in wf {
        attrs.insert(k, v);
    }
    attrs
}

/// LLM 组件 → SpanKind.CLIENT;其余 → INTERNAL(对齐 handler 的 span_kind 决策)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtelSpanKind {
    Internal,
    Client,
}

/// 解析 span kind(LLM 组件 → Client)。
pub fn span_kind_for_component(component_type: &str) -> OtelSpanKind {
    if is_llm_component(component_type) {
        OtelSpanKind::Client
    } else {
        OtelSpanKind::Internal
    }
}

/// 工作流 span 命名(根 → invoke_id;组件 → component.{invoke_id};
/// 对齐 `span_name = invoke_id if is_workflow_root else f"component.{invoke_id}"`)。
pub fn workflow_span_name(is_root: bool, invoke_id: &str) -> String {
    if is_root {
        invoke_id.to_string()
    } else {
        format!("component.{invoke_id}")
    }
}

/// 父上下文解析决策(对齐 `_resolve_parent_context`)。
///
/// 返回父引用来源:None(顶级根)/ RootLayerRoot(根工作流根 span)/
/// Component(node_id 对应的宿主组件 span)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParentContextRef {
    /// 顶级工作流根 — 无父。
    None,
    /// 根工作流中的组件 → 父 = 根工作流根 span。
    RootLayerRoot,
    /// 子工作流根或组件 → 父 = 宿主组件 span。
    Component(String),
}

/// 按 parent_node_id + metadata 解析父上下文引用
/// (对齐 `_resolve_parent_context` 的四分支)。
pub fn resolve_parent_context(
    parent_node_id: &str,
    metadata: Option<&Map<String, Value>>,
) -> ParentContextRef {
    let is_root = metadata.map(is_workflow_root).unwrap_or(false);
    if parent_node_id.is_empty() && is_root {
        ParentContextRef::None
    } else if parent_node_id.is_empty() {
        ParentContextRef::RootLayerRoot
    } else {
        ParentContextRef::Component(parent_node_id.to_string())
    }
}

/// tracer-otel Seam(Service Definition):OTel 追踪扩展门面。
///
/// 实现方(插件)持有真实 OTel tracer 与导出器;消费方(tracer 基础设施)
/// 只依赖本 trait 的确定性属性/决策面。
pub trait OtelTracer: Seam {
    /// 校验配置(sample_rate 范围)。
    fn validate(&self, config: &OtelTracerConfig) -> Result<(), OtelTracerError>;

    /// 解析导出器计划。
    fn resolve_exporter(&self, config: &OtelTracerConfig) -> Result<ExporterPlan, OtelTracerError>;

    /// 应用脱敏(启用 → 哈希;禁用 → 截断)。
    fn redact(
        &self,
        value: Option<&Value>,
        config: &OtelTracerConfig,
        field: Option<&str>,
    ) -> String;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sample_rate_validation() {
        assert!(validate_sample_rate(0.0).is_ok());
        assert!(validate_sample_rate(1.0).is_ok());
        assert!(validate_sample_rate(0.5).is_ok());
        assert!(validate_sample_rate(-0.1).is_err());
        assert!(validate_sample_rate(1.1).is_err());
    }

    #[test]
    fn truncate_and_hash() {
        assert_eq!(truncate("abc", 5), "abc");
        assert_eq!(truncate("abc", 0), "abc");
        let long = "x".repeat(100);
        let t = truncate(&long, 10);
        assert_eq!(t, format!("{}...<truncated>", "x".repeat(10)));
        // 中文按字符计。
        assert_eq!(truncate("你好世界", 2), "你好...<truncated>");
        // 哈希:sha256: + 16 hex。
        let h = hash_value("hello");
        assert!(h.starts_with("sha256:"));
        assert_eq!(h.len(), "sha256:".len() + 16);
        // 确定性。
        assert_eq!(hash_value("hello"), hash_value("hello"));
        assert_ne!(hash_value("hello"), hash_value("world"));
    }

    #[test]
    fn should_redact_resolution() {
        let config = OtelTracerConfig::default(); // redaction_enabled=true
        assert!(should_redact(&config, None));
        assert!(should_redact(&config, Some("prompts")));
        let mut config = config;
        config.redact_prompts = Some(false);
        assert!(!should_redact(&config, Some("prompts")));
        assert!(should_redact(&config, Some("completions")));
        assert!(should_redact(&config, None));
        config.redact_completions = Some(false);
        assert!(!should_redact(&config, Some("completions")));
        config.redaction_enabled = false;
        assert!(!should_redact(&config, None));
    }

    #[test]
    fn redact_hashes_or_truncates() {
        let config = OtelTracerConfig::default();
        let hashed = redact(Some(&json!("secret")), &config, None);
        assert!(hashed.starts_with("sha256:"));
        let mut config = config;
        config.redaction_enabled = false;
        let plain = redact(Some(&json!("hello")), &config, None);
        assert_eq!(plain, "hello");
        // None → 空串。
        assert_eq!(redact(None, &config, None), "");
    }

    #[test]
    fn span_managers_lifecycle() {
        let mut agent = OtelAgentSpanManager::new();
        agent.push(
            "i1".to_string(),
            OtelSpanState {
                invoke_id: "i1".to_string(),
                start_time_ms: Some(1),
            },
        );
        assert!(agent.get("i1").is_some());
        assert!(agent.pop("i1").is_some());
        assert!(agent.get("i1").is_none());

        let mut wf = OtelWorkflowSpanManager::new();
        wf.push(
            "i2".to_string(),
            OtelSpanState {
                invoke_id: "i2".to_string(),
                start_time_ms: None,
            },
        );
        wf.append_on_invoke_data("i2", json!({"a": 1}));
        wf.append_stream_input("i2", json!({"chunk": 1}));
        wf.append_stream_output("i2", json!({"out": 1}));
        assert_eq!(wf.get_on_invoke_data("i2").len(), 1);
        assert_eq!(wf.get_stream_inputs("i2").len(), 1);
        assert_eq!(wf.get_stream_outputs("i2").len(), 1);
        // 未注册 invoke_id → 缓冲忽略。
        wf.append_on_invoke_data("nope", json!({}));
        assert!(wf.get_on_invoke_data("nope").is_empty());
        // pop 清理缓冲。
        assert!(wf.pop("i2").is_some());
        assert!(wf.get_on_invoke_data("i2").is_empty());
    }

    #[test]
    fn exporter_resolution() {
        // console。
        let config = OtelTracerConfig {
            exporter_type: "console".to_string(),
            ..Default::default()
        };
        assert_eq!(resolve_exporter(&config).unwrap(), ExporterPlan::Console);
        // otlp grpc。
        let config = OtelTracerConfig {
            exporter_type: "otlp".to_string(),
            protocol: "grpc".to_string(),
            exporter_endpoint: Some("http://collector:4317".to_string()),
            ..Default::default()
        };
        let plan = resolve_exporter(&config).unwrap();
        assert!(matches!(plan, ExporterPlan::Otlp { .. }));
        // http 补 /v1/traces。
        let config = OtelTracerConfig {
            exporter_type: "otlp".to_string(),
            protocol: "http".to_string(),
            exporter_endpoint: Some("http://collector:4318".to_string()),
            ..Default::default()
        };
        let ExporterPlan::Otlp { endpoint, .. } = resolve_exporter(&config).unwrap() else {
            panic!("otlp");
        };
        assert_eq!(endpoint.as_deref(), Some("http://collector:4318/v1/traces"));
        // 已带 /v1/traces 不重复。
        let config = OtelTracerConfig {
            exporter_type: "otlp".to_string(),
            protocol: "http".to_string(),
            exporter_endpoint: Some("http://c/v1/traces".to_string()),
            ..Default::default()
        };
        let ExporterPlan::Otlp { endpoint, .. } = resolve_exporter(&config).unwrap() else {
            panic!("otlp");
        };
        assert_eq!(endpoint.as_deref(), Some("http://c/v1/traces"));
        // 未知类型 / 协议。
        let config = OtelTracerConfig {
            exporter_type: "nope".to_string(),
            ..Default::default()
        };
        assert!(resolve_exporter(&config).is_err());
        let config = OtelTracerConfig {
            exporter_type: "otlp".to_string(),
            protocol: "udp".to_string(),
            ..Default::default()
        };
        assert!(resolve_exporter(&config).is_err());
    }

    #[test]
    fn workflow_attr_building() {
        let mut metadata = Map::new();
        metadata.insert("workflow_id".to_string(), json!("w1"));
        metadata.insert("workflow_name".to_string(), json!("Build"));
        metadata.insert("workflow_version".to_string(), json!("1.0"));
        let attrs = workflow_attrs(Some(&metadata), "inv-1");
        assert_eq!(attrs[OJ_WORKFLOW_ID], "w1");
        assert_eq!(attrs[OJ_WORKFLOW_EXECUTION_ID], "w1");
        // 无 workflow_id 时 execution_id 回退 invoke_id。
        let mut metadata2 = Map::new();
        metadata2.insert("component_id".to_string(), json!("c1"));
        metadata2.insert("component_type".to_string(), json!("LLMComponent"));
        metadata2.insert("loop_index".to_string(), json!(3));
        let attrs2 = workflow_attrs(Some(&metadata2), "inv-2");
        assert_eq!(attrs2[OJ_WORKFLOW_COMPONENT_ID], "c1");
        assert_eq!(attrs2[OJ_WORKFLOW_LOOP_INDEX], "3");
        assert!(attrs2.get(OJ_WORKFLOW_EXECUTION_ID).is_none());
        assert_eq!(attrs2[OJ_WORKFLOW_ID], "");
    }

    #[test]
    fn component_classification_and_span_name() {
        assert!(is_llm_component("LLMComponent"));
        assert!(is_llm_component("IntentDetectionComponent"));
        assert!(is_llm_component("QuestionerComponent"));
        assert!(!is_llm_component("ToolExecutable"));
        assert!(is_tool_component("ToolExecutable"));
        assert_eq!(
            span_kind_for_component("LLMComponent"),
            OtelSpanKind::Client
        );
        assert_eq!(
            span_kind_for_component("ToolExecutable"),
            OtelSpanKind::Internal
        );
        assert_eq!(workflow_span_name(true, "inv"), "inv");
        assert_eq!(workflow_span_name(false, "inv"), "component.inv");
    }

    #[test]
    fn parent_context_resolution() {
        let mut root = Map::new();
        root.insert("workflow_id".to_string(), json!("w1"));
        assert_eq!(
            resolve_parent_context("", Some(&root)),
            ParentContextRef::None
        );
        assert_eq!(
            resolve_parent_context("", None),
            ParentContextRef::RootLayerRoot
        );
        let mut comp = Map::new();
        comp.insert("component_id".to_string(), json!("c1"));
        assert_eq!(
            resolve_parent_context("c1", Some(&comp)),
            ParentContextRef::Component("c1".to_string())
        );
    }

    #[test]
    fn elapsed_formatting() {
        assert_eq!(format_elapsed(500.0), "500ms");
        assert_eq!(format_elapsed(1500.0), "1.50s");
        assert_eq!(format_elapsed(999.0), "999ms");
    }

    #[test]
    fn call_start_attrs() {
        let mut metadata = Map::new();
        metadata.insert("workflow_id".to_string(), json!("w1"));
        let attrs = workflow_call_start_attrs(
            "trace-1",
            "sess-1",
            "inv-1",
            "",
            "2026-01-01 00:00:00",
            Some(&json!(["a", "b"])),
            "LLMComponent",
            Some(&metadata),
        );
        assert_eq!(attrs[GEN_AI_SYSTEM], "openjiuwen");
        assert_eq!(attrs[GEN_AI_OPERATION_NAME], "chat");
        assert_eq!(attrs[OJ_TRACE_ID], "trace-1");
        assert_eq!(attrs[OJ_SESSION_ID], "sess-1");
        assert_eq!(attrs[OJ_INVOKE_ID], "inv-1");
        assert_eq!(attrs[OJ_PARENT_NODE_ID], "");
        assert_eq!(attrs[OJ_SOURCE_IDS], "[\"a\",\"b\"]");
        assert_eq!(attrs[OJ_WORKFLOW_ID], "w1");
        // 工具组件 → execute_tool。
        let tool_attrs =
            workflow_call_start_attrs("t", "", "i", "", "s", None, "ToolExecutable", None);
        assert_eq!(tool_attrs[GEN_AI_OPERATION_NAME], "execute_tool");
        // 空 session_id → 不设 OJ_SESSION_ID。
        assert!(tool_attrs.get(OJ_SESSION_ID).is_none());
    }
}
