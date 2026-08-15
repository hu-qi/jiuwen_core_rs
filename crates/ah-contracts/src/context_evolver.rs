//! context_evolver seam:任务记忆服务(对齐 Python extensions/context_evolver)。
//!
//! 为 agent 提供可演化的任务记忆:保存任务记忆(JSON 持久化)、按任务检索相关
//! 记忆(关键词+向量混合打分)、把多条轨迹凝练为摘要(确定性归纳)、注入上下文。

use crate::evolving::Trajectory;
use crate::seam::Seam;

/// 一条任务记忆。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TaskMemory {
    pub id: String,
    /// 关联任务/主题。
    pub task: String,
    pub content: String,
    /// 标签(检索用)。
    #[serde(default)]
    pub tags: Vec<String>,
    pub saved_ms: u64,
}

/// 轨迹摘要(确定性归纳)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TrajectorySummary {
    /// 源轨迹任务。
    pub task: String,
    /// 步数。
    pub steps: usize,
    /// 成功步数。
    pub succeeded: usize,
    /// 失败步数。
    pub failed: usize,
    /// 是否预算内完成。
    pub finished: bool,
    /// 关键点(成功工具名去重 + 失败信息)。
    pub key_points: Vec<String>,
}

/// 记忆注入结果(供上下文组装)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MemoryInjection {
    /// 相关记忆内容(按相关性降序)。
    pub memories: Vec<String>,
    /// 凝练摘要(如有轨迹)。
    pub summary: Option<TrajectorySummary>,
    /// 注入文本(可直接并入 system prompt)。
    pub injected_text: String,
}

/// context_evolver 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryEvolverError(pub String);

impl core::fmt::Display for MemoryEvolverError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MemoryEvolverError {}

/// context_evolver Seam(Service Definition):任务记忆保存/检索/摘要/注入。
pub trait MemoryEvolver: Seam {
    /// 保存一条任务记忆(真实 JSON 落盘)。
    fn save(
        &self,
        task: &str,
        content: &str,
        tags: Vec<String>,
    ) -> Result<TaskMemory, MemoryEvolverError>;

    /// 按任务检索相关记忆(关键词 + 标签打分,降序)。
    fn retrieve(&self, task: &str, limit: usize) -> Vec<TaskMemory>;

    /// 把轨迹凝练为摘要(确定性归纳,无 LLM)。
    fn summarize(&self, trajectories: &[Trajectory]) -> TrajectorySummary;

    /// 注入:检索相关记忆 + 凝练摘要 → 注入文本。
    fn inject(&self, task: &str, trajectories: &[Trajectory]) -> MemoryInjection;
}
