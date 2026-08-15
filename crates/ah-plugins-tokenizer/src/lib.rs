//! # ah-plugins-tokenizer
//!
//! 真实确定性 tokenizer(对齐 context_engine 精确 tokenizer):
//! - CJK 字符每字一个 token;
//! - 连续拉丁/数字串按内置词汇表做 BPE-lite 字符对合并;
//! - 空白为分隔(不产出 token);
//! - 返回带字节偏移的 token,count 供上下文预算精确估计。

use std::sync::Arc;

use ah_contracts::keys::TOKENIZER;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::tokenizer::{Token, Tokenizer, TokenizerError};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 常见子词(英语词根/后缀 + 常见编程 token)。BPE-lite 的"已合并对"。
const VOCAB: &[&str] = &[
    "the",
    "ing",
    "ed",
    "ly",
    "er",
    "es",
    "tion",
    "ment",
    "ness",
    "less",
    "ful",
    "able",
    "ible",
    "ous",
    "ive",
    "al",
    "ic",
    "ize",
    "ise",
    "y",
    "ly",
    "system",
    "agent",
    "tool",
    "prompt",
    "memory",
    "context",
    "model",
    "user",
    "assistant",
    "function",
    "python",
    "workspace",
    "session",
    "message",
    "config",
    "error",
    "output",
    "input",
    "task",
    "team",
    "run",
    "file",
    "list",
    "read",
    "write",
    "search",
    "create",
    "update",
    "delete",
    "if",
    "for",
    "while",
    "return",
    "fn",
    "let",
    "mut",
    "pub",
    "use",
    "//",
    "::",
    "->",
    "<-",
    "=>",
    "==",
    "!=",
    "&&",
    "||",
];

fn is_cjk(c: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&c)
        || ('\u{3400}'..='\u{4dbf}').contains(&c)
        || ('\u{3040}'..='\u{30ff}').contains(&c) // 假名
        || ('\u{ac00}'..='\u{d7af}').contains(&c) // 谚文
}

/// 对一段连续 ASCII 字母数字串做 BPE-lite 切分(贪心最长词汇表匹配)。
fn bpe_lite(word: &str) -> Vec<String> {
    if word.is_empty() {
        return vec![];
    }
    let mut result: Vec<String> = Vec::new();
    let mut rest = word;
    while !rest.is_empty() {
        // 找词汇表中能作为 rest 前缀的最长项。
        let mut best: Option<&str> = None;
        for candidate in VOCAB {
            if let Some(stripped) = rest.strip_prefix(candidate)
                && best.map(|b| candidate.len() > b.len()).unwrap_or(true)
            {
                best = Some(candidate);
                let _ = stripped;
            }
        }
        match best {
            Some(candidate) => {
                result.push(candidate.to_string());
                rest = &rest[candidate.len()..];
            }
            None => {
                // 单字符。
                let c = rest.chars().next().unwrap();
                result.push(c.to_string());
                rest = &rest[c.len_utf8()..];
            }
        }
    }
    result
}

/// 真实确定性 tokenizer。
pub struct BpeLiteTokenizer;

impl Seam for BpeLiteTokenizer {}

