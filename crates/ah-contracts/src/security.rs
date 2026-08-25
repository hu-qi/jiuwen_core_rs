//! security seam:可复用的安全检测(guardrail)。

use std::sync::Arc;

use crate::effect::Effect;
use crate::seam::Seam;

/// 风险级别。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Low,
    Medium,
    High,
}

/// 一条 guardrail 决策。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GuardrailDecision {
    pub guardrail: String,
    pub allow: bool,
    pub severity: Severity,
    pub reason: String,
}

/// 组合安全裁决。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SecurityVerdict {
    /// 任一 guardrail 拒绝则不允许。
    pub allow: bool,
    pub decisions: Vec<GuardrailDecision>,
}

/// 安全错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityError(pub String);

impl core::fmt::Display for SecurityError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SecurityError {}

/// 单个 guardrail(规则后端)。
pub trait Guardrail: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    fn check(&self, content: &str) -> GuardrailDecision;
}

/// security Seam(Service Definition):guardrail 集合与组合检测。
pub trait SecurityProvider: Seam {
    /// 注册一个 guardrail(可逆)。
    fn register(&self, guardrail: Arc<dyn Guardrail>) -> Effect;

    /// 全部决策(每个 guardrail 一条)。
    fn check(&self, content: &str) -> Vec<GuardrailDecision>;

    /// 组合裁决:任一拒绝即不允许。
    fn verdict(&self, content: &str) -> SecurityVerdict;
}

// ---------------------------------------------------------------------------
// 路径防护(file_guard,对齐 harness/security/file_guard.py + models.py)
// ---------------------------------------------------------------------------

/// 权限级别(对齐 PermissionLevel)。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum PermissionLevel {
    Deny,
    Ask,
    Allow,
}

impl PermissionLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            PermissionLevel::Deny => "deny",
            PermissionLevel::Ask => "ask",
            PermissionLevel::Allow => "allow",
        }
    }
}

/// 权限判定结果(对齐 PermissionResult)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PermissionResult {
    pub permission: PermissionLevel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_paths: Option<Vec<String>>,
}

impl PermissionResult {
    pub fn is_allowed(&self) -> bool {
        self.permission == PermissionLevel::Allow
    }
    pub fn is_denied(&self) -> bool {
        self.permission == PermissionLevel::Deny
    }
    pub fn needs_approval(&self) -> bool {
        self.permission == PermissionLevel::Ask
    }
}

/// file_guard 模式(对齐 FileGuardMode)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileGuardMode {
    Legacy,
    Native,
}

/// 路径匹配方式(对齐 FileGuardMatch)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileGuardMatch {
    Prefix,
    Glob,
}

/// 路径访问轴(对齐 FileGuardAction)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileGuardAction {
    Read,
    Write,
    Exec,
}

/// 三轴默认(对齐 FileGuardAxisDefaults)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FileGuardAxisDefaults {
    pub read: PermissionLevel,
    pub write: PermissionLevel,
    pub exec: PermissionLevel,
}

/// 单条路径规则(对齐 FileGuardPathRule)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FileGuardPathRule {
    pub path: String,
    pub read: PermissionLevel,
    pub write: PermissionLevel,
    pub exec: PermissionLevel,
    pub r#match: FileGuardMatch,
}

/// 生效配置(对齐 EffectiveFileGuardConfig)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EffectiveFileGuardConfig {
    pub enabled: bool,
    pub mode: FileGuardMode,
    pub defaults: FileGuardAxisDefaults,
    pub paths: Vec<FileGuardPathRule>,
    pub workspace_root: Option<String>,
}

/// 解析权限级别字符串(对齐 `_parse_level`;无法识别回退 default)。
pub fn parse_level(raw: &str, default: PermissionLevel) -> PermissionLevel {
    match raw.trim().to_lowercase().as_str() {
        "allow" => PermissionLevel::Allow,
        "ask" => PermissionLevel::Ask,
        "deny" => PermissionLevel::Deny,
        _ => default,
    }
}

