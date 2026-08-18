//! reliability-detectors seam:可靠性检测器(对齐 openjiuwen/agent_teams/reliability/)。
//!
//! 共享数据模型与 Detector 契约:
//! - `Signal`:成员执行生命周期点的一条观测(kind/member_name + 可选字段);
//! - `Anomaly`:阈值触发时检测器产出的一条异常(detector/kind/severity/summary/evidence);
//! - `Severity`:LOW→MEDIUM→HIGH→CRITICAL(rank 0..3);
//! - `Detector` trait:observe(signal) → Option<Anomaly>(边沿触发,severity 上升才发)+ reset。
//!
//! 具体检测器(突发/重复/超长等)由插件提供(如 ah-plugins-reliability-burst / -tools)。

use crate::seam::Seam;

/// 异常严重度(对齐 Severity,rank 0..3)。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    pub fn rank(self) -> u8 {
        match self {
            Severity::Low => 0,
            Severity::Medium => 1,
            Severity::High => 2,
            Severity::Critical => 3,
        }
    }
}

/// 异常种类(对齐 AnomalyKind)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnomalyKind {
    ToolErrorRate,
    RepeatToolCall,
    ToolCallLoop,
    ModelError,
    OutputTooLong,
    ThinkingTooLong,
    FrequentCompaction,
    PingPong,
}

/// 信号生命周期点(对齐 SignalKind)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalKind {
    BeforeToolCall,
    AfterToolCall,
    ToolException,
    ModelException,
    AfterModelCall,
    BeforeModelCall,
    Message,
}

/// 一条观测(对齐 Signal;可选字段按 kind 填充,检测器需容忍 None)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Signal {
    pub kind: SignalKind,
    pub member_name: String,
    pub tool_name: Option<String>,
    pub tool_args: Option<serde_json::Value>,
    pub error: Option<String>,
    pub text_len: Option<u64>,
    pub thinking_len: Option<u64>,
    pub message_count: Option<u64>,
    pub peer_member: Option<String>,
    pub tool_result: Option<serde_json::Value>,
}

impl Signal {
    pub fn new(kind: SignalKind, member_name: impl Into<String>) -> Self {
        Self {
            kind,
            member_name: member_name.into(),
            tool_name: None,
            tool_args: None,
            error: None,
            text_len: None,
            thinking_len: None,
            message_count: None,
            peer_member: None,
            tool_result: None,
        }
    }
}

/// 一条异常(对齐 Anomaly)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Anomaly {
    pub detector: String,
    pub kind: AnomalyKind,
    pub severity: Severity,
    pub member_name: String,
    pub summary: String,
    pub evidence: serde_json::Map<String, serde_json::Value>,
    pub peer_member: Option<String>,
}

/// 检测器 Seam(Service Definition)。
pub trait Detector: Seam {
    /// 稳定标识。
    fn name(&self) -> &str;

    /// 消费一条信号;阈值触发返回 Anomaly(边沿触发:severity 上升才发)。
    fn observe(&self, signal: &Signal) -> Option<Anomaly>;

    /// 重置本轮检测状态。
    fn reset(&self);
}
