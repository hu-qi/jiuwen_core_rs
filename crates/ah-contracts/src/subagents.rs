//! subagents seam:类型化子代理(code/research/plan/verify)。
//!
//! 每种类型有专属 system 提示与工具白名单;run 时注入提示并经
//! SubagentSpec.allowed_tools 真实过滤工具(白名单外调用被拒)。

use async_trait::async_trait;

use crate::seam::Seam;
use crate::subagent::SubagentResult;

/// 子代理类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentKind {
    Code,
    Research,
    Plan,
    Verify,
}

/// 类型档案。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SubagentProfile {
    pub kind: SubagentKind,
    /// 类型专属 system 提示。
    pub system_prompt: String,
    /// 工具白名单(空 = 全部工具)。
    pub allowed_tools: Vec<String>,
}

/// subagents 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedSubagentError(pub String);

impl core::fmt::Display for TypedSubagentError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TypedSubagentError {}

/// subagents Seam(Service Definition):类型化子代理运行。
#[async_trait]
pub trait TypedSubagents: Seam {
    /// 全部类型。
    fn kinds(&self) -> Vec<SubagentKind>;

    /// 类型的档案(提示 + 白名单)。
    fn profile(&self, kind: SubagentKind) -> Option<SubagentProfile>;

    /// 运行指定类型子代理(注入类型提示;白名单真实生效)。
    async fn run(
        &self,
        kind: SubagentKind,
        task: &str,
        budget: Option<usize>,
    ) -> Result<SubagentResult, TypedSubagentError>;
}