impl Tokenizer for BpeLiteTokenizer {
    fn tokenize(&self, text: &str) -> Result<Vec<Token>, TokenizerError> {
        let mut tokens = Vec::new();
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = text[i..].chars().next().unwrap();
            let len = c.len_utf8();
            if c.is_alphanumeric() || matches!(c, '_' | '/' | ':' | '-' | '>' | '<' | '=') {
                // 连续 ASCII/数字 run。
                let start = i;
                let mut end = i + len;
                let mut j = end;
                while j < bytes.len() {
                    let cc = text[j..].chars().next().unwrap();
                    if cc.is_alphanumeric() || matches!(cc, '_' | '/' | ':' | '-' | '>' | '<' | '=')
                    {
                        end = j + cc.len_utf8();
                        j = end;
                    } else {
                        break;
                    }
                }
                let word = &text[start..end];
                let mut cursor = 0;
                for piece in bpe_lite(word) {
                    let pos = word[cursor..].find(piece.as_str()).unwrap_or(0);
                    let s = start + cursor + pos;
                    let e = s + piece.len();
                    tokens.push(Token {
                        text: piece.clone(),
                        start: s,
                        end: e,
                    });
                    cursor = (s - start) + piece.len();
                }
                i = end;
            } else if c.is_whitespace() {
                i += len;
            } else if is_cjk(c) {
                tokens.push(Token {
                    text: c.to_string(),
                    start: i,
                    end: i + len,
                });
                i += len;
            } else {
                // 其他符号(标点等):单字符 token。
                tokens.push(Token {
                    text: c.to_string(),
                    start: i,
                    end: i + len,
                });
                i += len;
            }
        }
        Ok(tokens)
    }

    fn count(&self, text: &str) -> Result<usize, TokenizerError> {
        Ok(self.tokenize(text)?.len())
    }
}

/// tokenizer 插件:提供 tokenizer seam。
pub struct TokenizerPlugin;

impl Plugin for TokenizerPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-tokenizer"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![TOKENIZER]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let tokenizer: Arc<dyn Tokenizer> = Arc::new(BpeLiteTokenizer);
        Ok(vec![ctx.register(TOKENIZER, tokenizer)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::TOKENIZER;
    use ah_contracts::tokenizer::Tokenizer;
    use ah_hub::plugin::DynPlugin;
    use std::sync::Arc as StdArc;

    #[test]
    fn tokenizer_splits_cjk_and_latin() {
        let tokenizer = BpeLiteTokenizer;
        // CJK 每字一 token。
        let tokens = tokenizer.tokenize("你好世界").expect("cjk");
        assert_eq!(
            tokens.len(),
            4,
            "four CJK chars: {:?}",
            tokens.iter().map(|t| t.text.clone()).collect::<Vec<_>>()
        );
        // 字节偏移连续。
        for pair in tokens.windows(2) {
            assert_eq!(pair[0].end, pair[1].start, "contiguous offsets");
        }

        // 拉丁子词合并(如 "working" -> work + ing)。
        let tokens = tokenizer.tokenize("working").expect("latin");
        let text: Vec<String> = tokens.iter().map(|t| t.text.clone()).collect();
        let joined = text.join("");
        assert_eq!(joined, "working", "reconstruction: {text:?}");
        assert!(text.len() < 7, "subword merged: {text:?}");

        // 空串与纯空白。
        assert_eq!(tokenizer.count("").expect("empty"), 0);
        assert_eq!(tokenizer.count("   ").expect("space"), 0);
    }

    #[test]
    fn tokenizer_counts_consistently_with_offsets() {
        let tokenizer = BpeLiteTokenizer;
        let text = "list files in the workspace 搜索";
        let count = tokenizer.count(text).expect("count");
        assert!(count > 0);
        // 重建文本 = 原文。
        let tokens = tokenizer.tokenize(text).expect("tokens");
        let rebuilt: String = tokens.iter().map(|t| t.text.clone()).collect();
        // 空白被跳过,重建会丢失空白,故比较去空白版。
        let stripped: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        assert_eq!(rebuilt, stripped, "token reconstruction (no whitespace)");
        // 每个 token 的字节区间有效。
        for token in &tokens {
            assert!(token.start < token.end);
            assert_eq!(
                &text[token.start..token.end],
                token.text,
                "offset points at token"
            );
        }
    }

    #[test]
    fn plugin_registers_tokenizer() {
        let ctx = Context::new();
        let plugin: DynPlugin = StdArc::new(TokenizerPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let tokenizer = ctx.service::<dyn Tokenizer>(&TOKENIZER).expect("tokenizer");
        assert!(tokenizer.count("hello 世界").expect("count") >= 2);
        drop(effects);
        assert!(!ctx.has_service(&TOKENIZER));
    }
}
