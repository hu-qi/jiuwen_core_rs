//! pregel seam:超级步图执行(状态通道 + 条件触发 + 中断)。
//!
//! 对应 openjiuwen/core 的 graph/Pregel:图节点共享状态(状态通道),按超级步
//! 推进——每步执行所有条件满足的节点,把输出写入状态;无可触发节点或达
//! 上限即停;节点可配置 halt 中断并返回部分状态。

use async_trait::async_trait;
use serde_json::Value;

use crate::seam::Seam;

/// 图节点类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PregelNodeKind {
    /// 调用工具(config: {tool, args});输出写入 state[node.id]。
    Tool,
    /// 调用 LLM(config: {prompt});输出写入 state[node.id]。
    Llm,
    /// 纯函数占位(无副作用;用于中断点或编排)。
    Noop,
}

/// 图节点。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PregelNodeSpec {
    pub id: String,
    pub kind: PregelNodeKind,
    #[serde(default)]
    pub config: Value,
}

/// 图边(条件同 workflow:always / equals / not_equals)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PregelEdge {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub condition: Option<Value>,
}

/// 图定义。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PregelGraph {
    pub id: String,
    pub nodes: Vec<PregelNodeSpec>,
    pub edges: Vec<PregelEdge>,
    pub max_supersteps: u32,
}

/// 执行结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PregelResult {
    /// 最终状态(每节点输出写入 state[node.id])。
    pub state: Value,
    pub supersteps: u32,
    /// 中断点(命中 halt 配置的节点 id)。
    pub interrupted_at: Option<String>,
}

/// pregel 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PregelError(pub String);

impl core::fmt::Display for PregelError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PregelError {}

/// pregel Seam(Service Definition):超级步图执行。
#[async_trait]
pub trait PregelEngine: Seam {
    /// 执行图:超级步推进直到无节点触发/达上限/中断。
    async fn run(
        &self,
        graph: &PregelGraph,
        initial_state: Value,
    ) -> Result<PregelResult, PregelError>;
}
