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

// ---------------------------------------------------------------------------
// signal_extractor 确定性工具(对齐 rsi/evaluation_result_analyzer/signal_extractor.py)
// ---------------------------------------------------------------------------

/// 错误指纹(对齐 `_fingerprint_error`):时间戳/uuid/路径/hex/行号 → 占位,空白归一。
///
/// 替换顺序严格:ts → uuid → path → hex → :<N> → 空白 join。
pub fn fingerprint_error(error: &str) -> String {
    let mut text = error.to_string();
    // 时间戳:YYYY-MM-DD[T ]HH:MM:SS[.fff][Z|±hh:mm]
    text = replace_regex(
        &text,
        r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})?",
        "<ts>",
    );
    // UUID。
    text = replace_regex(
        &text,
        r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}",
        "<uuid>",
    );
    // 路径:Windows 盘符或 / 开头。
    text = replace_regex(&text, "__path_pattern__", "<path>");
    // hex ≥6 位。
    text = replace_regex(&text, r"[0-9a-fA-F]{6,}", "<hex>");
    // 行号 :N。
    text = replace_regex(&text, r":\d+", ":<N>");
    // 空白归一。
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 简易正则替换(自实现,支持 \\d \\w \\s 与字符类的基础子集)。
fn replace_regex(text: &str, pattern: &str, replacement: &str) -> String {
    // 对给定 pattern 做字面匹配 + 特殊类展开;精确正则引擎不在契约层。
    // 这里用轻量扫描:把 pattern 中的 \d \w \s [..] 类解析为字符谓词。
    match pattern {
        r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})?" => {
            replace_timestamp(text, replacement)
        }
        r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}" => {
            replace_uuid(text, replacement)
        }
        "__path_pattern__" => replace_path(text, replacement),
        r"[0-9a-fA-F]{6,}" => replace_hex(text, replacement),
        r":\d+" => replace_line_no(text, replacement),
        _ => text.to_string(),
    }
}

fn is_digit(c: char) -> bool {
    c.is_ascii_digit()
}

fn is_hex(c: char) -> bool {
    c.is_ascii_hexdigit()
}

