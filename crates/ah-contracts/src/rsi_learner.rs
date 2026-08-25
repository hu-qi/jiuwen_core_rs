//! optimization experience learner seam(对齐 rsi/optimization_experience_learner)。
//!
//! 确定性部分:索引检索(ExperienceRetriever)、净化、状态机、目录分配。
//! LLM 驱动的 learn/提取(ExperienceExtractor)留待 LLM seam。

use serde_json::Value;

/// 合法学习状态(对齐 `_VALID_STATUSES`)。
pub const VALID_STATUSES: &[&str] = &[
    "provisional",
    "accepted",
    "rejected",
    "expired",
    "deprecated",
];
/// 默认检索状态(对齐 `_DEFAULT_RETRIEVAL_STATUSES`)。
pub const DEFAULT_RETRIEVAL_STATUSES: &[&str] = &["accepted"];
/// 敏感键(对齐 `_SENSITIVE_KEYS`)。
pub const SENSITIVE_KEYS: &[&str] = &[
    "api_key",
    "apikey",
    "authorization",
    "credential",
    "credentials",
    "password",
    "raw_trace",
    "secret",
    "token",
    "answer",
    "expected_answer",
    "gold_answer",
];

/// 置信度分数(对齐 `_confidence_score`)。
pub fn confidence_score(value: &str) -> i32 {
    match value {
        "high" => 3,
        "medium" => 2,
        "low" => 1,
        _ => 0,
    }
}

/// 截断(对齐 `_truncate`):limit<=0 → 空;超长保留前 limit-15 字符 + "...[truncated]"。
pub fn truncate(value: &str, limit: usize) -> String {
    if limit == 0 {
        return String::new();
    }
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= limit {
        return value.to_string();
    }
    let keep = limit.saturating_sub(15);
    let head: String = chars.into_iter().take(keep).collect();
    format!("{head}...[truncated]")
}

/// 有界字符串列表(对齐 `_bounded_list`):逐条截断,预算耗尽停止。
pub fn bounded_list(value: &Value, limit: usize) -> Vec<String> {
    let mut remaining = limit;
    let mut result: Vec<String> = Vec::new();
    for item in string_list(value) {
        if remaining == 0 {
            break;
        }
        let text = truncate(&item, remaining);
        let chars = text.chars().count();
        result.push(text);
        remaining = remaining.saturating_sub(chars);
    }
    result
}

/// 字符串列表解析(对齐 `_string_list`)。
pub fn string_list(value: &Value) -> Vec<String> {
    match value {
        Value::Null => vec![],
        Value::String(s) => {
            if s.is_empty() {
                vec![]
            } else {
                vec![s.clone()]
            }
        }
        Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                let text = match item {
                    Value::String(s) => s.trim().to_string(),
                    other => other.to_string(),
                };
                if text.is_empty() { None } else { Some(text) }
            })
            .collect(),
        _ => vec![],
    }
}

/// 状态值规整(对齐 `_status_value`):非法 → default。
pub fn status_value(value: &Value, default: &str) -> String {
    let status = match value {
        Value::Null => default.to_string(),
        Value::String(s) => s.trim().to_string(),
        other => other.to_string(),
    };
    if VALID_STATUSES.contains(&status.as_str()) {
        status
    } else {
        default.to_string()
    }
}

/// 安全名(对齐 `_safe_name`):非字母数字/-_ → _;空 → "default"。
pub fn safe_name(value: &str) -> String {
    let cleaned: String = value
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "default".to_string()
    } else {
        cleaned
    }
}

/// 递归净化值(对齐 `_sanitize_value`):字符串含 sk-/Bearer → "[redacted]"。
pub fn sanitize_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, item) in map {
                let key_lower = key.to_lowercase();
                if SENSITIVE_KEYS.contains(&key_lower.as_str()) {
                    continue;
                }
                out.insert(key.clone(), sanitize_value(item));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(sanitize_value).collect()),
        Value::String(s) => {
            if s.contains("sk-") || s.contains("Bearer ") {
                Value::String("[redacted]".to_string())
            } else {
                Value::String(s.clone())
            }
        }
        other => other.clone(),
    }
}

/// 取首个 mapping(对齐 `_first_mapping`)。
pub fn first_mapping(value: &Value) -> serde_json::Map<String, Value> {
    match value {
        Value::Object(map) => map.clone(),
        Value::Array(items) => {
            for item in items {
                if let Value::Object(map) = item {
                    return map.clone();
                }
            }
            serde_json::Map::new()
        }
        _ => serde_json::Map::new(),
    }
}

