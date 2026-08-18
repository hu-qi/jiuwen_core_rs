//! resource tag manager seam(对齐 core/runner/resources_manager/{base,tag_manager}.py)。
//!
//! 纯逻辑契约:资源标签双向索引 + 更新/匹配策略 + 错误类型。

/// Tag 类型(字符串标识)。
pub type Tag = String;

/// 特殊 tag:GLOBAL(无显式标签的默认分组)。
pub const GLOBAL: &str = "__global__";

/// 标签匹配策略(对齐 TagMatchStrategy)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TagMatchStrategy {
    /// 资源必须包含全部指定标签。
    All,
    /// 资源包含任一指定标签即可。
    Any,
}

/// 标签更新策略(对齐 TagUpdateStrategy)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TagUpdateStrategy {
    /// 新标签与现有标签并集(去重)。
    Merge,
    /// 用新标签替换现有标签。
    Replace,
}

/// 资源标签管理错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagError {
    pub code: &'static str,
    pub message: String,
}

impl TagError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl core::fmt::Display for TagError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for TagError {}
