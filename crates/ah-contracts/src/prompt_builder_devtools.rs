//! prompt_builder_devtools seam:dev_tools 提示构建器的确定性契约。
//!
//! 对齐 `openjiuwen/dev_tools/prompt_builder/builder/*.py` 的确定性部分
//! (LLM 调用经 [`PromptBuilderModel`] trait 注入,本契约不含 LLM 逻辑):
//! - utils:`select_template` / `get_string_prompt`;
//! - badcase:`validate_bad_case_input`(空/上限 10)、`build_bad_case_string`、
//!   `parse_feedback_summary`、`extract_intent_blocks`;
//! - feedback:`validate_feedback_input`、`validate_index_bounds`(insert/select)、
//!   `insert_string`、`extract_intent_from_responses`(```json 围栏解析);
//! - meta_template:`MetaTemplateManager`(META_TEMPLATE_ 前缀注册/取/弹)。
//!
//! 契约零实现:LLM invoke 由插件经 trait 注入;模板文本素材在插件侧。

use std::collections::BTreeMap;

use crate::seam::Seam;

/// 模板语言(对齐 Literal["zh-CN", "en-US"])。
pub const LANG_ZH: &str = "zh-CN";
pub const LANG_EN: &str = "en-US";

/// badcase 用例数上限(对齐 MAX_CASES_LIMIT = 10)。
pub const MAX_CASES_LIMIT: usize = 10;
/// feedback JSON 围栏长度上限(对齐 JSON_STRING_MAX_LENGTH = 10000)。
pub const JSON_STRING_MAX_LENGTH: usize = 10_000;
/// insert 模式插入标记(对齐 INSERT_STR)。
pub const INSERT_STR: &str = "[用户要插入的位置]";
/// meta template 名称前缀(对齐 META_TEMPLATE_NAME_PREFIX)。
pub const META_TEMPLATE_NAME_PREFIX: &str = "META_TEMPLATE_";

/// prompt_builder 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptBuilderError(pub String);

impl core::fmt::Display for PromptBuilderError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PromptBuilderError {}

/// 模板选择(对齐 `select_template`):未知语言回退 zh-CN。
pub fn select_template(language: &str) -> &'static str {
    if language == LANG_EN {
        LANG_EN
    } else {
        LANG_ZH
    }
}

/// 从字符串提取纯文本 prompt(对齐 `get_string_prompt` 的字符串分支)。
pub fn get_string_prompt(prompt: Option<&str>) -> Result<String, PromptBuilderError> {
    match prompt {
        Some(p) => Ok(p.to_string()),
        None => Err(PromptBuilderError(
            "Prompt type <None> is not supported".to_string(),
        )),
    }
}

/// badcase 输入校验(对齐 `_validate_input`)。
pub fn validate_bad_case_input(prompt: &str, cases_len: usize) -> Result<(), PromptBuilderError> {
    if prompt.trim().is_empty() {
        return Err(PromptBuilderError("prompt cannot be empty".to_string()));
    }
    if cases_len == 0 {
        return Err(PromptBuilderError("The cases cannot be empty".to_string()));
    }
    if cases_len > MAX_CASES_LIMIT {
        return Err(PromptBuilderError(format!(
            "The number of cases cannot exceed {MAX_CASES_LIMIT}"
        )));
    }
    Ok(())
}

/// 一条 bad case(对齐 EvaluatedCase 视图)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BadCaseEntry {
    pub question: String,
    pub label: String,
    pub answer: String,
    pub reason: String,
}