/// 严格性排序:deny < ask < allow;多级取最严(对齐 tiered_policy.strictest)。
pub fn strictest(levels: &[PermissionLevel]) -> PermissionLevel {
    levels
        .iter()
        .copied()
        .min_by_key(|level| match level {
            PermissionLevel::Deny => 0,
            PermissionLevel::Ask => 1,
            PermissionLevel::Allow => 2,
        })
        .unwrap_or(PermissionLevel::Ask)
}

/// 单轴动作映射(对齐 `_axis_from_star`)。
pub fn axis_from_star(action: &str, default: PermissionLevel) -> FileGuardAxisDefaults {
    let level = parse_level(action, default);
    FileGuardAxisDefaults {
        read: level,
        write: level,
        exec: level,
    }
}

/// 蕴含规则 Write⇒Read / Exec⇒Read(对齐 `_apply_implications`)。
pub fn apply_implications(
    read: PermissionLevel,
    write: PermissionLevel,
    exec: PermissionLevel,
) -> (PermissionLevel, PermissionLevel, PermissionLevel) {
    if write == PermissionLevel::Allow || exec == PermissionLevel::Allow {
        if read == PermissionLevel::Deny {
            // 显式 deny 优先于蕴含。
            return (read, write, exec);
        }
        return (PermissionLevel::Allow, write, exec);
    }
    (read, write, exec)
}

/// 编译单条路径条目(对齐 `_compile_path_entry`):
/// 空串/* 跳过;prefix 模式无 `/` 或规范化后无 `/` 跳过;prefix 需绝对化。
pub fn compile_path_entry(
    path_raw: &str,
    read: Option<PermissionLevel>,
    write: Option<PermissionLevel>,
    exec: Option<PermissionLevel>,
    r#match: FileGuardMatch,
    default_level: PermissionLevel,
) -> Option<FileGuardPathRule> {
    let path_s = path_raw.trim();
    if path_s.is_empty() || path_s == "*" {
        return None;
    }
    let r = read.unwrap_or(default_level);
    let w = write.unwrap_or(default_level);
    let e = exec.unwrap_or(default_level);
    let (r, w, e) = apply_implications(r, w, e);
    if r#match == FileGuardMatch::Prefix {
        let norm = path_s.replace('\\', "/").trim_end_matches('/').to_string();
        if !norm.contains('/') {
            return None;
        }
        // prefix 需含 /,否则跳过(避免 "C:" 误匹配整盘)。
    }
    Some(FileGuardPathRule {
        path: path_s.to_string(),
        read: r,
        write: w,
        exec: e,
        r#match,
    })
}

/// 简单 glob 匹配(对齐 `_match_glob`):支持 `**` / `*` / `?`(路径 posix)。
///
/// 手写匹配器(无 regex 依赖):按 `/` 分段,`**` 匹配 0+ 段,`*` 单段内任意、
/// `?` 单字符。
pub fn match_glob(pattern: &str, path_posix: &str) -> bool {
    let p_segs: Vec<&str> = pattern.split('/').collect();
    let s_segs: Vec<&str> = path_posix.split('/').collect();
    glob_match_segments(&p_segs, &s_segs)
}

/// 分段 glob 匹配:递归处理 `**`(0+ 段)与单段 `*`/`?`。
fn glob_match_segments(p: &[&str], s: &[&str]) -> bool {
    if p.is_empty() {
        return s.is_empty();
    }
    if p[0] == "**" {
        // `**` 匹配 0 或多段。
        if p.len() == 1 {
            return true;
        }
        // 尝试 0 段消费。
        if glob_match_segments(&p[1..], s) {
            return true;
        }
        // 尝试逐段消费。
        for i in 0..s.len() {
            if glob_match_segments(&p[1..], &s[i + 1..]) {
                return true;
            }
        }
        return false;
    }
    if s.is_empty() {
        return false;
    }
    if glob_match_segment(p[0], s[0]) {
        glob_match_segments(&p[1..], &s[1..])
    } else {
        false
    }
}

