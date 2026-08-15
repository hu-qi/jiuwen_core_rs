//! subagent seam:子任务委派(隔离会话 + 预算受限执行)。

use async_trait::async_trait;

use crate::seam::Seam;

/// 子代理规格。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SubagentSpec {
    /// 子任务 id(用于隔离会话命名)。
    pub id: String,
    /// 任务描述。
    pub task: String,
    /// 可选注入上下文(system 消息)。
    pub context: Option<String>,
    /// 预算:最大循环轮数;None 用默认。
    pub budget: Option<usize>,
    /// 工具白名单:Some 时模型只见白名单工具,越权调用被拒(真实强制);
    /// None = 全部注册工具。
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,
}

/// 子代理结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SubagentResult {
    pub answer: String,
    /// 实际使用的循环轮数。
    pub iterations_used: usize,
}

/// 子代理错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentError(pub String);

impl core::fmt::Display for SubagentError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SubagentError {}

/// subagent Seam(Service Definition):在隔离会话中执行子任务。
#[async_trait]
pub trait SubagentRuntime: Seam {
    async fn run(&self, spec: SubagentSpec) -> Result<SubagentResult, SubagentError>;
}
