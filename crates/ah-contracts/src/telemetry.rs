//! telemetry seam:span 记录与导出(OTel 基础的本地落地)。
//!
//! 契约零实现:span 的存储与导出由插件提供(如 ah-plugins-telemetry 的
//! JSONL 文件导出)。OTLP/JSON HTTP 导出由 `ah-plugins-telemetry` 提供。

use serde_json::Value;

use crate::seam::Seam;

/// 一条 span:一段有名字、可嵌套、可携带属性的执行区间。
///
/// - `parent`:外层 span 的 id/name 字符串(树形重建用,None 为根);
/// - `attributes`:自由键值属性(如 workflow/agent/event 上下文);
/// - `start_ms` / `duration_ms`:毫秒时间戳与耗时。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Span {
    /// span 名(如 `agent/step#3`、`tool/read_file`)。
    pub name: String,
    /// 外层 span 的 id/name 字符串;根 span 为 None。
    pub parent: Option<String>,
    /// 键值属性(JSON 对象,可序列化落盘)。
    pub attributes: serde_json::Map<String, Value>,
    /// 开始时间(epoch 毫秒)。
    pub start_ms: u64,
    /// 耗时(毫秒)。
    pub duration_ms: u64,
}

impl Span {
    /// 构造一条无父级、空属性的 span。
    pub fn new(name: impl Into<String>, start_ms: u64, duration_ms: u64) -> Self {
        Self {
            name: name.into(),
            parent: None,
            attributes: serde_json::Map::new(),
            start_ms,
            duration_ms,
        }
    }
}

/// 遥测错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetryError(pub String);

impl core::fmt::Display for TelemetryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TelemetryError {}

/// 遥测 Seam(Service Definition):span 记录与导出。
///
/// 消费方(插件/宿主)通过 `telemetry` 服务键解析本 trait,
/// 记录 span 并在适当时机导出(返回本次导出的 span 条数)。
#[async_trait::async_trait]
pub trait TelemetryProvider: Seam {
    /// 记录一条 span(内存存储;是否落盘由实现决定)。
    fn record_span(&self, span: Span) -> Result<(), TelemetryError>;

    /// 当前全部已记录 span(含已导出)。
    fn spans(&self) -> Vec<Span>;

    /// 导出尚未导出的 span(如追加写入 JSONL 文件)并返回条数。
    async fn export(&self) -> Result<usize, TelemetryError>;

    /// 以 OTLP/JSON 编码把尚未导出的 span POST 到 OTLP collector,返回条数。
    async fn export_otlp(&self, collector_url: &str) -> Result<usize, TelemetryError>;
}
