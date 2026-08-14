//! analyzer seam:评测结果分析(evaluation_result_analyzer)。
//!
//! 真实确定性信号提取(超时/工具错误/判据不匹配/预算耗尽/未完成)+ 规则根因
//! 归因(带证据引用),聚合为 issue 列表并落盘 artifact。LLM 深度诊断留待后续。

use async_trait::async_trait;
use std::path::Path;

use crate::seam::Seam;

/// 一次待分析的用例结果(由 rsi evaluate_round 或外部评测喂入)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AnalyzerCase {
    pub case_id: String,
    pub passed: bool,
    pub score: f64,
    /// 失败原因(如 "timed out"、"tool error: ...")。
    pub error: Option<String>,
    /// 期望判据(Some 时按包含匹配)。
    pub expected: Option<String>,
    /// 实际输出。
    pub answer: Option<String>,
}

/// 确定性信号种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalKind {
    Timeout,
    ToolError,
    ExpectedMismatch,
    BudgetHeavy,
    Unfinished,
}

/// 一条从失败用例提取的信号。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AnalysisSignal {
    pub case_id: String,
    pub kind: SignalKind,
    pub detail: String,
}

/// 一条证据引用(trace_id = case_id,含步骤摘要)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EvidenceRef {
    pub case_id: String,
    pub trace: String,
    pub detail: String,
}

/// 聚合出的问题(根因归因 + 证据)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TeamIssue {
    pub title: String,
    /// 受影响用例数。
    pub case_count: usize,
    pub diagnosis: String,
    pub evidence_refs: Vec<EvidenceRef>,
}

/// 分析产物。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AnalysisArtifact {
    pub issues: Vec<TeamIssue>,
    pub signals: Vec<AnalysisSignal>,
    /// 落盘路径(Some = 已写文件)。
    pub written_to: Option<String>,
}

/// analyzer 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalyzerError(pub String);

impl core::fmt::Display for AnalyzerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AnalyzerError {}

/// analyzer Seam(Service Definition):评测结果分析。
#[async_trait]
pub trait EvaluationAnalyzer: Seam {
    /// 分析一批用例:提取信号 → 根因归因 → 聚合 issue → 落盘 artifact(JSON)。
    async fn analyze(
        &self,
        cases: &[AnalyzerCase],
        out_dir: &Path,
    ) -> Result<AnalysisArtifact, AnalyzerError>;
}
