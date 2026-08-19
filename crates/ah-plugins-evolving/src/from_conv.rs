//! 对话信号检测(对齐 signal/from_conv.py 的确定性部分)。
//!
//! 纯逻辑:失败关键词/用户纠正模式手写扫描(无 regex lookahead 依赖)+ 技能读取历史 +
//! 脚本产物信号 + 去重;LLM 判断的 detect_user_intent 留待集成(本模块提供 fallback)。

use crate::protocols::USER_INTENT_SIGNAL;
use crate::signal::{EvolutionSignal, make_evolution_signal, make_signal_fingerprint};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};

/// 内容获取类工具(输出为网页/文件/搜索结果;对齐 _DATA_FETCH_TOOLS)。
pub const DATA_FETCH_TOOLS: [&str; 15] = [
    "mcp_fetch_webpage",
    "fetch_webpage",
    "web_fetch",
    "search",
    "web_search",
    "google_search",
    "bing_search",
    "view_file",
    "read_file",
    "cat_file",
    "list_directory",
    "ls",
    "get_url",
    "curl",
    "wget",
];

/// 内联代码/命令执行工具(对齐 _CODE_EXEC_TOOLS)。
pub const CODE_EXEC_TOOLS: [&str; 8] = [
    "code",
    "bash",
    "execute_python_code",
    "run_python",
    "exec_code",
    "execute_code",
    "python_exec",
    "run_code",
];

/// 可执行内容参数键(对齐 _EXEC_CONTENT_KEYS)。
pub const EXEC_CONTENT_KEYS: [&str; 8] = [
    "code",
    "code_block",
    "script",
    "source",
    "python_code",
    "command",
    "cmd",
    "shell_command",
];

fn lowercase(haystack: &str) -> String {
    haystack.to_lowercase()
}

fn contains_any(haystack_lower: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack_lower.contains(n))
}

/// 失败关键词检测(error 需排除后随 "= None";对齐 _FAILURE_KEYWORDS)。
pub fn failure_keywords_found(content: &str) -> bool {
    let lower = lowercase(content);
    // error 特例:每个 error 出现位置检查后随非 "= none"
    let mut rest = lower.as_str();
    let mut error_hit = false;
    while let Some(pos) = rest.find("error") {
        let after = &rest[pos + 5..];
        let trimmed = after.trim_start();
        if !trimmed.starts_with("= none") {
            error_hit = true;
            break;
        }
        rest = &rest[pos + 5..];
    }
    if error_hit {
        return true;
    }
    contains_any(
        &lower,
        &[
            "exception",
            "traceback",
            "failed",
            "failure",
            "timeout",
            "timed out",
            "errno",
            "connectionerror",
            "oserror",
            "valueerror",
            "typeerror",
            "错误",
            "异常",
            "失败",
            "超时",
            "no such file",
            "permission denied",
            "access denied",
            "command not found",
            "not recognized",
            "module not found",
            "econnrefused",
            "econnreset",
            "enoent",
            "enotfound",
            "npm err!",
        ],
    )
}

/// 用户纠正模式检测(对齐 _CORRECTION_PATTERN,手写展开)。
pub fn correction_found(text: &str) -> bool {
    if text.contains("不对")
        || text.contains("不是这")
        || text.contains("不是那")
        || text.contains("错了")
        || text.contains("错啦")
        || text.contains("你搞错了")
        || text.contains("你搞错啦")
        || text.contains("这不对")
        || text.contains("你理解错了")
        || text.contains("你理解错啦")
        || text.contains("纠正一下")
        || text.contains("我的意思是")
    {
        return true;
    }
    // 应该(是|用|改|换)
    if let Some(pos) = text.find("应该")
        && let Some(ch) = text[pos + 2..].chars().next()
        && "是用改换".contains(ch)
    {
        return true;
    }
    // 重新(来|做|执行|尝试)
    if let Some(pos) = text.find("重新") {
        let after: String = text[pos + 2..].chars().take(4).collect();
        if ["来", "做", "执行", "尝试"]
            .iter()
            .any(|p| after.starts_with(p))
        {
            return true;
        }
    }
    let lower = lowercase(text);
    if lower.contains("that's wrong")
        || lower.contains("that's incorrect")
        || lower.contains("that's not right")
        || lower.contains("that is wrong")
        || lower.contains("that is incorrect")
        || lower.contains("that is not right")
    {
        return true;
    }
    if lower.contains("you're wrong") || lower.contains("you are wrong") {
        return true;
    }
    for p in ["should be ", "should use ", "should have "] {
        if lower.contains(p) {
            return true;
        }
    }
    if lower.contains("actually,") || text.contains("actually，") {
        return true;
    }
    for p in ["no, wait", "no, actually", "no，wait", "no，actually"] {
        if lower.contains(p) {
            return true;
        }
    }
    if lower.contains("correct:")
        || lower.contains("correction:")
        || lower.contains("fix:")
        || lower.contains("fixed:")
    {
        return true;
    }
    false
}

