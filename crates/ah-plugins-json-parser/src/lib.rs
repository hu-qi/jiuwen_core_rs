//! # ah-plugins-json-parser
//!
//! 真实 LLM 输出 JSON 解析(对齐 openjiuwen/core/foundation/llm/output_parsers/json_output_parser.py):
//! - `FencedJsonParser`:提取 ```json ... ``` 围栏(跨行),优先围栏否则整段文本解析;
//! - `StreamJsonParser`:流式累加解析(围栏完整才产出,产出后推进缓冲区)。

use std::sync::Arc;

use ah_contracts::json_parser::{JsonOutputParser, JsonParseError};
use ah_contracts::keys::JSON_PARSER;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use serde_json::Value;

/// 在文本中查找围栏起点(```json 后必须跟 \n 或 \r\n,对齐 Python 正则)。
fn find_fence_start(text: &str) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 7 <= bytes.len() {
        if &text[i..i + 7] == "```json" {
            let after = i + 7;
            if after < bytes.len()
                && bytes[after] == b'\r'
                && after + 1 < bytes.len()
                && bytes[after + 1] == b'\n'
            {
                return Some((i, after + 2));
            }
            if after < bytes.len() && bytes[after] == b'\n' {
                return Some((i, after + 1));
            }
        }
        i += 1;
    }
    None
}

/// 在文本中查找围栏结束(```)。
fn find_fence_end(text: &str) -> Option<usize> {
    text.find("```")
}

/// 提取围栏内容(无围栏返回 None)。
pub fn extract_fenced(text: &str) -> Option<String> {
    let (start, content_start) = find_fence_start(text)?;
    let content = &text[content_start..];
    let end = find_fence_end(content)?;
    let inner = &content[..end];
    let trimmed = inner.trim();
    if trimmed.is_empty() {
        return None;
    }
    let _ = start;
    Some(trimmed.to_string())
}

/// 解析文本中的 JSON:优先围栏,否则整段。
pub fn parse_json(text: &str) -> Result<Value, JsonParseError> {
    if text.trim().is_empty() {
        return Err(JsonParseError::NoJson);
    }
    let candidate = extract_fenced(text).unwrap_or_else(|| text.trim().to_string());
    serde_json::from_str(&candidate).map_err(|e| JsonParseError::Invalid(e.to_string()))
}

/// 无状态 JSON 解析器(seam 实现)。
pub struct FencedJsonParser;

impl Seam for FencedJsonParser {}

impl JsonOutputParser for FencedJsonParser {
    fn extract_fenced(&self, text: &str) -> Option<String> {
        extract_fenced(text)
    }

    fn parse(&self, text: &str) -> Result<Value, JsonParseError> {
        parse_json(text)
    }
}

/// 流式 JSON 解析器:累加 chunk,围栏完整才产出,产出后推进缓冲区。
#[derive(Debug, Default)]
pub struct StreamJsonParser {
    buffer: String,
}

impl StreamJsonParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前缓冲区(调试/观测)。
    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    /// 追加一个 chunk,返回本次完整解析出的 JSON 对象(可能多个)。
    pub fn push(&mut self, chunk: &str) -> Vec<Value> {
        self.buffer.push_str(chunk);
        let mut out = Vec::new();
        loop {
            let Some((_, content_start)) = find_fence_start(&self.buffer) else {
                break;
            };
            let content = &self.buffer[content_start..];
            let Some(end_rel) = find_fence_end(content) else {
                break;
            };
            let inner = &content[..end_rel];
            let trimmed = inner.trim();
            if trimmed.is_empty() {
                break;
            }
            match serde_json::from_str::<Value>(trimmed) {
                Ok(value) => {
                    out.push(value);
                    let fence_end = content_start + end_rel + 3; // 越过 ```
                    self.buffer = self.buffer[fence_end..].trim_start().to_string();
                }
                Err(_) => {
                    // JSON 未完整 → 继续缓冲(对齐 Python 在 JSONDecodeError 时不断言)。
                    break;
                }
            }
        }
        out
    }
}

/// json-parser 插件:注册 `json-parser` seam。
pub struct JsonParserPlugin;