/// 单段匹配(无 `/`;`*` 任意、`?` 单字符、其余字面)。
fn glob_match_segment(pattern: &str, segment: &str) -> bool {
    let p_chars: Vec<char> = pattern.chars().collect();
    let s_chars: Vec<char> = segment.chars().collect();
    let mut pi = 0;
    let mut si = 0;
    while pi < p_chars.len() {
        match p_chars[pi] {
            '*' => {
                while pi < p_chars.len() && p_chars[pi] == '*' {
                    pi += 1;
                }
                if pi == p_chars.len() {
                    return true;
                }
                // 尝试任意长度剩余。
                let mut matched = false;
                let mut k = si;
                while k <= s_chars.len() {
                    if glob_match_segment(
                        &p_chars[pi..].iter().collect::<String>(),
                        &s_chars[k..].iter().collect::<String>(),
                    ) {
                        matched = true;
                        break;
                    }
                    k += 1;
                }
                return matched;
            }
            '?' => {
                if si >= s_chars.len() {
                    return false;
                }
                pi += 1;
                si += 1;
            }
            c => {
                if si >= s_chars.len() || s_chars[si] != c {
                    return false;
                }
                pi += 1;
                si += 1;
            }
        }
    }
    si == s_chars.len()
}

/// 规则在某轴上的级别(对齐 `_level_for_action`)。
pub fn level_for_action(rule: &FileGuardPathRule, action: FileGuardAction) -> PermissionLevel {
    match action {
        FileGuardAction::Write => rule.write,
        FileGuardAction::Exec => rule.exec,
        FileGuardAction::Read => rule.read,
    }
}