/// 从 SKILL.md 路径提取技能名(对齐 _SKILL_MD_PATTERN:name/SKILL.md)。
pub fn skill_md_name(arguments: &str) -> Option<String> {
    let lower = lowercase(arguments);
    let mut search_from = 0;
    while let Some(pos) = lower[search_from..].find("skill.md") {
        let abs = search_from + pos;
        // 跳过 SKILL.md 前的分隔符,再回退收集名称段(对齐 [/\\]+([^/\\]+)[/\\]+SKILL\.md)
        let mut sep = abs;
        while sep > 0
            && (arguments.as_bytes()[sep - 1] == b'/' || arguments.as_bytes()[sep - 1] == b'\\')
        {
            sep -= 1;
        }
        let mut start = sep;
        while start > 0 {
            let b = arguments.as_bytes()[start - 1];
            if b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b >= 0x80 {
                start -= 1;
            } else {
                break;
            }
        }
        if start < sep
            && (start == 0
                || arguments.as_bytes()[start - 1] == b'/'
                || arguments.as_bytes()[start - 1] == b'\\')
        {
            return Some(arguments[start..sep].to_string());
        }
        search_from = abs + 8;
    }
    None
}

/// 工具 schema 模式检测(对齐 _TOOL_SCHEMA_PATTERN:{'content': '---\nname: ...)。
pub fn tool_schema_pattern_found(content: &str) -> bool {
    content.contains("{'content': '---") && content.contains("name:")
}

