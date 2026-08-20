//! 评测结果分析器(evaluation_result_analyzer):确定性信号提取 + 规则根因归因。
//!
//! 对每一条失败用例提取信号(超时/工具错误/判据不匹配/预算耗尽/未完成),
//! 按信号种类聚合为带证据引用的 issue,落盘 analysis.json。LLM 深度诊断留待后续。

use std::path::Path;
use std::sync::Arc;

use ah_contracts::analyzer::{
    AnalysisArtifact, AnalysisSignal, AnalyzerCase, AnalyzerError, EvaluationAnalyzer, EvidenceRef,
    SignalKind, TeamIssue, fingerprint_error,
};
use ah_contracts::keys::RSI_ANALYZER;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;

/// 从一条失败用例提取确定性信号(可能多条,取最主要一条)。
fn extract_signals(case: &AnalyzerCase) -> Option<AnalysisSignal> {
    let error = case.error.as_deref().unwrap_or("");
    let lower = error.to_lowercase();
    let (kind, detail) = if lower.contains("timed out") || lower.contains("timeout") {
        (
            SignalKind::Timeout,
            format!("case {} timed out: {error}", case.case_id),
        )
    } else if lower.contains("tool error") || lower.contains("spawn failed") {
        (
            SignalKind::ToolError,
            format!("case {} tool error: {error}", case.case_id),
        )
    } else if lower.contains("budget") {
        (
            SignalKind::BudgetHeavy,
            format!("case {} budget issue: {error}", case.case_id),
        )
    } else if let Some(expected) = &case.expected {
        let matched = case
            .answer
            .as_deref()
            .map(|a| a.contains(expected.as_str()))
            .unwrap_or(false);
        if !matched {
            (
                SignalKind::ExpectedMismatch,
                format!(
                    "case {} expected {:?} but got {:?}",
                    case.case_id,
                    expected.chars().take(60).collect::<String>(),
                    case.answer
                        .as_deref()
                        .unwrap_or("")
                        .chars()
                        .take(60)
                        .collect::<String>()
                ),
            )
        } else {
            // passed=false 但 expected 命中:视为未完成(判据外)。
            (
                SignalKind::Unfinished,
                format!("case {} unfinished despite match", case.case_id),
            )
        }
    } else {
        (
            SignalKind::Unfinished,
            format!("case {} unfinished (score {:.2})", case.case_id, case.score),
        )
    };
    Some(AnalysisSignal {
        case_id: case.case_id.clone(),
        kind,
        detail,
    })
}

/// 真实评测结果分析器。
pub struct RuleBasedAnalyzer;

impl RuleBasedAnalyzer {
    /// 错误指纹聚类(对齐 signal_extractor 的 error_clusters):
    /// 返回 (fingerprint, [case_ids]) 列表,按指纹字典序。
    pub fn error_clusters(cases: &[AnalyzerCase]) -> Vec<serde_json::Value> {
        let mut map: std::collections::BTreeMap<String, Vec<String>> =
            std::collections::BTreeMap::new();
        for case in cases {
            if let Some(error) = &case.error
                && !error.is_empty()
            {
                let fp = fingerprint_error(error);
                map.entry(fp).or_default().push(case.case_id.clone());
            }
        }
        map.into_iter()
            .map(|(fp, cases)| serde_json::json!({"fingerprint": fp, "cases": cases}))
            .collect()
    }
}

impl Seam for RuleBasedAnalyzer {}

