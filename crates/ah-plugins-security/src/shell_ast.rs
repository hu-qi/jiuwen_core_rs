//! Shell AST 预处理器(对齐 Python harness/security/shell_ast.py)。
//!
//! 保守回退扫描器:树-sitter bash 后端是 Python 可选运行时依赖,真实路径总是
//! 走 `_parse_with_conservative_fallback`:
//! - 明显简单的命令 → `simple` + 单条 `ShellSubcommand`(shlex 风格 argv);
//! - 复合/重定向/替换语法 → `parse_unavailable`(调用方必须 fail closed);
//! - 无 tree-sitter 时结构与 Python 回退路径逐字段一致。

use std::collections::HashSet;

/// shell 结构标志(对齐 ShellStructureFlags)。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ShellStructureFlags {
    pub has_compound_operators: bool,
    pub has_pipeline: bool,
    pub has_subshell: bool,
    pub has_command_group: bool,
    pub has_command_substitution: bool,
    pub has_process_substitution: bool,
    pub has_parameter_expansion: bool,
    pub has_heredoc: bool,
    pub has_input_redirection: bool,
    pub has_output_redirection: bool,
    pub has_actual_operator_nodes: bool,
    pub operators: Vec<String>,
}

impl ShellStructureFlags {
    /// 是否存在风险结构(对齐 has_risky_structure)。
    pub fn has_risky_structure(&self) -> bool {
        self.has_compound_operators
            || self.has_pipeline
            || self.has_subshell
            || self.has_command_group
            || self.has_command_substitution
            || self.has_process_substitution
            || self.has_parameter_expansion
            || self.has_heredoc
            || self.has_input_redirection
            || self.has_output_redirection
    }
}

/// 单条子命令(对齐 ShellSubcommand)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellSubcommand {
    pub text: String,
    pub argv: Vec<String>,
    pub redirects: Vec<String>,
    pub source_span: Option<(usize, usize)>,
    pub parent_operators: Vec<String>,
}

/// 解析结果(对齐 ShellAstParseResult;backend 恒为 "fallback")。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellAstParseResult {
    pub kind: &'static str,
    pub subcommands: Vec<ShellSubcommand>,
    pub flags: ShellStructureFlags,
    pub reason: Option<String>,
    pub backend: &'static str,
}

const COMMAND_SUBSTITUTION: &[&str] = &["`", "$("];
const PROCESS_SUBSTITUTION: &[&str] = &["<(", ">("];
const HEREDOC: &[&str] = &["<<", "<<<"];
const PARAM_EXPANSION: &[&str] = &["${"];

/// 解析 shell 命令供权限检查(对齐 parse_shell_for_permission 的回退路径)。
pub fn parse_shell_for_permission(command: &str) -> ShellAstParseResult {
    let text = command.trim();
    if text.is_empty() {
        return ShellAstParseResult {
            kind: "simple",
            subcommands: vec![],
            flags: ShellStructureFlags::default(),
            reason: None,
            backend: "fallback",
        };
    }

    let flags = scan_shell_structure(text);
    if flags.has_risky_structure() {
        return ShellAstParseResult {
            kind: "parse_unavailable",
            subcommands: vec![],
            flags,
            reason: Some(
                "tree-sitter backend unavailable and fallback detected shell structure".to_string(),
            ),
            backend: "fallback",
        };
    }

    let argv = match shlex_split_posix(text) {
        Some(argv) => argv,
        None => {
            return ShellAstParseResult {
                kind: "parse_unavailable",
                subcommands: vec![],
                flags,
                reason: Some("fallback lexer failed to tokenize command safely".to_string()),
                backend: "fallback",
            };
        }
    };

    let subcommand = ShellSubcommand {
        text: text.to_string(),
        argv,
        redirects: vec![],
        source_span: Some((0, text.len())),
        parent_operators: flags.operators.clone(),
    };
    ShellAstParseResult {
        kind: "simple",
        subcommands: vec![subcommand],
        flags,
        reason: None,
        backend: "fallback",
    }
}

