//! rsi seam:递归自改进(数据生成 → 执行评测 → 报告 → 任务提示精化 → checkpoint)。
//!
//! 对应 openjiuwen/rsi 的 orchestrator / dataset_generator / evaluator /
//! evaluation_result_analyzer:真实管线,checkpoint 落盘支持中断续跑。

use async_trait::async_trait;

use crate::seam::Seam;

/// 一个评测用例(数据集条目)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RsiCase {
    pub id: String,
    /// 任务描述(喂给被评测的 agent)。
    pub task: String,
    /// 期望输出片段;Some 时按包含匹配判定,None 时按轨迹评估判定。
    pub expected: Option<String>,
}

/// 单个用例的执行结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RsiRunOutcome {
    pub case_id: String,
    pub passed: bool,
    /// 0..1 归一化得分。
    pub score: f64,
    pub answer: String,
    pub error: Option<String>,
}

/// 一轮评测报告。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RsiReport {
    pub round: u32,
    pub total: usize,
    pub passed: usize,
    pub avg_score: f64,
    pub issues: Vec<String>,
    pub summary: String,
}

/// checkpoint:一轮结束时的状态(支持中断续跑)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RsiCheckpoint {
    pub round: u32,
    pub cases: Vec<RsiCase>,
    /// 当前(可能已精化)的任务提示。
    pub task_prompt: String,
    pub updated_at_ms: u64,
}

/// rsi 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RsiError(pub String);

impl core::fmt::Display for RsiError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RsiError {}

/// rsi Seam(Service Definition):端到端自改进管线。
///
/// 消费方(如 ah-cli 的 rsi 子命令)通过 "rsi" 服务键解析本 trait。
#[async_trait]
pub trait RsiRuntime: Seam {
    /// 从种子任务生成数据集(真实确定性扩展:改写/组合/边界,LLM 生成留待后续)。
    fn generate_dataset(
        &self,
        seed_tasks: Vec<String>,
        count: usize,
    ) -> Result<Vec<RsiCase>, RsiError>;

    /// 执行单个用例:经 SubagentRuntime 真实委派,再以 evolving 评估。
    async fn run_case(
        &self,
        round: u32,
        case: &RsiCase,
        task_prompt: &str,
    ) -> Result<RsiRunOutcome, RsiError>;

    /// 执行一轮:依次运行全部用例并聚合报告。
    async fn evaluate_round(
        &self,
        round: u32,
        cases: &[RsiCase],
        task_prompt: &str,
    ) -> Result<RsiReport, RsiError>;

    /// 保存 checkpoint(真实 JSONL 落盘)。
    fn save_checkpoint(&self, checkpoint: &RsiCheckpoint) -> Result<(), RsiError>;

    /// 加载最新 checkpoint(无则 None)。
    fn load_checkpoint(&self) -> Result<Option<RsiCheckpoint>, RsiError>;

    /// 依据报告精化任务提示:对最差用例的轨迹做 evolving 评估与优化,返回新提示。
    async fn refine_task(&self, report: &RsiReport, task_prompt: &str) -> Result<String, RsiError>;
}