#[async_trait]
impl EvaluationAnalyzer for RuleBasedAnalyzer {
    async fn analyze(
        &self,
        cases: &[AnalyzerCase],
        out_dir: &Path,
    ) -> Result<AnalysisArtifact, AnalyzerError> {
        // 1) 信号提取(仅失败用例)。
        let mut signals = Vec::new();
        for case in cases {
            if !case.passed
                && let Some(signal) = extract_signals(case)
            {
                signals.push(signal);
            }
        }

        // 2) 根因归因 + 聚合(按信号种类分组,带证据引用)。
        let mut issues: Vec<TeamIssue> = Vec::new();
        for kind in [
            SignalKind::Timeout,
            SignalKind::ToolError,
            SignalKind::ExpectedMismatch,
            SignalKind::BudgetHeavy,
            SignalKind::Unfinished,
        ] {
            let matching: Vec<&AnalysisSignal> =
                signals.iter().filter(|s| s.kind == kind).collect();
            if matching.is_empty() {
                continue;
            }
            let (title, diagnosis) = match kind {
                SignalKind::Timeout => (
                    format!(
                        "subagent budget/timeout exceeded ({} cases)",
                        matching.len()
                    ),
                    "任务超出单次委派预算:拆分为可验证子目标或提高预算".to_string(),
                ),
                SignalKind::ToolError => (
                    format!("tool execution errors ({} cases)", matching.len()),
                    "工具调用失败:需要输入校验、错误处理或回退路径".to_string(),
                ),
                SignalKind::ExpectedMismatch => (
                    format!(
                        "output failed the expected criterion ({} cases)",
                        matching.len()
                    ),
                    "输出未满足判据:检查任务理解与提示精化".to_string(),
                ),
                SignalKind::BudgetHeavy => (
                    format!("budget-heavy executions ({} cases)", matching.len()),
                    "迭代预算消耗过高:提示中补充终止判据".to_string(),
                ),
                SignalKind::Unfinished => (
                    format!("unfinished executions ({} cases)", matching.len()),
                    "任务未完成:拆分目标或补充成功判据".to_string(),
                ),
            };
            issues.push(TeamIssue {
                title,
                case_count: matching.len(),
                diagnosis,
                evidence_refs: matching
                    .iter()
                    .map(|s| EvidenceRef {
                        case_id: s.case_id.clone(),
                        trace: format!("eval/{}", s.case_id),
                        detail: s.detail.clone(),
                    })
                    .collect(),
            });
        }

        // 3) 落盘 artifact(真实 JSON 文件)。
        std::fs::create_dir_all(out_dir)
            .map_err(|e| AnalyzerError(format!("create out dir: {e}")))?;
        let artifact = AnalysisArtifact {
            issues,
            signals,
            written_to: None,
        };
        let path = out_dir.join("analysis.json");
        let text = serde_json::to_string_pretty(&artifact)
            .map_err(|e| AnalyzerError(format!("serialize artifact: {e}")))?;
        std::fs::write(&path, text).map_err(|e| AnalyzerError(format!("write artifact: {e}")))?;
        Ok(AnalysisArtifact {
            issues: artifact.issues,
            signals: artifact.signals,
            written_to: Some(path.to_string_lossy().into_owned()),
        })
    }
}

/// analyzer 插件:提供评测结果分析 seam。
pub struct AnalyzerPlugin;

