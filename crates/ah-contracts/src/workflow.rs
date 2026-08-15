//! workflow seam:工作流执行引擎(组件 + 边 + 条件分支 + 循环)。

use async_trait::async_trait;
use serde_json::Value;

use crate::seam::Seam;

/// 工作流节点类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    /// 入口:透传输入。
    Start,
    /// 出口:产出最终输出。
    End,
    /// LLM 组件:调用 llm seam(config: {prompt} + 上一步输出作为用户消息)。
    Llm,
    /// 工具组件:调用 tools seam(config: {tool, args} + 上一步输出合并)。
    Tool,
    /// 循环组件:重复执行目标节点 N 次(config: {iterations, target})。
    Loop,
    /// 子工作流组件:执行内嵌 WorkflowSpec(config: {workflow: WorkflowSpec})。
    SubWorkflow,
    /// 并行组件:并发执行多个目标节点(config: {targets: [node_id...]})。
    Parallel,
    /// HTTP 组件:真实请求(config: {url, timeout_ms?}),输出 {status, body}。
    Http,
    /// 意图路由组件(config: {intents: [{id, description}], mode: "keyword"|"llm",
    /// patterns: {id: [keywords]}}),输出 {intent, confidence}。
    Intent,
    /// 提问组件(config: {question, timeout_ms?}):经 queue 发布问题并等待回答,
    /// 输出 {answer}。
    Questioner,
}

/// 节点规格。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NodeSpec {
    pub id: String,
    pub kind: NodeKind,
    #[serde(default)]
    pub config: Value,
}

/// 边(可带条件)。condition 约定:
/// - {"type":"always"} 或缺失:无条件;
/// - {"type":"equals","path":"a.b","value":X}:state 中 path 等于 X 才走;
/// - {"type":"not_equals","path":"a.b","value":X}:不等才走。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EdgeSpec {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub condition: Option<Value>,
}

/// 工作流规格。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WorkflowSpec {
    pub id: String,
    pub nodes: Vec<NodeSpec>,
    pub edges: Vec<EdgeSpec>,
}

/// 工作流输出。
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowOutput {
    /// 最终输出(End 节点的产出)。
    pub output: Value,
    /// 已执行节点序列。
    pub executed: Vec<String>,
}

/// 检查点续跑的完整结果。
#[derive(Debug, Clone, PartialEq)]
pub struct CheckpointedOutput {
    /// 本次新执行节点。
    pub executed: Vec<String>,
    /// 从检查点复用输出的节点。
    pub resumed: Vec<String>,
    /// 最终输出。
    pub output: Value,
}

/// 工作流错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowError(pub String);

impl core::fmt::Display for WorkflowError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for WorkflowError {}

/// 工作流执行引擎 Seam(Service Definition)。
#[async_trait]
pub trait WorkflowEngine: Seam {
    /// 执行工作流;组件真实调用 llm/tools seam,执行轨迹通过事件发布。
    async fn run(&self, spec: &WorkflowSpec, input: Value)
    -> Result<WorkflowOutput, WorkflowError>;

    /// 带检查点执行:每节点输出追加到 JSONL(checkpoint_path),已有记录直接复用;
    /// 返回新执行与复用的节点序列。
    async fn run_checkpointed(
        &self,
        spec: &WorkflowSpec,
        input: Value,
        checkpoint_path: &std::path::Path,
    ) -> Result<CheckpointedOutput, WorkflowError>;
}
