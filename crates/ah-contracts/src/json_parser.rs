//! json-parser seam:LLM 输出 JSON 解析(对齐 openjiuwen/core/foundation/llm/output_parsers/json_output_parser.py)。
//!
//! - `extract_fenced`:提取 ```json ... ``` 代码围栏内容(DOTALL 跨行);
//! - `parse`:优先围栏内容,否则整段文本,serde_json 解析;解析失败显式报错。
//!
//! 契约零实现:解析算法由插件提供(如 ah-plugins-json-parser)。

use serde_json::Value;

use crate::seam::Seam;

/// JSON 解析错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonParseError {
    /// 文本中找不到可解析的 JSON。
    NoJson,
    /// 找到候选但解析失败(附 serde 错误信息)。
    Invalid(String),
}

impl core::fmt::Display for JsonParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoJson => f.write_str("no JSON found in output"),
            Self::Invalid(msg) => write!(f, "invalid JSON: {msg}"),
        }
    }
}

impl std::error::Error for JsonParseError {}

/// LLM 输出 JSON 解析 Seam(Service Definition)。
pub trait JsonOutputParser: Seam {
    /// 提取 ```json ... ``` 围栏内容(无围栏返回 None)。
    fn extract_fenced(&self, text: &str) -> Option<String>;

    /// 解析:优先围栏内容,否则整段文本;解析失败显式报错。
    fn parse(&self, text: &str) -> Result<Value, JsonParseError>;
}
