//! tokenizer seam:精确 token 切分(对齐 openjiuwen context_engine 精确 tokenizer)。
//!
//! 提供确定性 tokenization:BPE-lite(字符对合并,带真实词汇表)+ CJK 感知
//! (CJK 字符每字一 token,连续拉丁/数字串按子词合并)。可替换 context 引擎的
//! 启发式 estimate_tokens,供预算组装更精确。

use crate::seam::Seam;

/// 一个 token 及其来源文本。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Token {
    /// token 文本。
    pub text: String,
    /// 在原文中的起始字节偏移。
    pub start: usize,
    /// 在原文中的结束字节偏移(开区间)。
    pub end: usize,
}

/// tokenizer 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenizerError(pub String);

impl core::fmt::Display for TokenizerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TokenizerError {}

/// tokenizer Seam(Service Definition):确定性切分。
pub trait Tokenizer: Seam {
    /// 把文本切分为 token(含字节偏移)。
    fn tokenize(&self, text: &str) -> Result<Vec<Token>, TokenizerError>;

    /// token 数。
    fn count(&self, text: &str) -> Result<usize, TokenizerError>;
}
