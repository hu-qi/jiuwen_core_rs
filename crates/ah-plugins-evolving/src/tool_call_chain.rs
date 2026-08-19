//! 结构化工具调用链构建(对齐 optimizer/skill_call/tool_call_chain.py)。
//!
//! 纯逻辑:对话消息 → [Turn N] 行(assistant 工具调用/工具结果摘要(失败·空·OK)/用户纠正),
//! 事件数上限与字符截断;失败/纠正关键词复用 from_conv 手写扫描。

use crate::from_conv::{correction_found, failure_keywords_found};
use serde_json::Value;

const TOOL_CHAIN_ARGS_MAX_CHARS: usize = 120;
const TOOL_CHAIN_RESULT_MAX_CHARS: usize = 100;

/// 消息文本提取(对齐 _extract_message_text:字符串/list 文本块/其他)。
pub fn extract_message_text(message: &Value) -> String {
    let Some(content) = message.get("content") else {
        return String::new();
    };
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => {
            let mut parts = Vec::new();
            for block in blocks {
                match block {
                    Value::String(s) => parts.push(s.clone()),
                    Value::Object(_) => {
                        if let Some(t) = block.get("text") {
                            parts.push(t.to_string());
                        }
                    }
                    _ => {}
                }
            }
            parts.join("\n")
        }
        other => other.to_string(),
    }
}

/// 工具结果摘要(对齐 _summarize_tool_result:(状态标签, 单行摘要))。
pub fn summarize_tool_result(content: &str, language: &str) -> (String, String) {
    let text = content.trim();
    if text.is_empty() {
        let (empty, status) = if language == "en" {
            ("(empty)", "EMPTY")
        } else {
            ("(空)", "空")
        };
        return (status.to_string(), empty.to_string());
    }
    let status = if failure_keywords_found(text) {
        if language == "en" {
            "FAIL".to_string()
        } else {
            "失败".to_string()
        }
    } else {
        "OK".to_string()
    };
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let one_line = if one_line.chars().count() > TOOL_CHAIN_RESULT_MAX_CHARS {
        let clipped: String = one_line.chars().take(TOOL_CHAIN_RESULT_MAX_CHARS).collect();
        format!("{clipped}...")
    } else {
        one_line
    };
    (status, one_line)
}

/// 构建工具调用链(对齐 build_tool_call_chain)。
pub fn build_tool_call_chain(messages: &[Value], language: &str, max_events: usize) -> String {
    if messages.is_empty() {
        return if language == "en" {
            "(No execution trace)".to_string()
        } else {
            "(无执行轨迹)".to_string()
        };
    }

    let mut lines: Vec<String> = Vec::new();
    let mut turn = 0usize;
    'outer: for message in messages {
        let role = message.get("role").and_then(|v| v.as_str()).unwrap_or("");
        if role == "assistant" {
            if let Some(tool_calls) = message.get("tool_calls").and_then(|v| v.as_array()) {
                for tool_call in tool_calls {
                    if !tool_call.is_object() {
                        continue;
                    }
                    turn += 1;
                    if turn > max_events {
                        break 'outer;
                    }
                    let name = tool_call
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let args_str = match tool_call.get("arguments") {
                        Some(Value::Object(_)) => tool_call["arguments"].to_string(),
                        Some(Value::String(s)) => s.clone(),
                        Some(other) => other.to_string(),
                        None => String::new(),
                    };
                    let args_str = if args_str.chars().count() > TOOL_CHAIN_ARGS_MAX_CHARS {
                        let clipped: String =
                            args_str.chars().take(TOOL_CHAIN_ARGS_MAX_CHARS).collect();
                        format!("{clipped}...")
                    } else {
                        args_str
                    };
                    lines.push(format!("[Turn {turn}] assistant → {name}({args_str})"));
                }
            }
        } else if role == "tool" || role == "function" {
            turn += 1;
            if turn > max_events {
                break;
            }
            let tool_name = message
                .get("name")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    message
                        .get("tool_name")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                })
                .unwrap_or("tool");
            let (status, summary) = summarize_tool_result(&extract_message_text(message), language);
            lines.push(format!("[Turn {turn}] {tool_name} → {status}: {summary}"));
        } else if role == "user" {
            let text = extract_message_text(message);
            let trimmed = text.trim();
            if !trimmed.is_empty() && correction_found(trimmed) {
                turn += 1;
                if turn > max_events {
                    break;
                }
                let preview: String = trimmed.chars().take(150).collect();
                let preview = if trimmed.chars().count() > 150 {
                    format!("{preview}...")
                } else {
                    preview
                };
                let tag = if language == "en" {
                    "user_correction"
                } else {
                    "用户纠正"
                };
                lines.push(format!("[Turn {turn}] user ({tag}): {preview}"));
            }
        }
        if turn >= max_events {
            break;
        }
    }

    if lines.is_empty() {
        return if language == "en" {
            "(No tool calls; see conversation history)".to_string()
        } else {
            "(无工具调用轨迹；参见对话历史)".to_string()
        };
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_messages() {
        assert_eq!(build_tool_call_chain(&[], "cn", 40), "(无执行轨迹)");
        assert_eq!(build_tool_call_chain(&[], "en", 40), "(No execution trace)");
    }

    #[test]
    fn assistant_tool_call_line() {
        let messages = vec![json!({
            "role": "assistant",
            "tool_calls": [{"name": "bash", "arguments": {"command": "ls"}}],
        })];
        let chain = build_tool_call_chain(&messages, "cn", 40);
        assert_eq!(chain, "[Turn 1] assistant → bash({\"command\":\"ls\"})");
    }

    #[test]
    fn tool_result_status_labels() {
        assert_eq!(
            summarize_tool_result("", "cn"),
            ("空".to_string(), "(空)".to_string())
        );
        assert_eq!(
            summarize_tool_result("", "en"),
            ("EMPTY".to_string(), "(empty)".to_string())
        );
        assert_eq!(summarize_tool_result("Traceback error", "cn").0, "失败");
        assert_eq!(summarize_tool_result("Traceback error", "en").0, "FAIL");
        assert_eq!(summarize_tool_result("all good", "cn").0, "OK");
    }

    #[test]
    fn tool_result_truncation() {
        let long = "x".repeat(200);
        let (_, summary) = summarize_tool_result(&long, "cn");
        assert!(summary.ends_with("..."));
        assert!(summary.chars().count() <= 103);
    }

    #[test]
    fn user_correction_line() {
        let messages = vec![json!({"role": "user", "content": "不对，应该用 yyy"})];
        let chain = build_tool_call_chain(&messages, "cn", 40);
        assert!(chain.contains("[Turn 1] user (用户纠正): 不对，应该用 yyy"));
        let en = build_tool_call_chain(&messages, "en", 40);
        assert!(en.contains("[Turn 1] user (user_correction): 不对，应该用 yyy"));
    }

    #[test]
    fn max_events_cap_and_no_tool_fallback() {
        let mut messages = Vec::new();
        for _ in 0..5 {
            messages.push(
                json!({"role": "assistant", "tool_calls": [{"name": "t", "arguments": "a"}]}),
            );
            messages.push(json!({"role": "tool", "name": "t", "content": "ok"}));
        }
        let chain = build_tool_call_chain(&messages, "cn", 3);
        assert_eq!(chain.lines().count(), 3);
        // 无工具调用 → 回退文案
        let plain = vec![json!({"role": "user", "content": "你好"})];
        assert_eq!(
            build_tool_call_chain(&plain, "cn", 40),
            "(无工具调用轨迹；参见对话历史)",
        );
    }
}
