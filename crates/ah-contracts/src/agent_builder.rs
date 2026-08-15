//! agent-builder seam:NL 任务 → 设计 → DSL → 执行(agent_builder)。
//!
//! 真实确定性管线:design 从自然语言提取 goal/工具/步骤;to_dsl 生成
//! workflow 规格 JSON(经 WorkflowEngine 可执行);run 真实执行并返回输出。

use async_trait::async_trait;
use serde_json::Value;

use crate::seam::Seam;

/// 设计结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AgentDesign {
    pub name: String,
    pub goal: String,
    /// 从 NL 匹配出的工具名(基于已知工具词表)。
    pub tools: Vec<String>,
    /// 拆分出的步骤(按句子)。
    pub steps: Vec<String>,
}

/// agent-builder 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildError(pub String);

impl core::fmt::Display for BuildError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BuildError {}

/// agent-builder Seam(Service Definition):NL → 设计 → DSL → 执行。
#[async_trait]
pub trait AgentBuilder: Seam {
    /// 从自然语言任务生成设计(确定性意图解析)。
    fn design(&self, nl: &str) -> Result<AgentDesign, BuildError>;

    /// 设计 → 工作流 DSL(JSON WorkflowSpec)。
    fn to_dsl(&self, design: &AgentDesign) -> Result<Value, BuildError>;

    /// 执行 DSL(经 WorkflowEngine 真实运行),返回最终输出。
    async fn run(&self, dsl: &Value) -> Result<Value, BuildError>;
}
