//! security seam:可复用的安全检测(guardrail)。

use std::sync::Arc;

use crate::effect::Effect;
use crate::seam::Seam;

/// 风险级别。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Low,
    Medium,
    High,
}

/// 一条 guardrail 决策。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GuardrailDecision {
    pub guardrail: String,
    pub allow: bool,
    pub severity: Severity,
    pub reason: String,
}

/// 组合安全裁决。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SecurityVerdict {
    /// 任一 guardrail 拒绝则不允许。
    pub allow: bool,
    pub decisions: Vec<GuardrailDecision>,
}

/// 安全错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityError(pub String);

impl core::fmt::Display for SecurityError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SecurityError {}

/// 单个 guardrail(规则后端)。
pub trait Guardrail: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    fn check(&self, content: &str) -> GuardrailDecision;
}

/// security Seam(Service Definition):guardrail 集合与组合检测。
pub trait SecurityProvider: Seam {
    /// 注册一个 guardrail(可逆)。
    fn register(&self, guardrail: Arc<dyn Guardrail>) -> Effect;

    /// 全部决策(每个 guardrail 一条)。
    fn check(&self, content: &str) -> Vec<GuardrailDecision>;

    /// 组合裁决:任一拒绝即不允许。
    fn verdict(&self, content: &str) -> SecurityVerdict;
}