/// 首个非空文本(对齐 `_first_text`)。
pub fn first_text(values: &[Option<&Value>]) -> String {
    for value in values.iter().flatten() {
        let text = match value {
            Value::String(s) => s.trim().to_string(),
            other => other.to_string(),
        };
        if !text.is_empty() {
            return text;
        }
    }
    String::new()
}

/// 合并字典(对齐 `_merge_dicts`):已有非空键不覆盖。
pub fn merge_dicts(items: &[serde_json::Map<String, Value>]) -> serde_json::Map<String, Value> {
    let mut merged = serde_json::Map::new();
    for item in items {
        for (key, value) in item {
            let exists_non_empty = merged
                .get(key)
                .map(|v| {
                    !matches!(v, Value::Null) && v.as_str().map(|s| !s.is_empty()).unwrap_or(false)
                })
                .unwrap_or(false);
            if !exists_non_empty {
                merged.insert(key.clone(), value.clone());
            }
        }
    }
    merged
}

/// 允许的检索状态集(对齐 `_allowed_statuses`)。
pub fn allowed_statuses(learning_statuses: &Value, allow_provisional: bool) -> Vec<String> {
    let configured: Vec<String> = string_list(learning_statuses)
        .into_iter()
        .filter(|s| VALID_STATUSES.contains(&s.as_str()))
        .collect();
    if !configured.is_empty() {
        return configured;
    }
    let mut allowed: Vec<String> = DEFAULT_RETRIEVAL_STATUSES
        .iter()
        .map(|s| s.to_string())
        .collect();
    if allow_provisional {
        allowed.push("provisional".to_string());
    }
    allowed.sort();
    allowed
}