/// 消息中读字段(对齐 _get_field;Value 统一转字符串)。
fn get_field(msg: &Value, key: &str) -> String {
    match msg.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn get_tool_calls(msg: &Value) -> Vec<Value> {
    msg.get("tool_calls")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
}

/// 从 SKILL.md 路径或 skill_tool 调用推断技能名(对齐 _detect_skill_from_tool_calls)。
pub fn detect_skill_from_tool_calls(
    tool_calls: &[Value],
    existing_skills: &BTreeSet<String>,
) -> Option<String> {
    for tool_call in tool_calls {
        let name = lowercase(&get_field(tool_call, "name"));
        let arguments = get_field(tool_call, "arguments");
        let skill_name: Option<String> = if let Some(n) = skill_md_name(&arguments) {
            if is_skill_md_read_tool(&name) {
                Some(n)
            } else {
                None
            }
        } else if name == "skill_tool" {
            serde_json::from_str::<Value>(&arguments)
                .ok()
                .and_then(|v| v.get("skill_name").cloned())
                .and_then(|v| v.as_str().map(|s| s.to_string()))
        } else {
            None
        };
        if let Some(sn) = skill_name
            && is_existing_skill(&sn, existing_skills)
        {
            return Some(sn);
        }
    }
    None
}

fn is_existing_skill(skill_name: &str, existing_skills: &BTreeSet<String>) -> bool {
    existing_skills.is_empty() || existing_skills.contains(skill_name)
}

/// 名称含 file/read 视为 SKILL.md 读取工具(对齐 _is_skill_md_read_tool)。
pub fn is_skill_md_read_tool(name: &str) -> bool {
    name.is_empty() || name.contains("file") || name.contains("read")
}

/// 消息列表扫描出原始信号(对齐 _detect_from_messages)。
pub fn detect_from_messages(
    messages: &[Value],
    existing_skills: &BTreeSet<String>,
) -> Vec<EvolutionSignal> {
    let mut signals = Vec::new();
    let mut skill_read_history: Vec<(usize, String)> = Vec::new();
    let mut pending_scripts: HashMap<String, String> = HashMap::new();
    let mut tool_call_id_to_name: HashMap<String, String> = HashMap::new();

    for (msg_idx, msg) in messages.iter().enumerate() {
        let role = get_field(msg, "role");
        let content = get_field(msg, "content");
        let tool_calls = get_tool_calls(msg);

        if role == "assistant" && !tool_calls.is_empty() {
            if let Some(detected) = detect_skill_from_tool_calls(&tool_calls, existing_skills) {
                skill_read_history.push((msg_idx, detected));
            }
            for tc in &tool_calls {
                let tc_id = get_field(tc, "id");
                let tc_name = lowercase(&get_field(tc, "name"));
                if !tc_id.is_empty() && !tc_name.is_empty() {
                    tool_call_id_to_name.insert(tc_id.clone(), tc_name.clone());
                }
                if CODE_EXEC_TOOLS.contains(&tc_name.as_str()) {
                    let code = extract_code_from_args(tc);
                    if !code.is_empty() && !tc_id.is_empty() {
                        pending_scripts.insert(tc_id, code);
                    }
                }
            }
        }

        if role == "tool" || role == "function" {
            let name_field = get_field(msg, "name");
            let tool_name = if name_field.is_empty() {
                get_field(msg, "tool_name")
            } else {
                name_field
            };
            let tool_call_id = get_field(msg, "tool_call_id");
            let tool_name = if tool_name.is_empty() && !tool_call_id.is_empty() {
                tool_call_id_to_name
                    .get(&tool_call_id)
                    .cloned()
                    .unwrap_or_default()
            } else {
                tool_name
            };
            let active_skill = resolve_active_skill(msg_idx, &skill_read_history);

            if !tool_call_id.is_empty() && pending_scripts.contains_key(&tool_call_id) {
                let has_failure = !content.is_empty() && failure_keywords_found(&content);
                if !has_failure {
                    let code = pending_scripts
                        .get(&tool_call_id)
                        .cloned()
                        .unwrap_or_default();
                    let excerpt: String = code.chars().take(600).collect();
                    signals.push(make_evolution_signal(
                        "script_artifact",
                        "Scripts",
                        excerpt,
                        if tool_name.is_empty() {
                            None
                        } else {
                            Some(tool_name.as_str())
                        },
                        active_skill.as_deref(),
                        Some("passive_conversation"),
                        None,
                    ));
                }
                pending_scripts.remove(&tool_call_id);
            }

            if DATA_FETCH_TOOLS.contains(&tool_name.to_lowercase().as_str()) {
                continue;
            }

            if failure_keywords_found(&content) {
                if tool_schema_pattern_found(&content) {
                    continue;
                }
                let excerpt = extract_around_match(&content);
                signals.push(make_evolution_signal(
                    "execution_failure",
                    "Troubleshooting",
                    excerpt,
                    if tool_name.is_empty() {
                        None
                    } else {
                        Some(tool_name.as_str())
                    },
                    active_skill.as_deref(),
                    Some("passive_conversation"),
                    None,
                ));
            }
        }
    }
    signals
}

/// 最近读取技能(msg_idx 之前;对齐 _resolve_active_skill)。
pub fn resolve_active_skill(msg_idx: usize, history: &[(usize, String)]) -> Option<String> {
    history
        .iter()
        .rev()
        .find(|(idx, _)| *idx <= msg_idx)
        .map(|(_, name)| name.clone())
}

/// 从消息推断当前技能(对齐 _infer_skill_from_messages)。
pub fn infer_skill_from_messages(
    messages: &[Value],
    existing_skills: &BTreeSet<String>,
) -> Option<String> {
    let mut history: Vec<(usize, String)> = Vec::new();
    for (msg_idx, msg) in messages.iter().enumerate() {
        if get_field(msg, "role") == "assistant" {
            let tool_calls = get_tool_calls(msg);
            if let Some(detected) = detect_skill_from_tool_calls(&tool_calls, existing_skills) {
                history.push((msg_idx, detected));
            }
        }
    }
    resolve_active_skill(messages.len(), &history)
}

/// 摘录匹配位置前后(对齐 _extract_around_match;前后各 300 字符)。
pub fn extract_around_match(content: &str) -> String {
    // 确定性实现:取首个失败关键词位置,前后 300 字符
    let lower = lowercase(content);
    let keywords: [&str; 12] = [
        "error",
        "exception",
        "traceback",
        "failed",
        "timeout",
        "errno",
        "错误",
        "异常",
        "失败",
        "超时",
        "no such file",
        "permission denied",
    ];
    let mut best: Option<(usize, &str)> = None;
    for kw in keywords {
        if let Some(pos) = lower.find(kw)
            && best.map(|(p, _)| pos < p).unwrap_or(true)
        {
            best = Some((pos, kw));
        }
    }
    let (pos, kw) = match best {
        Some(v) => v,
        None => return content.chars().take(600).collect::<String>(),
    };
    let chars: Vec<char> = content.chars().collect();
    let kw_len = kw.chars().count();
    let start = pos.saturating_sub(300);
    let end = (pos + kw_len + 300).min(chars.len());
    chars[start..end].iter().collect()
}

/// 从代码执行工具调用提取代码(>20 字符;对齐 _extract_code_from_args)。
pub fn extract_code_from_args(tool_call: &Value) -> String {
    let raw = get_field(tool_call, "arguments");
    let parsed: Value = serde_json::from_str(&raw).unwrap_or(Value::String(raw));
    let obj = match parsed.as_object() {
        Some(o) => o,
        None => return String::new(),
    };
    for key in EXEC_CONTENT_KEYS {
        if let Some(Value::String(v)) = obj.get(key)
            && v.trim().chars().count() > 20
        {
            return v.clone();
        }
    }
    String::new()
}

/// 指纹去重(对齐 _deduplicate)。
pub fn deduplicate(signals: Vec<EvolutionSignal>) -> Vec<EvolutionSignal> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for signal in signals {
        if seen.insert(make_signal_fingerprint(&signal)) {
            out.push(signal);
        }
    }
    out
}

