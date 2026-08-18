//! 注入清洗(与 agent-core `harness/prompts/sanitize.py` 1:1 对齐)。
//!
//! Python 原始模式:
//! ```python
//! _INJECTION_PATTERN = re.compile(r"[<>\{\}\[\]`\$]|\.{3,}|\\n|\\r")
//! ```
//! raw 字符串中 `\\n` 是两个字符(字面反斜杠 + `n`),故正则语义为:
//! - 单字符类 `< > { } [ ] ` $`:全部移除;
//! - 三个及以上连续点号 `\.{3,}`:整段移除(双点与单点保留);
//! - 字面两字符序列 `\\n`(反斜杠+n)与 `\\r`(反斜杠+r):移除。
//!
//! 手写实现(不依赖 regex crate):纯 ASCII 字符类 + 点号游程 + 反斜杠序列,
//! 自左向右逐字符扫描,与 Python `re.sub` 的最左匹配语义一致。
//! 逐字符处理保证对任意 UTF-8 输入安全;截断按 Unicode 标量计数
//! (Rust `chars().count()`,对应 Python `len()` 的字符数)。

/// Python `sanitize_user_content` 的默认上限。
pub const DEFAULT_MAX_LEN: usize = 2000;

/// 移除注入危险字符:`<>{}[]`$`、三连及以上点号、字面 `\n` / `\r`。
///
/// 与 `_INJECTION_PATTERN.sub("", s)` 等价:各分支首字符互不重叠
/// (`<>{}[]`$` / `.` / `\`),从左到右贪心匹配点号游程即可复现。
fn strip(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if matches!(c, '<' | '>' | '{' | '}' | '[' | ']' | '`' | '$') {
            i += 1;
            continue;
        }
        if c == '.' {
            // `\.{3,}`:整段移除 3+ 连续点号;1~2 个点保留(每次推进一个点)。
            let mut j = i;
            while j < chars.len() && chars[j] == '.' {
                j += 1;
            }
            if j - i >= 3 {
                i = j;
                continue;
            }
            out.push('.');
            i += 1;
            continue;
        }
        if c == '\\' && i + 1 < chars.len() && matches!(chars[i + 1], 'n' | 'r') {
            // 字面 `\n` / `\r` 两字符序列移除;其余反斜杠(如 `\t`)保留。
            i += 2;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// 清洗用户可控路径字符串,保留正常路径分隔符(`/`、`.` 等)。
///
/// 对应 `sanitize_path(path)`。
pub fn sanitize_path(path: &str) -> String {
    strip(path)
}

/// 清洗用户内容并截断到 `max_len` 字符(按 Unicode 标量计数,同 Python `len`)。
///
/// 对应 `sanitize_user_content(content, max_len=2000)`;Python 侧默认值为
/// 2000,调用方传入 [`DEFAULT_MAX_LEN`] 即可对齐默认行为。
pub fn sanitize_user_content(content: &str, max_len: usize) -> String {
    let safe = strip(content);
    if safe.chars().count() > max_len {
        safe.chars().take(max_len).collect()
    } else {
        safe
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (a) 路径清洗移除 `<>{}[]` 及反引号、美元符。
    #[test]
    fn path_removes_injection_special_chars() {
        assert_eq!(sanitize_path("a<b>c{d}e[f]g`h$i"), "abcdefghi");
        assert_eq!(
            sanitize_path("ignore {system} [prompt] $cmd `code`"),
            "ignore system prompt cmd code"
        );
    }

    /// (b) 保留正常路径分隔符 `/` 与 `.`(以及 `~`、`-`、Unicode 字符)。
    #[test]
    fn path_preserves_separators_and_dots() {
        assert_eq!(
            sanitize_path("dir/sub.dir/file.txt"),
            "dir/sub.dir/file.txt"
        );
        assert_eq!(sanitize_path("~/repo/src/main.rs"), "~/repo/src/main.rs");
        assert_eq!(sanitize_path("数据/报告_v2.xlsx"), "数据/报告_v2.xlsx");
    }

    /// (c) 用户内容先清洗再截断(截断作用于清洗后的文本)。
    #[test]
    fn user_content_sanitizes_then_truncates() {
        assert_eq!(sanitize_user_content("a<b>c", DEFAULT_MAX_LEN), "abc");
        // 清洗后 "abcdef"(6 字符),截断到 4。
        assert_eq!(sanitize_user_content("ab<cd>ef", 4), "abcd");
        // 含 `\n` 字面序列:清洗后 "hello world",截断到 5。
        assert_eq!(sanitize_user_content("hello\\nworld", 5), "hello");
    }

    /// (d) 长内容截断到 `max_len` 字符。
    #[test]
    fn long_content_truncated_to_max_len() {
        let long = "a".repeat(2500);
        let out = sanitize_user_content(&long, DEFAULT_MAX_LEN);
        assert_eq!(out.chars().count(), DEFAULT_MAX_LEN);
        assert_eq!(out.len(), DEFAULT_MAX_LEN);

        let out2 = sanitize_user_content(&"x".repeat(100), 10);
        assert_eq!(out2, "x".repeat(10));
    }

    /// (e) 点号序列:≥3 个移除(整段),双点/单点保留。
    #[test]
    fn dot_runs_removed_double_dots_kept() {
        assert_eq!(sanitize_path("a...b"), "ab");
        assert_eq!(sanitize_path("a....b"), "ab");
        assert_eq!(sanitize_path("..."), "");
        assert_eq!(sanitize_path("a..b"), "a..b");
        assert_eq!(sanitize_path("a.b"), "a.b");
        assert_eq!(sanitize_path("..\\n..."), ".."); // 双点保留 + `\n` 移除 + 三点移除
    }

    /// (f) 字面 `\n` / `\r` 两字符序列移除;其他反斜杠序列与孤立反斜杠保留。
    #[test]
    fn literal_backslash_n_and_r_removed() {
        assert_eq!(sanitize_path("a\\nb"), "ab");
        assert_eq!(sanitize_path("a\\rb"), "ab");
        assert_eq!(sanitize_path("a\\tb"), "a\\tb");
        assert_eq!(sanitize_path("a\\\\nb"), "a\\b"); // \\ + n → 保留前一个反斜杠
        assert_eq!(sanitize_path("tail\\"), "tail\\");
    }

    /// (g) Unicode 按字符(标量)计数截断,不产生 UTF-8 边界错误。
    #[test]
    fn unicode_safe_truncation() {
        let out = sanitize_user_content("你好世界你好", 3);
        assert_eq!(out, "你好世");
        assert_eq!(out.chars().count(), 3);
    }
}
