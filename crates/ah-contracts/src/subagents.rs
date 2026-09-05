//! subagents seam:类型化子代理(code/research/plan/verify/browser/mobile_gui)。
//!
//! 每种类型有专属 system 提示与工具白名单;run 时注入提示并经
//! `SubagentSpec.allowed_tools` 真实过滤工具(白名单外调用被拒)。

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
    /// Python `browser_agent` factory equivalent.
    Browser,
    /// Python `mobile_gui_agent` factory equivalent.
    MobileGui,
}

impl SubagentKind {
    /// Stable factory/subagent name used on spawn wires and in profiles.
    pub const fn factory_name(self) -> &'static str {
        match self {
            Self::Code => "code_agent",
            Self::Research => "research_agent",
            Self::Plan => "plan_agent",
            Self::Verify => "verification_agent",
            Self::Browser => "browser_agent",
            Self::MobileGui => "mobile_gui_agent",
        }
    }
}

/// 类型档案。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SubagentProfile {
    pub kind: SubagentKind,
    /// 类型专属 system 提示。
    pub system_prompt: String,
    /// 工具白名单(空 = 全部工具)。
    pub allowed_tools: Vec<String>,
    /// 默认循环预算;builder 未显式传预算时使用。
    #[serde(default)]
    pub default_budget: usize,
}

/// 一组子代理执行请求,用于父会话继承和 task 聚合。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SubagentRequest {
    pub kind: SubagentKind,
    pub task: String,
    #[serde(default)]
    pub budget: Option<usize>,
    /// Optional parent session whose model-visible history is inherited.
    #[serde(default)]
    pub parent_session_id: Option<String>,
    /// Browser capability categories; only meaningful for `browser_agent`.
    #[serde(default)]
    pub browser_capabilities: Vec<String>,
    /// Android device identity; only meaningful for `mobile_gui_agent`.
    #[serde(default)]
    pub device_serial: Option<String>,
}

/// 聚合后的子代理执行摘要。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SubagentAggregate {
    pub total: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub iterations_used: usize,
    pub answers: Vec<String>,
}

impl SubagentAggregate {
    pub fn empty() -> Self {
        Self {
            total: 0,
            succeeded: 0,
            failed: 0,
            iterations_used: 0,
            answers: Vec::new(),
        }
    }
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

    /// Run with parent-context inheritance and factory-specific settings.
    async fn run_request(
        &self,
        request: SubagentRequest,
    ) -> Result<SubagentResult, TypedSubagentError> {
        self.run(request.kind, &request.task, request.budget).await
    }

    /// Execute requests independently and retain successful answers in order.
    async fn run_many(&self, requests: Vec<SubagentRequest>) -> SubagentAggregate {
        let total = requests.len();
        let mut aggregate = SubagentAggregate {
            total,
            ..SubagentAggregate::empty()
        };
        for request in requests {
            match self.run_request(request).await {
                Ok(result) => {
                    aggregate.succeeded += 1;
                    aggregate.iterations_used += result.iterations_used;
                    aggregate.answers.push(result.answer);
                }
                Err(_) => aggregate.failed += 1,
            }
        }
        aggregate
    }
}