/// 对话信号检测器(对齐 ConversationSignalDetector;确定性部分)。
#[derive(Debug, Clone, Default)]
pub struct ConversationSignalDetector {
    existing_skills: BTreeSet<String>,
}

impl ConversationSignalDetector {
    pub fn new(existing_skills: BTreeSet<String>) -> Self {
        Self { existing_skills }
    }

    /// 检测并过滤默认信号类型 + 去重(对齐 _detect_message_signals 默认)。
    pub fn detect(&self, messages: &[Value]) -> Vec<EvolutionSignal> {
        self.detect_with_types(messages, &["execution_failure", "script_artifact"])
    }

    pub fn detect_with_types(
        &self,
        messages: &[Value],
        signal_types: &[&str],
    ) -> Vec<EvolutionSignal> {
        let signals = detect_from_messages(messages, &self.existing_skills);
        let filtered: Vec<_> = signals
            .into_iter()
            .filter(|s| signal_types.contains(&s.signal_type.as_str()))
            .collect();
        deduplicate(filtered)
    }

    /// 无 LLM 时的用户反馈 fallback(对齐 _fallback_user_feedback_signals)。
    pub fn fallback_user_feedback_signals(
        &self,
        user_messages: &[String],
        skill_name: &str,
    ) -> Vec<EvolutionSignal> {
        for message in user_messages.iter().rev() {
            if correction_found(message) {
                let excerpt: String = message.chars().take(600).collect();
                return vec![make_evolution_signal(
                    USER_INTENT_SIGNAL,
                    "Instructions",
                    excerpt,
                    None,
                    Some(skill_name),
                    Some("passive_conversation"),
                    None,
                )];
            }
        }
        Vec::new()
    }