/// 条目匹配查询(对齐 `_entry_matches_query`)。
#[allow(clippy::too_many_arguments)] // 镜像 Python 查询参数字段。
pub fn entry_matches_query(
    entry: &serde_json::Map<String, Value>,
    optimization_type: &str,
    stage: &str,
    target_members: &[String],
    candidate_modules: &[String],
    failure_signature: &str,
    mechanism_type: &str,
    allowed_statuses: &[String],
) -> bool {
    let learning_status = match entry.get("learning_status") {
        Some(Value::String(s)) => s.clone(),
        other => other.map(Value::to_string).unwrap_or_default(),
    };
    if !allowed_statuses.contains(&learning_status) {
        return false;
    }
    if entry.get("optimization_type").and_then(Value::as_str) != Some(optimization_type) {
        return false;
    }
    if !stage.is_empty() && entry.get("stage").and_then(Value::as_str) != Some(stage) {
        return false;
    }
    if !target_members.is_empty() {
        let role = entry.get("role").and_then(Value::as_str).unwrap_or("");
        if !target_members.iter().any(|m| m == role) {
            return false;
        }
    }
    if !candidate_modules.is_empty() {
        let component = entry
            .get("component_layer")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !candidate_modules.iter().any(|m| m == component) {
            return false;
        }
    }
    if !failure_signature.is_empty() {
        let entry_sig = entry
            .get("failure_signature")
            .and_then(Value::as_str)
            .unwrap_or("");
        if entry_sig != failure_signature {
            return false;
        }
    }
    if !mechanism_type.is_empty() {
        let entry_mech = entry
            .get("mechanism_type")
            .and_then(Value::as_str)
            .unwrap_or("");
        if entry_mech != mechanism_type {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn confidence_and_truncate() {
        assert_eq!(confidence_score("high"), 3);
        assert_eq!(confidence_score("medium"), 2);
        assert_eq!(confidence_score("low"), 1);
        assert_eq!(confidence_score("unknown"), 0);
        assert_eq!(truncate("abc", 5), "abc");
        assert_eq!(truncate("abcdefghij", 5), "...[truncated]");
        assert_eq!(truncate("abcdefghij", 0), "");
        let long = "x".repeat(100);
        let t = truncate(&long, 30);
        assert!(t.ends_with("...[truncated]"));
        assert!(t.chars().count() <= 30);
    }

    #[test]
    fn bounded_list_respects_budget() {
        let items = json!(["aaaa", "bb", "cccc"]);
        let out = bounded_list(&items, 7);
        // 与 Python 一致:预算耗尽后最后一条仍截断追加。
        assert_eq!(out, vec!["aaaa", "bb", "...[truncated]"]);
    }

    #[test]
    fn string_list_and_status_value() {
        assert_eq!(string_list(&Value::Null), Vec::<String>::new());
        assert_eq!(string_list(&json!("x")), vec!["x".to_string()]);
        assert_eq!(
            string_list(&json!(["a", "b"])),
            vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(status_value(&json!("accepted"), "provisional"), "accepted");
        assert_eq!(status_value(&json!("bogus"), "provisional"), "provisional");
        assert_eq!(status_value(&Value::Null, "provisional"), "provisional");
    }

    #[test]
    fn safe_name_sanitizes() {
        assert_eq!(safe_name("my-type_2"), "my-type_2");
        assert_eq!(safe_name("a b/c"), "a_b_c");
        assert_eq!(safe_name("   "), "default", "blank → default");
    }

    #[test]
    fn sanitize_value_redacts() {
        let v = json!({
            "api_key": "sk-abc",
            "name": "hello sk-xyz",
            "nested": {"token": "t", "keep": "v"},
            "list": [{"password": "p"}, "Bearer abc"]
        });
        let out = sanitize_value(&v);
        assert!(out.get("api_key").is_none(), "sensitive key dropped");
        assert_eq!(out["name"], "[redacted]");
        assert!(out["nested"].get("token").is_none());
        assert_eq!(out["nested"]["keep"], "v");
        assert_eq!(out["list"][0], json!({}));
        assert_eq!(out["list"][1], "[redacted]");
    }

    #[test]
    fn allowed_statuses_and_matching() {
        let allowed = allowed_statuses(&Value::Null, false);
        assert_eq!(allowed, vec!["accepted"]);
        let allowed = allowed_statuses(&Value::Null, true);
        assert!(allowed.contains(&"provisional".to_string()));
        let allowed = allowed_statuses(&json!(["accepted", "rejected"]), false);
        assert_eq!(allowed.len(), 2);

        let entry = json!({
            "learning_status": "accepted",
            "optimization_type": "prompt",
            "stage": "evaluate",
            "role": "leader",
            "component_layer": "scheduler",
            "failure_signature": "sig1",
            "mechanism_type": "mechanism1",
        });
        let entry = entry.as_object().unwrap();
        let allowed = vec!["accepted".to_string()];
        assert!(entry_matches_query(
            entry,
            "prompt",
            "evaluate",
            &[],
            &[],
            "",
            "",
            &allowed
        ));
        assert!(!entry_matches_query(
            entry,
            "prompt",
            "train",
            &[],
            &[],
            "",
            "",
            &allowed
        ));
        assert!(!entry_matches_query(
            entry,
            "prompt",
            "evaluate",
            &["teammate".to_string()],
            &[],
            "",
            "",
            &allowed
        ));
        assert!(entry_matches_query(
            entry,
            "prompt",
            "evaluate",
            &[],
            &["scheduler".to_string()],
            "sig1",
            "mechanism1",
            &allowed
        ));
        assert!(!entry_matches_query(
            entry,
            "prompt",
            "evaluate",
            &[],
            &[],
            "sig2",
            "",
            &allowed
        ));
        // 状态不在允许集 → 拒绝。
        assert!(!entry_matches_query(
            entry,
            "prompt",
            "evaluate",
            &[],
            &[],
            "",
            "",
            &["rejected".to_string()]
        ));
    }

    #[test]
    fn first_mapping_first_text_merge() {
        assert_eq!(
            first_mapping(&json!([{"a": 1}, {"b": 2}])),
            serde_json::json!({"a": 1}).as_object().unwrap().clone()
        );
        assert_eq!(first_text(&[Some(&json!("")), Some(&json!("x"))]), "x");
        assert_eq!(first_text(&[None, Some(&json!(5))]), "5");
        let m1 = serde_json::json!({"a": "x", "b": ""})
            .as_object()
            .unwrap()
            .clone();
        let m2 = serde_json::json!({"a": "y", "c": "z"})
            .as_object()
            .unwrap()
            .clone();
        let merged = merge_dicts(&[m1, m2]);
        assert_eq!(merged["a"], "x", "non-empty wins");
        assert_eq!(merged["c"], "z");
    }
}
