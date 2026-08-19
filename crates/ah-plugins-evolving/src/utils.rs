//! 自进化工具函数(对齐 agent_evolving/utils.py)。
//!
//! 纯逻辑:技能引用推断(三源命中统计)、frontmatter 顶层解析、
//! LLM 输出 JSON/list 解析、参数校验、dict 序列化。

use std::collections::HashMap;

use serde_json::Value;

/// 优先级感知的技能引用得分(对齐 SkillReferenceScore)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SkillReferenceScore {
    pub skill_tool_hits: usize,
    pub skills_path_hits: usize,
    pub legacy_skill_md_hits: usize,
}

impl SkillReferenceScore {
    /// 优先级排序键(对齐 ranking_key;tool > path > legacy)。
    pub fn ranking_key(&self) -> (usize, usize, usize) {
        (
            self.skill_tool_hits,
            self.skills_path_hits,
            self.legacy_skill_md_hits,
        )
    }
}

/// 从 skill_tool payload 提取 skill_name(对齐 _extract_skill_tool_name)。
pub fn extract_skill_tool_name(payload: &Value) -> String {
    match payload {
        Value::Object(map) => map
            .get("skill_name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        Value::String(s) => {
            if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(s) {
                map.get("skill_name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string()
            } else {
                String::new()
            }
        }
        _ => String::new(),
    }
}

/// 提取内联 skill_tool(skill_name=...) 提及(对齐 _find_skill_tool_mentions)。
pub fn find_skill_tool_mentions(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let lower = text.to_lowercase();
    let mut i = 0;
    while i + 10 <= text.len() {
        // look for 'skill_tool(' case-insensitively
        if lower[i..].starts_with("skill_tool(") {
            // find 'skill_name' then '=' then quoted value
            if let Some(rest) = text[i + 10..].find("skill_name") {
                let after = &text[i + 10 + rest..];
                let eq = after.find('=');
                if let Some(eq) = eq {
                    let v = after[eq + 1..].trim_start();
                    let val_start = v.trim_start_matches(['\'', '\"']);
                    let val_end = val_start
                        .chars()
                        .take_while(|c| {
                            c.is_ascii_alphanumeric() || *c == '.' || *c == '_' || *c == '-'
                        })
                        .collect::<String>();
                    if !val_end.is_empty() {
                        out.push(val_end);
                    }
                }
            }
        }
        i += 1;
    }
    out
}

fn scan_skill_path(
    text: &str,
    known: &std::collections::HashSet<&str>,
    hits: &mut HashMap<String, SkillReferenceScore>,
) {
    let lower = text.to_lowercase();
    let mut i = 0;
    while i + 8 <= text.len() {
        if lower[i..].starts_with("/skills/") || lower[i..].starts_with("\\skills\\") {
            let start = i + 8;
            let rest = &text[start..];
            let name: String = rest
                .chars()
                .take_while(|c| *c != '/' && *c != '\\')
                .collect();
            if !name.is_empty() && known.contains(name.as_str()) {
                let _ = &name;
                hits.entry(name.clone()).or_default().skills_path_hits += 1;
            }
            i = start + name.len();
            continue;
        }
        i += 1;
    }
}

fn scan_legacy_skill_md(
    text: &str,
    known: &std::collections::HashSet<&str>,
    hits: &mut HashMap<String, SkillReferenceScore>,
) {
    let lower = text.to_lowercase();
    let mut i = 0;
    while i + 9 <= text.len() {
        if lower[i..].starts_with("/skill.md") || lower[i..].starts_with("\\skill.md") {
            let end = i;
            let before = &text[..end];
            let name: String = before
                .chars()
                .rev()
                .take_while(|c| *c != '/' && *c != '\\')
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            if !name.is_empty() && known.contains(name.as_str()) {
                let _ = &name;
                hits.entry(name.clone()).or_default().legacy_skill_md_hits += 1;
            }
            i = end + 9;
            continue;
        }
        i += 1;
    }
}
pub fn infer_skill_from_texts(
    skill_names: &[String],
    skill_tool_payloads: &[Value],
    texts: &[String],
) -> Option<String> {
    let known: std::collections::HashSet<&str> = skill_names.iter().map(|s| s.as_str()).collect();
    if known.is_empty() {
        return None;
    }
    let mut hits: HashMap<String, SkillReferenceScore> = HashMap::new();
    for payload in skill_tool_payloads {
        let name = extract_skill_tool_name(payload);
        if known.contains(name.as_str()) {
            hits.entry(name).or_default().skill_tool_hits += 1;
        }
    }
    for text in texts {
        for name in find_skill_tool_mentions(text) {
            if known.contains(name.as_str()) {
                hits.entry(name.clone()).or_default().skill_tool_hits += 1;
            }
        }
        // scan for '/skills/name/' path patterns
        scan_skill_path(text, &known, &mut hits);
        // scan for '/name/SKILL.md' legacy patterns
        scan_legacy_skill_md(text, &known, &mut hits);
    }
    hits.into_iter()
        .max_by_key(|(_, s)| s.ranking_key())
        .map(|(name, _)| name)
}

/// 解析 Markdown 顶层 frontmatter 标量字段(对齐 parse_top_level_frontmatter)。
pub fn parse_top_level_frontmatter(content: &str) -> HashMap<String, String> {
    let text = content.trim();
    if !text.starts_with("---") {
        return HashMap::new();
    }
    let Some(end_rel) = text[3..].find("---") else {
        return HashMap::new();
    };
    let end = end_rel + 3;
    let mut fm = HashMap::new();
    for line in text[3..end].trim().lines() {
        let line = line.trim_end();
        if line.is_empty()
            || line.starts_with(' ')
            || line.starts_with('\t')
            || line.starts_with('-')
        {
            continue;
        }
        if let Some(colon) = line.find(':') {
            let key = line[..colon].trim().to_string();
            let value = line[colon + 1..].trim().to_string();
            fm.insert(key, value);
        }
    }
    fm
}

/// 参数数值边界校验(对齐 validate_digital_parameter)。
pub fn validate_digital_parameter(
    param: f64,
    param_name: &str,
    lower: f64,
    upper: f64,
) -> Result<(), String> {
    if param < lower || param > upper {
        Err(format!(
            "{param_name} should be between {lower} and {upper}"
        ))
    } else {
        Ok(())
    }
}

/// dict 转单行字符串(对齐 _convert_dict_to_string;"k:v | ...")。
pub fn convert_dict_to_string(data: &serde_json::Map<String, Value>) -> String {
    data.iter()
        .map(|(k, v)| format!("{k}:{v}"))
        .collect::<Vec<_>>()
        .join(" | ")
}

/// 从围栏或原始 LLM 输出解析 JSON dict(对齐 parse_json_from_llm_response)。
pub fn parse_json_from_llm_response(json_like: &str) -> Option<Value> {
    let text = json_like.trim();
    if text.is_empty() {
        return None;
    }
    let parsed = if let Some(start) = text.find("```json") {
        let after = &text[start + 7..];
        match after.find("```") {
            Some(end) => serde_json::from_str(after[..end].trim()).ok(),
            None => None,
        }
    } else {
        serde_json::from_str(text).ok()
    };
    match parsed {
        Some(Value::Object(_)) => parsed,
        _ => None,
    }
}

/// 从 ```list ... ``` 块解析 JSON 列表(对齐 parse_list_from_llm_response)。
pub fn parse_list_from_llm_response(list_like: &str) -> Option<Vec<Value>> {
    let re = regex::Regex::new(r"(?s)```list(.*?)```").unwrap();
    let matched = re
        .captures(list_like)
        .and_then(|c| c.get(1).map(|m| m.as_str().trim().to_string()))
        .unwrap_or_default();
    match serde_json::from_str::<Value>(&matched) {
        Ok(Value::Array(arr)) => Some(arr),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_skill_tool_name_from_payload() {
        assert_eq!(
            extract_skill_tool_name(&json!({"skill_name": "code"})),
            "code"
        );
        assert_eq!(extract_skill_tool_name(&json!({"x": 1})), "");
        // JSON string payload
        assert_eq!(
            extract_skill_tool_name(&json!("{\"skill_name\": \"debug\"}")),
            "debug"
        );
        assert_eq!(extract_skill_tool_name(&json!(42)), "");
    }

    #[test]
    fn find_skill_tool_mentions_extracts_names() {
        let mentions = find_skill_tool_mentions(
            "use skill_tool(skill_name=code) and skill_tool(skill_name='debug')",
        );
        assert!(mentions.contains(&"code".to_string()));
        assert!(mentions.contains(&"debug".to_string()));
        assert!(find_skill_tool_mentions("").is_empty());
    }

    #[test]
    fn infer_skill_prefers_tool_then_path_then_legacy() {
        let names = vec!["code".to_string(), "debug".to_string()];
        let payloads = vec![json!({"skill_name": "code"})];
        let texts = vec!["used /skills/debug/ path".to_string()];
        // code has tool hit (priority), debug has path hit
        let inferred = infer_skill_from_texts(&names, &payloads, &texts);
        assert_eq!(inferred.as_deref(), Some("code"));
        // empty skill names -> None
        assert!(infer_skill_from_texts(&[], &payloads, &texts).is_none());
    }

    #[test]
    fn parse_top_level_frontmatter_scalars_only() {
        let fm = parse_top_level_frontmatter(
            "---\nname: X\n  nested: skip\n- item: skip\ndescription: D\n---\nbody",
        );
        assert_eq!(fm.get("name").unwrap(), "X");
        assert_eq!(fm.get("description").unwrap(), "D");
        assert!(!fm.contains_key("nested"));
        assert!(!fm.contains_key("item"));
        assert!(parse_top_level_frontmatter("no frontmatter").is_empty());
    }

    #[test]
    fn validate_param_bounds() {
        assert!(validate_digital_parameter(0.5, "p", 0.0, 1.0).is_ok());
        let err = validate_digital_parameter(1.5, "p", 0.0, 1.0).unwrap_err();
        assert!(err.contains("p should be between"));
    }

    #[test]
    fn convert_dict_to_string_formats() {
        let mut m = serde_json::Map::new();
        m.insert("a".to_string(), json!(1));
        m.insert("b".to_string(), json!("x"));
        let s = convert_dict_to_string(&m);
        assert!(
            s.contains("a:1") && s.contains("b:\"x\"") || s.contains("a:1") && s.contains("b:x")
        );
    }

    #[test]
    fn parse_json_from_llm_response_fenced_and_raw() {
        let fenced = "prefix\n```json\n{\"k\": 1}\n```\nsuffix";
        let parsed = parse_json_from_llm_response(fenced).unwrap();
        assert_eq!(parsed["k"], json!(1));
        let raw = parse_json_from_llm_response("{\"a\": 2}").unwrap();
        assert_eq!(raw["a"], json!(2));
        // list is not a dict -> None
        assert!(parse_json_from_llm_response("[1,2]").is_none());
        assert!(parse_json_from_llm_response("garbage").is_none());
    }

    #[test]
    fn parse_list_from_llm_response_block() {
        let s = "```list\n[1, 2, 3]\n```";
        let list = parse_list_from_llm_response(s).unwrap();
        assert_eq!(list.len(), 3);
        assert!(parse_list_from_llm_response("```list\n{\"a\":1}\n```").is_none());
    }
}