fn replace_timestamp(text: &str, replacement: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        // 找 YYYY-MM-DD 开头。
        if i + 10 <= chars.len()
            && chars[i..i + 4].iter().all(|&c| is_digit(c))
            && chars[i + 4] == '-'
            && chars[i + 5..i + 7].iter().all(|&c| is_digit(c))
            && chars[i + 7] == '-'
            && chars[i + 8..i + 10].iter().all(|&c| is_digit(c))
        {
            // 匹配到日期后,继续吞掉时间部分。
            let mut j = i + 10;
            if j < chars.len() && (chars[j] == 'T' || chars[j] == ' ') {
                j += 1;
                let mut time_digits = 0;
                while j < chars.len() && (is_digit(chars[j]) || chars[j] == ':') {
                    if is_digit(chars[j]) {
                        time_digits += 1;
                    }
                    j += 1;
                }
                if time_digits >= 6 {
                    // 小数秒。
                    if j < chars.len() && chars[j] == '.' {
                        j += 1;
                        while j < chars.len() && is_digit(chars[j]) {
                            j += 1;
                        }
                    }
                    // 时区 Z 或 ±hh:mm。
                    if j < chars.len() && chars[j] == 'Z' {
                        j += 1;
                    } else if j < chars.len()
                        && (chars[j] == '+' || chars[j] == '-')
                        && j + 5 <= chars.len()
                        && is_digit(chars[j + 1])
                        && is_digit(chars[j + 2])
                        && chars[j + 3] == ':'
                        && is_digit(chars[j + 4])
                        && is_digit(chars[j + 5])
                    {
                        j += 6;
                    }
                    out.push_str(replacement);
                    i = j;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn replace_uuid(text: &str, replacement: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if i + 36 <= chars.len() && is_uuid_at(&chars, i) {
            out.push_str(replacement);
            i += 36;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn is_uuid_at(chars: &[char], i: usize) -> bool {
    let groups = [8usize, 4, 4, 4, 12];
    let mut pos = i;
    for (gi, len) in groups.iter().enumerate() {
        for _ in 0..*len {
            if pos >= chars.len() || !is_hex(chars[pos]) {
                return false;
            }
            pos += 1;
        }
        if gi < groups.len() - 1 {
            if pos >= chars.len() || chars[pos] != '-' {
                return false;
            }
            pos += 1;
        }
    }
    // 前后不能是 hex(保证 \b 边界)。
    if i > 0 && is_hex(chars[i - 1]) {
        return false;
    }
    // 后边界:pos 处(36 长度末尾)不能是 hex。
    !(pos < chars.len() && is_hex(chars[pos]))
}

fn replace_path(text: &str, replacement: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        // Windows 盘符 C:\... 或 / 开头。
        let is_win = i + 2 < chars.len()
            && chars[i].is_ascii_alphabetic()
            && chars[i + 1] == ':'
            && (chars[i + 2] == '/' || chars[i + 2] == '\\');
        let is_unix = chars[i] == '/';
        if is_win || is_unix {
            let mut j = i;
            // 吞路径字符(非空白/逗号/分号/引号/])。
            if is_win {
                j += 3;
            } else {
                j += 1;
            }
            while j < chars.len()
                && !chars[j].is_whitespace()
                && !matches!(chars[j], ',' | ';' | '\'' | '"' | ']' | ':')
            {
                j += 1;
            }
            if j > i {
                out.push_str(replacement);
                i = j;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn replace_hex(text: &str, replacement: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if is_hex(chars[i]) {
            let mut j = i;
            while j < chars.len() && is_hex(chars[j]) {
                j += 1;
            }
            if j - i >= 6 {
                out.push_str(replacement);
                i = j;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn replace_line_no(text: &str, replacement: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == ':' && i + 1 < chars.len() && is_digit(chars[i + 1]) {
            let mut j = i + 1;
            while j < chars.len() && is_digit(chars[j]) {
                j += 1;
            }
            out.push_str(replacement);
            i = j;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// 聚合评测摘要输入(对齐 EvaluationSummaryInput)。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EvaluationSummaryInput {
    pub total_cases: usize,
    pub passed_count: usize,
    pub failed_count: usize,
    pub average_score: f64,
    pub evaluation_method: String,
}

/// 单用例分析输入(对齐 CaseAnalysisInput 的核心字段)。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CaseAnalysisInput {
    pub case_id: String,
    /// "passed" / "failed" 等状态。
    pub status: String,
    pub score: f64,
    pub input: String,
    pub expected: Option<String>,
    pub response: String,
    pub error: String,
    pub evaluation_method: String,
    pub evaluation_passed: bool,
    pub evaluation_reason: String,
    pub evaluation_metadata: serde_json::Value,
    pub trace_path: String,
    pub result_path: String,
}

/// 确定性信号(对齐 DeterministicSignals)。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DeterministicSignals {
    pub method: String,
    pub exec_failures: Vec<String>,
    pub judge_failures: Vec<String>,
    pub error_clusters: Vec<serde_json::Value>,
    pub method_specific: serde_json::Value,
}

/// 通用提取器(对齐 GenericSignalExtractor.extract)。
///
/// - status == "failed" → exec_failure;
/// - 否则 evaluation_passed == false → judge_failure;
/// - error 非空 → 指纹聚类;
/// - 非 exec-failure:expected None → missing_reference;expected != response →
///   expected_mismatch。
pub fn extract_generic_signals(
    summary: &EvaluationSummaryInput,
    case_inputs: &[CaseAnalysisInput],
) -> DeterministicSignals {
    let mut exec_failures: Vec<String> = Vec::new();
    let mut judge_failures: Vec<String> = Vec::new();
    let mut expected_mismatch: Vec<String> = Vec::new();
    let mut missing_reference: Vec<String> = Vec::new();
    let mut error_map: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();

    for case in case_inputs {
        if case.status == "failed" {
            exec_failures.push(case.case_id.clone());
        } else if !case.evaluation_passed {
            judge_failures.push(case.case_id.clone());
        }
        if !case.error.is_empty() {
            let fp = fingerprint_error(&case.error);
            error_map.entry(fp).or_default().push(case.case_id.clone());
        }
        if case.status != "failed" {
            match &case.expected {
                None => missing_reference.push(case.case_id.clone()),
                Some(expected) => {
                    if *expected != case.response {
                        expected_mismatch.push(case.case_id.clone());
                    }
                }
            }
        }
    }
    let error_clusters: Vec<serde_json::Value> = error_map
        .into_iter()
        .map(|(fp, cases)| serde_json::json!({"fingerprint": fp, "cases": cases}))
        .collect();
    DeterministicSignals {
        method: summary.evaluation_method.clone(),
        exec_failures,
        judge_failures,
        error_clusters,
        method_specific: serde_json::json!({
            "expected_mismatch_cases": expected_mismatch,
            "missing_reference_cases": missing_reference,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_normalizes_volatile_tokens() {
        let error = "Error at 2024-01-02 03:04:05Z: file /home/user/x.py:42, id abcdef123456, uuid 12345678-1234-1234-1234-123456789012";
        let fp = fingerprint_error(error);
        assert!(fp.contains("<ts>"));
        assert!(fp.contains("<path>"));
        assert!(fp.contains(":<N>"));
        assert!(fp.contains("<hex>"));
        assert!(fp.contains("<uuid>"));
        // 空白归一。
        assert!(!fp.contains("  "));
    }

    #[test]
    fn fingerprint_stable_for_same_shape() {
        let a = fingerprint_error("err 2024-01-02 03:04:05 file /a/b.py:42");
        let b = fingerprint_error("err 2025-06-07 08:09:10 file /x/y.py:99");
        assert_eq!(a, b);
    }

    #[test]
    fn fingerprint_keeps_static_text() {
        let fp = fingerprint_error("syntax error near 'foo'");
        assert_eq!(fp, "syntax error near 'foo'");
    }

    #[test]
    fn generic_extractor_rules() {
        let summary = EvaluationSummaryInput {
            total_cases: 4,
            passed_count: 1,
            failed_count: 1,
            average_score: 0.5,
            evaluation_method: "exact_match".to_string(),
        };
        let cases = vec![
            CaseAnalysisInput {
                case_id: "c1".to_string(),
                status: "failed".to_string(),
                error: "timeout 2024-01-02T03:04:05Z".to_string(),
                ..Default::default()
            },
            CaseAnalysisInput {
                case_id: "c2".to_string(),
                status: "passed".to_string(),
                expected: Some("a".to_string()),
                response: "b".to_string(),
                ..Default::default()
            },
            CaseAnalysisInput {
                case_id: "c3".to_string(),
                status: "passed".to_string(),
                expected: None,
                response: "x".to_string(),
                ..Default::default()
            },
            CaseAnalysisInput {
                case_id: "c4".to_string(),
                status: "passed".to_string(),
                evaluation_passed: false,
                error: "timeout 2025-01-02T03:04:05Z".to_string(),
                ..Default::default()
            },
        ];
        let signals = extract_generic_signals(&summary, &cases);
        assert_eq!(signals.method, "exact_match");
        assert_eq!(signals.exec_failures, vec!["c1".to_string()]);
        // 非 failed 且 !evaluation_passed → judge_failure(c2/c3/c4)。
        assert_eq!(
            signals.judge_failures,
            vec!["c2".to_string(), "c3".to_string(), "c4".to_string()]
        );
        // c1 和 c4 的 timeout 指纹相同 → 同一聚类。
        assert_eq!(signals.error_clusters.len(), 1);
        assert_eq!(
            signals.method_specific["expected_mismatch_cases"],
            serde_json::json!(["c2"])
        );
        assert_eq!(
            signals.method_specific["missing_reference_cases"],
            serde_json::json!(["c3", "c4"])
        );
    }
}