impl Plugin for JsonParserPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-json-parser"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![JSON_PARSER]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let parser: Arc<dyn JsonOutputParser> = Arc::new(FencedJsonParser);
        Ok(vec![ctx.register(JSON_PARSER, parser)])
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::json_parser::JsonParseError;
    use ah_contracts::keys::JSON_PARSER;
    use ah_hub::plugin::DynPlugin;
    use serde_json::json;

    #[test]
    fn parse_bare_json_object() {
        let parser = FencedJsonParser;
        let value = parser.parse(r#"{"ok": true, "n": 3}"#).expect("parse");
        assert_eq!(value, json!({ "ok": true, "n": 3 }));
    }

    #[test]
    fn parse_fenced_json_with_prose() {
        let parser = FencedJsonParser;
        let text = [
            "Sure! Here is the result:",
            "```json",
            "{\"task\": \"build\", \"steps\": [1, 2]}",
            "```",
            "Hope that helps.",
        ]
        .join("\n");
        let value = parser.parse(&text).expect("parse");
        assert_eq!(value, json!({ "task": "build", "steps": [1, 2] }));
    }

    #[test]
    fn parse_multiline_fenced_json() {
        let parser = FencedJsonParser;
        let text = "prefix\n```json\n{\n  \"a\": 1,\n  \"b\": [true, null]\n}\n```\nsuffix";
        let fenced = parser.extract_fenced(text).expect("fenced");
        assert!(fenced.contains("\"a\": 1"), "跨行提取完整: {fenced}");
        let value = parser.parse(text).expect("parse");
        assert_eq!(value, json!({ "a": 1, "b": [true, null] }));
    }

    #[test]
    fn parse_errors_are_explicit() {
        let parser = FencedJsonParser;
        assert!(matches!(parser.parse(""), Err(JsonParseError::NoJson)));
        assert!(matches!(
            parser.parse("   \n  "),
            Err(JsonParseError::NoJson)
        ));
        assert!(matches!(
            parser.parse("hello world"),
            Err(JsonParseError::Invalid(_))
        ));
        assert!(matches!(
            parser.parse("{\"broken\": }"),
            Err(JsonParseError::Invalid(_))
        ));
        // 围栏内非法 JSON → Invalid(而非 NoJson)。
        assert!(matches!(
            parser.parse("```json\nnot json\n```"),
            Err(JsonParseError::Invalid(_))
        ));
    }

    #[test]
    fn fenced_requires_json_marker_with_newline() {
        let parser = FencedJsonParser;
        // 无 ```json 标记 → 无围栏。
        assert!(parser.extract_fenced("{\"a\":1}").is_none());
        // ```json 后无换行 → 不识别为围栏(Python 正则要求 \n)。
        assert!(parser.extract_fenced("```json{\"a\":1}```").is_none());
        // \r\n 换行也识别。
        let crlf = "```json\r\n{\"a\": 1}\r\n```";
        assert!(parser.extract_fenced(crlf).is_some());
    }

    #[test]
    fn stream_parser_yields_completed_fenced_objects() {
        let mut sp = StreamJsonParser::new();
        // 分片投递围栏 JSON。
        assert!(
            sp.push("Here is:\n```json\n{\"a\": ").is_empty(),
            "未完整不产出"
        );
        assert!(sp.push("1}").is_empty(), "内容未闭合不产出");
        let out = sp.push("\n```\ntail");
        assert_eq!(out.len(), 1, "围栏闭合后产出");
        assert_eq!(out[0], json!({ "a": 1 }));
        // 尾部文本仍在缓冲区(非围栏内容不产出)。
        assert!(sp.buffer().contains("tail"));
    }

    #[test]
    fn stream_parser_handles_multiple_objects_and_prose() {
        let mut sp = StreamJsonParser::new();
        let text = [
            "first:\n```json\n{\"x\": 1}\n```",
            "\nsecond:\n```json\n{\"y\": 2}\n```",
        ]
        .join("");
        // 一次投递全部。
        let out = sp.push(&text);
        assert_eq!(out.len(), 2, "两个围栏对象都产出");
        assert_eq!(out[0], json!({ "x": 1 }));
        assert_eq!(out[1], json!({ "y": 2 }));
    }

    #[test]
    fn stream_parser_ignores_bare_non_fenced_text() {
        let mut sp = StreamJsonParser::new();
        let out = sp.push("{\"bare\": true} no fence here");
        assert!(out.is_empty(), "无围栏不产出(对齐 Python 仅匹配围栏)");
        // 完整非法围栏 JSON → 保持缓冲等待后续(不产出、不丢弃)。
        let mut sp2 = StreamJsonParser::new();
        sp2.push("```json\n{\"broken\": }\n```");
        assert!(sp2.buffer().contains("broken"), "非法 JSON 保留在缓冲区");
    }

    #[test]
    fn plugin_registers_json_parser() {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(JsonParserPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let parser = ctx
            .service::<dyn JsonOutputParser>(&JSON_PARSER)
            .expect("json-parser seam");
        let value = parser.parse("```json\n{\"k\": \"v\"}\n```").expect("parse");
        assert_eq!(value, json!({ "k": "v" }));
        drop(effects);
        assert!(!ctx.has_service(&JSON_PARSER));
    }
}