impl Plugin for AnalyzerPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-rsi-analyzer"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![RSI_ANALYZER]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let analyzer: Arc<dyn EvaluationAnalyzer> = Arc::new(RuleBasedAnalyzer);
        Ok(vec![ctx.register(RSI_ANALYZER, analyzer)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::RSI_ANALYZER;
    use ah_hub::plugin::DynPlugin;
    use std::sync::Arc as StdArc;

    fn build_ctx() -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![StdArc::new(AnalyzerPlugin)];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    fn case(
        id: &str,
        passed: bool,
        error: Option<&str>,
        expected: Option<&str>,
        answer: Option<&str>,
    ) -> AnalyzerCase {
        AnalyzerCase {
            case_id: id.to_string(),
            passed,
            score: if passed { 1.0 } else { 0.1 },
            error: error.map(str::to_string),
            expected: expected.map(str::to_string),
            answer: answer.map(str::to_string),
        }
    }

    #[tokio::test]
    async fn extracts_signals_from_failed_cases() {
        let root = std::env::temp_dir().join(format!("ah-an-signals-{}", std::process::id()));
        let (ctx, effects) = build_ctx();
        let analyzer = ctx
            .service::<dyn EvaluationAnalyzer>(&RSI_ANALYZER)
            .expect("analyzer");

        let cases = vec![
            case("t1", false, Some("timed out after 500ms"), None, None),
            case("t2", false, Some("tool error: boom"), None, None),
            case(
                "t3",
                false,
                None,
                Some("expected answer"),
                Some("wrong answer"),
            ),
        ];
        let artifact = analyzer.analyze(&cases, &root).await.expect("analyze");
        assert_eq!(artifact.signals.len(), 3);
        assert!(
            artifact
                .signals
                .iter()
                .any(|s| s.kind == SignalKind::Timeout)
        );
        assert!(
            artifact
                .signals
                .iter()
                .any(|s| s.kind == SignalKind::ToolError)
        );
        assert!(
            artifact
                .signals
                .iter()
                .any(|s| s.kind == SignalKind::ExpectedMismatch)
        );

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn aggregates_issues_with_evidence_and_writes_artifact() {
        let root = std::env::temp_dir().join(format!("ah-an-agg-{}", std::process::id()));
        let (ctx, effects) = build_ctx();
        let analyzer = ctx
            .service::<dyn EvaluationAnalyzer>(&RSI_ANALYZER)
            .expect("analyzer");

        let cases = vec![
            case("a", false, Some("tool error: e1"), None, None),
            case("b", false, Some("tool error: e2"), None, None),
            case("c", true, None, None, None),
        ];
        let artifact = analyzer.analyze(&cases, &root).await.expect("analyze");
        let tool_issue = artifact
            .issues
            .iter()
            .find(|i| i.title.contains("tool execution errors"))
            .expect("tool issue aggregated");
        assert_eq!(tool_issue.case_count, 2, "grouped by root cause");
        assert_eq!(tool_issue.evidence_refs.len(), 2, "evidence refs per case");
        assert!(
            root.join("analysis.json").exists(),
            "artifact written to disk"
        );

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn all_passing_produces_no_issues() {
        let root = std::env::temp_dir().join(format!("ah-an-pass-{}", std::process::id()));
        let (ctx, effects) = build_ctx();
        let analyzer = ctx
            .service::<dyn EvaluationAnalyzer>(&RSI_ANALYZER)
            .expect("analyzer");
        let cases = vec![
            case("a", true, None, None, None),
            case("b", true, None, None, None),
        ];
        let artifact = analyzer.analyze(&cases, &root).await.expect("analyze");
        assert!(artifact.issues.is_empty());
        assert!(artifact.signals.is_empty());
        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn error_clusters_group_by_fingerprint() {
        let cases = vec![
            AnalyzerCase {
                case_id: "c1".to_string(),
                passed: false,
                score: 0.0,
                error: Some("timeout 2024-01-02T03:04:05Z".to_string()),
                expected: None,
                answer: None,
            },
            AnalyzerCase {
                case_id: "c2".to_string(),
                passed: false,
                score: 0.0,
                error: Some("timeout 2025-06-07T08:09:10Z".to_string()),
                expected: None,
                answer: None,
            },
            AnalyzerCase {
                case_id: "c3".to_string(),
                passed: false,
                score: 0.0,
                error: Some("syntax error".to_string()),
                expected: None,
                answer: None,
            },
        ];
        let f1 = fingerprint_error("timeout 2024-01-02T03:04:05Z");
        let f2 = fingerprint_error("timeout 2025-06-07T08:09:10Z");
        eprintln!("f1=[{f1}] f2=[{f2}]");
        let clusters = RuleBasedAnalyzer::error_clusters(&cases);
        assert_eq!(clusters.len(), 2);
        // BTreeMap 排序:"syntax error" 组在前,"timeout" 组在后。
        assert_eq!(clusters[0]["cases"], serde_json::json!(["c3"]));
        assert_eq!(clusters[1]["cases"], serde_json::json!(["c1", "c2"]));
    }
}