    pub fn infer_skill(&self, messages: &[Value]) -> Option<String> {
        infer_skill_from_messages(messages, &self.existing_skills)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn failure_keyword_matching() {
        assert!(failure_keywords_found("Traceback (most recent call last)"));
        assert!(failure_keywords_found("npm err! code 1"));
        assert!(failure_keywords_found("TimeoutError: timed out"));
        assert!(failure_keywords_found("命令执行错误"));
        assert!(failure_keywords_found("错误"));
        // error 后随 = None 不命中
        assert!(!failure_keywords_found("error = None"));
        assert!(!failure_keywords_found("all good"));
    }

    #[test]
    fn correction_pattern_matching() {
        assert!(correction_found("不对，应该用 yyy"));
        assert!(correction_found("你理解错了"));
        assert!(correction_found("that's wrong"));
        assert!(correction_found("you're wrong"));
        assert!(correction_found("should be x"));
        assert!(correction_found("纠正一下"));
        assert!(!correction_found("好的，继续"));
    }

    #[test]
    fn skill_md_name_extraction() {
        assert_eq!(
            skill_md_name("/tmp/skills/my-skill/SKILL.md").as_deref(),
            Some("my-skill"),
        );
        assert_eq!(
            skill_md_name("args={'path': 'a/b/SKILL.md'}").as_deref(),
            Some("b"),
        );
        assert_eq!(skill_md_name("no skill here"), None);
    }

    #[test]
    fn tool_schema_pattern() {
        assert!(tool_schema_pattern_found(
            "{'content': '---\nname: foo\ndescription: bar}"
        ));
        assert!(!tool_schema_pattern_found("normal output"));
    }

    #[test]
    fn detect_script_artifact_and_failure() {
        let existing: BTreeSet<String> = BTreeSet::new();
        let messages = vec![
            json!({
                "role": "assistant",
                "tool_calls": [{"id": "c1", "name": "code", "arguments": "{\"code\": \"print('hello world' * 3)\"}"}],
            }),
            json!({"role": "tool", "tool_call_id": "c1", "content": "ok output"}),
            json!({
                "role": "assistant",
                "tool_calls": [{"id": "c2", "name": "bash", "arguments": "{\"command\": \"ls -la /nonexistent\"}"}],
            }),
            json!({"role": "tool", "tool_call_id": "c2", "content": "ls: cannot access: No such file or directory"}),
        ];
        let signals = detect_from_messages(&messages, &existing);
        let types: Vec<&str> = signals.iter().map(|s| s.signal_type.as_str()).collect();
        assert!(types.contains(&"script_artifact"));
        assert!(types.contains(&"execution_failure"));
        assert_eq!(signals.len(), 2);
    }

    #[test]
    fn detector_default_filter_and_dedup() {
        let existing: BTreeSet<String> = BTreeSet::new();
        let detector = ConversationSignalDetector::new(existing);
        let messages = vec![
            json!({
                "role": "assistant",
                "tool_calls": [{"id": "c1", "name": "bash", "arguments": "{\"command\": \"echo hi; echo hi; echo hi; echo hi; echo hi; echo hi; echo hi; echo hi; echo hi; echo hi; echo hi\"}"}],
            }),
            json!({"role": "tool", "tool_call_id": "c1", "content": "hi"}),
        ];
        let signals = detector.detect(&messages);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].signal_type, "script_artifact");
    }

    #[test]
    fn fallback_user_feedback() {
        let detector = ConversationSignalDetector::default();
        let msgs = vec!["继续".to_string(), "不对，应该先检查配置".to_string()];
        let signals = detector.fallback_user_feedback_signals(&msgs, "sk");
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].signal_type, "user_intent");
        assert_eq!(signals[0].section, "Instructions");
        assert_eq!(signals[0].skill_name.as_deref(), Some("sk"));
    }

    #[test]
    fn extract_code_length_filter() {
        let tc = json!({"arguments": "{\"code\": \"short\"}"});
        assert_eq!(extract_code_from_args(&tc), "");
        let long = "x".repeat(30);
        let tc2 = json!({"arguments": format!("{{\"code\": \"{long}\"}}")});
        assert_eq!(extract_code_from_args(&tc2), long);
    }
}