/// 扫描 shell 结构标志(对齐 `_scan_shell_structure`)。
fn scan_shell_structure(command: &str) -> ShellStructureFlags {
    let has_pipeline = command.contains('|');
    let has_compound = ["&&", "||", ";", "\n", "\r"]
        .iter()
        .any(|token| command.contains(token));
    let has_input_redirection = command.contains('<');
    let has_output_redirection = command.contains('>');
    let has_command_substitution = contains_any(command, COMMAND_SUBSTITUTION);
    let has_process_substitution = contains_any(command, PROCESS_SUBSTITUTION);
    let has_parameter_expansion = contains_any(command, PARAM_EXPANSION);
    let has_heredoc = contains_any(command, HEREDOC);
    let operators = collect_operator_markers(command);
    ShellStructureFlags {
        has_compound_operators: has_compound,
        has_pipeline,
        has_subshell: false,
        has_command_group: false,
        has_command_substitution,
        has_process_substitution,
        has_parameter_expansion,
        has_heredoc,
        has_input_redirection,
        has_output_redirection,
        has_actual_operator_nodes: false,
        operators,
    }
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

/// 收集运算符标记(对齐 `_collect_operator_markers`,保持出现顺序去重)。
fn collect_operator_markers(command: &str) -> Vec<String> {
    let mut markers: Vec<String> = Vec::new();
    for token in [
        "&&", "||", ";", "|", ">>", ">", "<", "$(", "`", "<(", ">(", "<<", "<<<",
    ] {
        if command.contains(token) && !markers.iter().any(|m| m == token) {
            markers.push(token.to_string());
        }
    }
    markers
}

/// POSIX shlex 风格分词(对齐 `shlex.split(text, posix=True)`)。
///
/// - 空白分隔;
/// - 单引号内字面量;
/// - 双引号内处理 `$` `` ` `` `"` `\` 与换行转义;
/// - 引号外反斜杠转义下一字符;
/// - 未闭合引号 → None(对应 Python ValueError)。
pub fn shlex_split_posix(text: &str) -> Option<Vec<String>> {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_token = false;
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            c if c.is_whitespace() => {
                if in_token {
                    tokens.push(std::mem::take(&mut current));
                    in_token = false;
                }
            }
            '\'' => {
                in_token = true;
                // 单引号内字面量,直到闭合。
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => current.push(c),
                        None => return None, // 未闭合
                    }
                }
            }
            '"' => {
                in_token = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(c @ ('$' | '`' | '"' | '\\' | '\n')) => {
                                if c != '\n' {
                                    current.push(c);
                                }
                            }
                            Some(c) => {
                                current.push('\\');
                                current.push(c);
                            }
                            None => return None, // 未闭合
                        },
                        Some(c) => current.push(c),
                        None => return None, // 未闭合
                    }
                }
            }
            '\\' => {
                in_token = true;
                match chars.next() {
                    Some(c) => {
                        if c != '\n' {
                            current.push(c);
                        }
                    }
                    None => current.push('\\'),
                }
            }
            other => {
                in_token = true;
                current.push(other);
            }
        }
    }

    if in_token {
        tokens.push(current);
    }
    Some(tokens)
}

/// 幂等去重集合(供调用方聚合运算符,对齐 Python tuple 语义)。
pub fn dedup_operators(operators: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for op in operators {
        if seen.insert(op.clone()) {
            out.push(op.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_command_is_simple_with_argv() {
        let result = parse_shell_for_permission("ls -la /tmp");
        assert_eq!(result.kind, "simple");
        assert_eq!(result.backend, "fallback");
        assert_eq!(result.subcommands.len(), 1);
        assert_eq!(result.subcommands[0].argv, vec!["ls", "-la", "/tmp"]);
        assert_eq!(result.subcommands[0].source_span, Some((0, 11)));
    }

    #[test]
    fn empty_and_blank_are_simple() {
        assert_eq!(parse_shell_for_permission("").kind, "simple");
        assert_eq!(parse_shell_for_permission("   ").kind, "simple");
    }

    #[test]
    fn risky_structures_are_parse_unavailable() {
        // 管道。
        assert_eq!(
            parse_shell_for_permission("a | b").kind,
            "parse_unavailable"
        );
        // 复合操作符。
        assert_eq!(
            parse_shell_for_permission("a && b").kind,
            "parse_unavailable"
        );
        assert_eq!(parse_shell_for_permission("a; b").kind, "parse_unavailable");
        // 重定向。
        assert_eq!(
            parse_shell_for_permission("a > out").kind,
            "parse_unavailable"
        );
        assert_eq!(
            parse_shell_for_permission("a < in").kind,
            "parse_unavailable"
        );
        // 命令替换。
        assert_eq!(
            parse_shell_for_permission("a `b`").kind,
            "parse_unavailable"
        );
        assert_eq!(
            parse_shell_for_permission("a $(b)").kind,
            "parse_unavailable"
        );
        // 进程替换。
        assert_eq!(
            parse_shell_for_permission("a <(b)").kind,
            "parse_unavailable"
        );
        // 参数展开。
        assert_eq!(
            parse_shell_for_permission("echo ${HOME}").kind,
            "parse_unavailable"
        );
        // heredoc。
        assert_eq!(
            parse_shell_for_permission("a << EOF").kind,
            "parse_unavailable"
        );
    }

    #[test]
    fn flags_and_operators_collected() {
        let result = parse_shell_for_permission("echo hi && ls > out");
        assert_eq!(result.kind, "parse_unavailable");
        assert!(result.flags.has_compound_operators);
        assert!(result.flags.has_output_redirection);
        assert!(result.flags.operators.contains(&"&&".to_string()));
        assert!(result.flags.operators.contains(&">".to_string()));
        assert!(result.flags.has_risky_structure());
    }

    #[test]
    fn shlex_posix_split_handles_quotes_and_escapes() {
        assert_eq!(
            shlex_split_posix("echo \"hello world\"").unwrap(),
            vec!["echo", "hello world"]
        );
        assert_eq!(
            shlex_split_posix("echo 'single quoted'").unwrap(),
            vec!["echo", "single quoted"]
        );
        assert_eq!(
            shlex_split_posix("echo a\\ b").unwrap(),
            vec!["echo", "a b"]
        );
        // 未闭合引号 → None。
        assert!(shlex_split_posix("echo \"unclosed").is_none());
        assert!(shlex_split_posix("echo 'unclosed").is_none());
        // 双引号内转义。
        assert_eq!(
            shlex_split_posix("echo \"a\\\"b\"").unwrap(),
            vec!["echo", "a\"b"]
        );
    }

    #[test]
    fn risky_text_is_never_argv_trusted() {
        // 含风险结构的命令不会产出 argv。
        let result = parse_shell_for_permission("rm -rf / && echo done");
        assert_eq!(result.kind, "parse_unavailable");
        assert!(result.subcommands.is_empty());
    }
}