/// 是否形如路径(对齐 `_looks_like_path`)。
pub fn looks_like_path(token: &str) -> bool {
    if token.starts_with("\\\\") || token.starts_with("./") || token.starts_with("../") {
        return true;
    }
    let bytes = token.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        return true;
    }
    token.contains('\\') || token.contains('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_level_parse_and_strictest() {
        assert_eq!(
            parse_level("allow", PermissionLevel::Ask),
            PermissionLevel::Allow
        );
        assert_eq!(
            parse_level(" DENY ", PermissionLevel::Ask),
            PermissionLevel::Deny
        );
        assert_eq!(
            parse_level("ask", PermissionLevel::Allow),
            PermissionLevel::Ask
        );
        assert_eq!(
            parse_level("bogus", PermissionLevel::Ask),
            PermissionLevel::Ask
        );
        assert_eq!(
            parse_level("", PermissionLevel::Allow),
            PermissionLevel::Allow
        );
        // strictest:deny < ask < allow。
        assert_eq!(
            strictest(&[PermissionLevel::Allow, PermissionLevel::Ask]),
            PermissionLevel::Ask
        );
        assert_eq!(
            strictest(&[PermissionLevel::Allow, PermissionLevel::Deny]),
            PermissionLevel::Deny
        );
        assert_eq!(
            strictest(&[PermissionLevel::Allow, PermissionLevel::Allow]),
            PermissionLevel::Allow
        );
        assert_eq!(strictest(&[]), PermissionLevel::Ask);
        assert_eq!(PermissionLevel::Deny.as_str(), "deny");
    }

    #[test]
    fn implications_write_or_exec_imply_read() {
        // write=allow → read=allow。
        let (r, w, _e) = apply_implications(
            PermissionLevel::Ask,
            PermissionLevel::Allow,
            PermissionLevel::Ask,
        );
        assert_eq!(r, PermissionLevel::Allow);
        assert_eq!(w, PermissionLevel::Allow);
        // exec=allow → read=allow。
        let (r, _, e) = apply_implications(
            PermissionLevel::Ask,
            PermissionLevel::Ask,
            PermissionLevel::Allow,
        );
        assert_eq!(r, PermissionLevel::Allow);
        assert_eq!(e, PermissionLevel::Allow);
        // 显式 deny read 优先于蕴含。
        let (r, w, _e) = apply_implications(
            PermissionLevel::Deny,
            PermissionLevel::Allow,
            PermissionLevel::Allow,
        );
        assert_eq!(r, PermissionLevel::Deny);
        assert_eq!(w, PermissionLevel::Allow);
        // 无 allow → 原样。
        let (r, w, e) = apply_implications(
            PermissionLevel::Ask,
            PermissionLevel::Ask,
            PermissionLevel::Deny,
        );
        assert_eq!(
            (r, w, e),
            (
                PermissionLevel::Ask,
                PermissionLevel::Ask,
                PermissionLevel::Deny
            )
        );
    }

    #[test]
    fn compile_path_entry_filters_short_prefixes() {
        // 空/* 跳过。
        assert!(
            compile_path_entry(
                "",
                None,
                None,
                None,
                FileGuardMatch::Prefix,
                PermissionLevel::Ask
            )
            .is_none()
        );
        assert!(
            compile_path_entry(
                "*",
                None,
                None,
                None,
                FileGuardMatch::Prefix,
                PermissionLevel::Ask
            )
            .is_none()
        );
        // prefix 无 / 跳过(避免 "C:" 误匹配整盘)。
        assert!(
            compile_path_entry(
                "C:",
                None,
                None,
                None,
                FileGuardMatch::Prefix,
                PermissionLevel::Ask
            )
            .is_none()
        );
        // 含 / 的 prefix 编译成功,蕴含生效。
        let rule = compile_path_entry(
            "/data",
            Some(PermissionLevel::Ask),
            Some(PermissionLevel::Allow),
            Some(PermissionLevel::Ask),
            FileGuardMatch::Prefix,
            PermissionLevel::Ask,
        )
        .expect("compiled");
        assert_eq!(
            rule.read,
            PermissionLevel::Allow,
            "write=allow implies read=allow"
        );
        assert_eq!(rule.r#match, FileGuardMatch::Prefix);
        // glob 模式不要求 /。
        let glob = compile_path_entry(
            "*.md",
            None,
            None,
            None,
            FileGuardMatch::Glob,
            PermissionLevel::Ask,
        )
        .expect("glob");
        assert_eq!(glob.r#match, FileGuardMatch::Glob);
    }

    #[test]
    fn glob_match_supports_star_globstar_question() {
        // * 单段。
        assert!(match_glob("*.md", "readme.md"));
        assert!(!match_glob("*.md", "a/readme.md"));
        assert!(match_glob("a/*/c", "a/b/c"));
        assert!(!match_glob("a/*/c", "a/b/d/c"));
        // ** 跨段。
        assert!(match_glob("**/*.md", "readme.md"));
        assert!(match_glob("**/*.md", "a/b/readme.md"));
        assert!(match_glob("a/**/c", "a/c"));
        assert!(match_glob("a/**/c", "a/b/c"));
        assert!(match_glob("a/**/c", "a/b/d/c"));
        assert!(!match_glob("a/**/c", "a/b/d"));
        // ? 单字符。
        assert!(match_glob("a?.txt", "ab.txt"));
        assert!(!match_glob("a?.txt", "abc.txt"));
        // 字面。
        assert!(match_glob("/data/public", "/data/public"));
        assert!(!match_glob("/data/public", "/data/private"));
    }

    #[test]
    fn level_for_action_selects_axis() {
        let rule = FileGuardPathRule {
            path: "/x".to_string(),
            read: PermissionLevel::Allow,
            write: PermissionLevel::Deny,
            exec: PermissionLevel::Ask,
            r#match: FileGuardMatch::Prefix,
        };
        assert_eq!(
            level_for_action(&rule, FileGuardAction::Read),
            PermissionLevel::Allow
        );
        assert_eq!(
            level_for_action(&rule, FileGuardAction::Write),
            PermissionLevel::Deny
        );
        assert_eq!(
            level_for_action(&rule, FileGuardAction::Exec),
            PermissionLevel::Ask
        );
    }

    #[test]
    fn looks_like_path_detects_shapes() {
        assert!(looks_like_path("./a"));
        assert!(looks_like_path("../a"));
        assert!(looks_like_path("\\\\server\\share"));
        assert!(looks_like_path("C:\\x"));
        assert!(looks_like_path("C:/x"));
        assert!(looks_like_path("a/b"));
        assert!(!looks_like_path("hello"));
        assert!(!looks_like_path("-flag"));
    }
}