/// 逐条 bad case 格式化并拼接(对齐 `_build_bad_case_string`)。
pub fn build_bad_case_string(cases: &[BadCaseEntry]) -> String {
    cases
        .iter()
        .map(|c| {
            format!(
                "question: {}\nlabel: {}\nanswer: {}\nreason: {}",
                c.question, c.label, c.answer, c.reason
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 解析 feedback 摘要(对齐 `_parse_feedback_summary`):取最后一个
/// `<summary>...</summary>` 块;无匹配返回全文。
pub fn parse_feedback_summary(response: &str) -> String {
    let mut matches = Vec::new();
    let mut rest = response;
    while let Some(start) = rest.find("<summary>") {
        let after = &rest[start + "<summary>".len()..];
        if let Some(end) = after.find("</summary>") {
            matches.push(after[..end].to_string());
            rest = &after[end + "</summary>".len()..];
        } else {
            break;
        }
    }
    matches
        .last()
        .cloned()
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| response.to_string())
}

/// 提取 `<intent>...</intent>` 块内容(对齐 badcase 的 intent 提取)。
pub fn extract_intent_blocks(response: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = response;
    while let Some(start) = rest.find("<intent>") {
        let after = &rest[start + "<intent>".len()..];
        if let Some(end) = after.find("</intent>") {
            out.push(after[..end].trim().to_string());
            rest = &after[end + "</intent>".len()..];
        } else {
            break;
        }
    }
    out
}

/// feedback 输入校验(对齐 `_is_valid_prompt`)。
pub fn validate_feedback_input(prompt: &str, feedback: &str) -> Result<(), PromptBuilderError> {
    if prompt.trim().is_empty() || feedback.trim().is_empty() {
        return Err(PromptBuilderError(
            "prompt or feedback cannot be empty".to_string(),
        ));
    }
    Ok(())
}

/// 插入模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertMode {
    Insert,
    Select,
}

/// 索引边界校验(对齐 `_is_index_within_bounds`)。
pub fn validate_index_bounds(
    prompt_len: usize,
    mode: InsertMode,
    start_pos: Option<usize>,
    end_pos: Option<usize>,
) -> Result<(), PromptBuilderError> {
    match mode {
        InsertMode::Insert => {
            let start = start_pos.ok_or_else(|| {
                PromptBuilderError("start_pos must be provided for int type".to_string())
            })?;
            if start <= prompt_len {
                Ok(())
            } else {
                Err(PromptBuilderError(
                    "start_pos must be provided for insert mode. Additionally, it must satisfy the conditions: 0 <= start_pos <= len(prompt).".to_string(),
                ))
            }
        }
        InsertMode::Select => {
            let (start, end) = match (start_pos, end_pos) {
                (Some(s), Some(e)) => (s, e),
                _ => {
                    return Err(PromptBuilderError(
                        "start_pos and end_pos must be provided for int type".to_string(),
                    ));
                }
            };
            if start < end && end <= prompt_len {
                Ok(())
            } else {
                Err(PromptBuilderError(
                    "start_pos and end_pos must be provided for select mode. Additionally, they must satisfy the conditions: 0 <= start_pos < end_pos <= len(prompt).".to_string(),
                ))
            }
        }
    }
}

/// 在 prompt 中插入标记(对齐 `_insert_sting`)。
pub fn insert_string(prompt: &str, insert: usize) -> String {
    let mut out = String::with_capacity(prompt.len() + INSERT_STR.len());
    out.push_str(&prompt[..insert]);
    out.push_str(INSERT_STR);
    out.push_str(&prompt[insert..]);
    out
}

/// 提取 ```json 围栏内容(带长度上限,对齐正则)。
pub fn extract_json_fence(input: &str) -> Option<String> {
    let marker = "```json";
    let start = input.find(marker)?;
    let after = &input[start + marker.len()..];
    let end = after.find("```")?;
    let content = after[..end].trim();
    if content.len() > JSON_STRING_MAX_LENGTH {
        return None;
    }
    Some(content.to_string())
}

/// 从 LLM 响应提取 intent + 优化反馈(对齐 `_extract_intent_from_responses`)。
///
/// 解析 ```json ... ``` 围栏;intent ∈ {true, "true", "True"} 为真;
/// 无围栏或解析失败返回 Err(显式错误)。
pub fn extract_intent_from_responses(input: &str) -> Result<(bool, String), PromptBuilderError> {
    let json_str = extract_json_fence(input)
        .ok_or_else(|| PromptBuilderError("no valid JSON string found".to_string()))?;
    let parsed: serde_json::Value = serde_json::from_str(&json_str)
        .map_err(|e| PromptBuilderError(format!("an error occurred while parsing JSON: {e}")))?;
    let intent = parsed
        .get("intent")
        .map(|v| {
            v.as_bool().unwrap_or(false) || v.as_str() == Some("true") || v.as_str() == Some("True")
        })
        .unwrap_or(false);
    let optimized = parsed
        .get("optimized_feedback")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    Ok((intent, optimized))
}

/// meta template 管理器(对齐 MetaTemplateBuilder 的注册语义)。
#[derive(Debug, Default, Clone)]
pub struct MetaTemplateManager {
    templates: BTreeMap<String, String>,
}

impl MetaTemplateManager {
    /// 构造空管理器。
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册模板(名称自动加 `META_TEMPLATE_` 前缀)。
    pub fn register(&mut self, name: &str, content: String) {
        let key = format!("{META_TEMPLATE_NAME_PREFIX}{name}");
        self.templates.insert(key, content);
    }

    /// 按模板名取(自动加前缀)。
    pub fn get(&self, name: &str) -> Option<&String> {
        self.templates
            .get(&format!("{META_TEMPLATE_NAME_PREFIX}{name}"))
    }

    /// 弹出模板(自动加前缀)。
    pub fn pop(&mut self, name: &str) -> Option<String> {
        self.templates
            .remove(&format!("{META_TEMPLATE_NAME_PREFIX}{name}"))
    }

    /// 模板数。
    pub fn len(&self) -> usize {
        self.templates.len()
    }

    /// 是否空。
    pub fn is_empty(&self) -> bool {
        self.templates.is_empty()
    }
}

/// 消息视图(对齐 BaseMessage 的最小面)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ChatMessageView {
    pub role: String,
    pub content: String,
}

/// LLM 调用面(对齐 BasePromptBuilder 的 model invoke;由插件注入)。
pub trait PromptBuilderModel: Send + Sync {
    /// 单轮调用,返回文本。
    fn invoke(&self, messages: &[ChatMessageView]) -> Result<Option<String>, PromptBuilderError>;
}

/// prompt_builder Seam(Service Definition):三构建器 + meta 模板管理。
pub trait PromptBuilder: Seam {
    /// badcase 构建(LLM;确定性前置校验在契约层)。
    fn build_bad_case(
        &self,
        prompt: &str,
        cases: &[BadCaseEntry],
        language: &str,
    ) -> Result<Option<String>, PromptBuilderError>;

    /// feedback 构建(LLM;确定性前置校验在契约层)。
    fn build_feedback(
        &self,
        prompt: &str,
        feedback: &str,
        mode: InsertMode,
        start_pos: Option<usize>,
        end_pos: Option<usize>,
        language: &str,
    ) -> Result<Option<String>, PromptBuilderError>;

    /// meta template 构建(LLM;确定性前置校验在契约层)。
    fn build_meta_template(
        &self,
        prompt: &str,
        template_type: &str,
        custom_template_name: Option<&str>,
        language: &str,
    ) -> Result<Option<String>, PromptBuilderError>;

    /// meta 模板管理器句柄。
    fn meta_templates(&self) -> MetaTemplateManager;

    /// 注册一个 meta 模板(对齐 `register_meta_template`,名称自动加前缀)。
    fn register_meta_template(&self, name: &str, content: String);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_selection_falls_back_to_zh() {
        assert_eq!(select_template("zh-CN"), LANG_ZH);
        assert_eq!(select_template("en-US"), LANG_EN);
        assert_eq!(select_template("fr-FR"), LANG_ZH);
    }

    #[test]
    fn bad_case_input_validation() {
        assert!(validate_bad_case_input("p", 1).is_ok());
        assert!(validate_bad_case_input("", 1).is_err());
        assert!(validate_bad_case_input("p", 0).is_err());
        assert!(validate_bad_case_input("p", 11).is_err());
        assert!(validate_bad_case_input("p", 10).is_ok());
    }

    #[test]
    fn bad_case_string_builds_lines() {
        let cases = vec![
            BadCaseEntry {
                question: "q1".into(),
                label: "l1".into(),
                answer: "a1".into(),
                reason: "r1".into(),
            },
            BadCaseEntry {
                question: "q2".into(),
                label: "l2".into(),
                answer: "a2".into(),
                reason: "r2".into(),
            },
        ];
        let s = build_bad_case_string(&cases);
        assert!(s.contains("question: q1"));
        assert!(s.contains("reason: r2"));
        assert!(s.contains('\n'));
    }

    #[test]
    fn feedback_summary_parses_last_block() {
        let response = "intro <summary>first</summary> middle <summary>second</summary> tail";
        assert_eq!(parse_feedback_summary(response), "second");
        assert_eq!(parse_feedback_summary("no blocks"), "no blocks");
    }

    #[test]
    fn intent_blocks_extracted() {
        let response = "<intent> false </intent><summary>x</summary>";
        assert_eq!(extract_intent_blocks(response), vec!["false"]);
        assert_eq!(parse_feedback_summary(response), "x");
    }

    #[test]
    fn index_bounds_validation() {
        assert!(validate_index_bounds(5, InsertMode::Insert, Some(5), None).is_ok());
        assert!(validate_index_bounds(5, InsertMode::Insert, None, None).is_err());
        assert!(validate_index_bounds(5, InsertMode::Insert, Some(6), None).is_err());
        assert!(validate_index_bounds(10, InsertMode::Select, Some(2), Some(5)).is_ok());
        assert!(validate_index_bounds(10, InsertMode::Select, Some(5), Some(2)).is_err());
        assert!(validate_index_bounds(10, InsertMode::Select, None, Some(5)).is_err());
        assert!(validate_index_bounds(10, InsertMode::Select, Some(2), Some(11)).is_err());
    }

    #[test]
    fn insert_string_places_marker() {
        assert_eq!(insert_string("abcdef", 3), "abc[用户要插入的位置]def");
        assert_eq!(insert_string("ab", 0), "[用户要插入的位置]ab");
        assert_eq!(insert_string("ab", 2), "ab[用户要插入的位置]");
    }

    #[test]
    fn intent_extraction_from_fence() {
        let ok = "text ```json\n{\"intent\": true, \"optimized_feedback\": \"  better  \"}\n```";
        let (intent, feedback) = extract_intent_from_responses(ok).expect("parse");
        assert!(intent);
        assert_eq!(feedback, "better");

        let string_true = "```json\n{\"intent\": \"true\"}\n```";
        let (intent, _) = extract_intent_from_responses(string_true).expect("parse");
        assert!(intent);

        assert!(extract_intent_from_responses("no fence").is_err());
        assert!(extract_intent_from_responses("```json\n{bad}\n```").is_err());
    }

    #[test]
    fn meta_template_manager_prefixes_names() {
        let mut mgr = MetaTemplateManager::new();
        mgr.register("plan", "plan content".to_string());
        assert_eq!(mgr.get("plan").map(|s| s.as_str()), Some("plan content"));
        assert!(mgr.get("META_TEMPLATE_plan").is_none());
        assert_eq!(mgr.len(), 1);
        assert_eq!(mgr.pop("plan"), Some("plan content".to_string()));
        assert!(mgr.is_empty());
    }

    #[test]
    fn json_fence_extraction() {
        assert_eq!(
            extract_json_fence("```json\n{\"a\": 1}\n```").as_deref(),
            Some("{\"a\": 1}")
        );
        assert!(extract_json_fence("no fence").is_none());
    }
}
