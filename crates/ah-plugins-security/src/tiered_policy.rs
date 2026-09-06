//! 分层工具权限策略。
//!
//! Pipeline A：按参数规则、审批覆盖、整工具策略和默认策略分层判定。
//! 路径规则交给 `file_guard` Pipeline B；未知能力默认 `ask`，不静默放行。

use ah_contracts::security::{PermissionLevel, strictest};
use serde_json::Value;
const SHELL_TOOLS: &[&str] = &["bash", "run_shell", "mcp_exec_command", "create_terminal"];
const PATH_TOOLS: &[&str] = &[
    "read_file",
    "write_file",
    "edit_file",
    "read_text_file",
    "write_text_file",
    "write",
    "read",
    "glob_file_search",
    "glob",
    "list_dir",
    "list_files",
    "grep",
    "search_replace",
];
const NETWORK_TOOLS: &[&str] = &["mcp_fetch_webpage", "mcp_free_search", "mcp_paid_search"];

fn category(tool: &str) -> Option<&'static str> {
    if SHELL_TOOLS.contains(&tool) {
        Some("shell")
    } else if PATH_TOOLS.contains(&tool) {
        Some("path")
    } else if NETWORK_TOOLS.contains(&tool) {
        Some("network")
    } else {
        None
    }
}

/// 验证单条规则中的工具是否属于同一安全类别。
pub fn rule_tools_category_consistent(tools: &[String]) -> bool {
    let mut kind = None;
    for tool in tools {
        let Some(current) = category(tool) else {
            return false;
        };
        if kind.is_some_and(|previous| previous != current) {
            return false;
        }
        kind = Some(current);
    }
    kind.is_some()
}

fn parse_action(value: &Value) -> Option<PermissionLevel> {
    match value.as_str()?.trim().to_ascii_lowercase().as_str() {
        "allow" => Some(PermissionLevel::Allow),
        "ask" => Some(PermissionLevel::Ask),
        "deny" => Some(PermissionLevel::Deny),
        _ => None,
    }
}

fn command_text(args: &Value) -> &str {
    args.get("command")
        .and_then(Value::as_str)
        .or_else(|| args.get("cmd").and_then(Value::as_str))
        .unwrap_or("")
        .trim()
}
fn network_text(args: &Value) -> &str {
    args.get("url")
        .and_then(Value::as_str)
        .or_else(|| args.get("query").and_then(Value::as_str))
        .or_else(|| args.get("endpoint").and_then(Value::as_str))
        .unwrap_or("")
        .trim()
}

fn wildcard_match(text: &str, pattern: &str) -> bool {
    let text: Vec<char> = text.chars().collect();
    let pattern: Vec<char> = pattern.chars().collect();
    let mut table = vec![vec![false; text.len() + 1]; pattern.len() + 1];
    table[0][0] = true;
    for (pi, pc) in pattern.iter().enumerate() {
        for ti in 0..=text.len() {
            if *pc == '*' {
                table[pi + 1][ti] = table[pi][ti] || (ti > 0 && table[pi + 1][ti - 1]);
            } else if ti > 0 && (*pc == '?' || *pc == text[ti - 1]) {
                table[pi + 1][ti] = table[pi][ti - 1];
            }
        }
    }
    table[pattern.len()][text.len()]
}

fn pattern_matches(tool: &str, pattern: &str, args: &Value) -> bool {
    let Some(kind) = category(tool) else {
        return false;
    };
    match kind {
        "shell" => wildcard_match(command_text(args), pattern.trim()),
        // Path policy must go through file_guard; network rules match URL/query here.
        "path" => false,
        "network" => wildcard_match(network_text(args), pattern.trim()),
        _ => false,
    }
}

fn rule_tools(rule: &Value) -> Option<Vec<String>> {
    let raw = rule.get("tools")?;
    let values = match raw {
        Value::String(tool) => vec![tool.clone()],
        Value::Array(values) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => return None,
    };
    if rule_tools_category_consistent(&values) {
        Some(values)
    } else {
        None
    }
}

fn rule_hits(
    config_key: &str,
    rules: &Value,
    tool: &str,
    args: &Value,
) -> Vec<(PermissionLevel, String)> {
    let Some(rules) = rules.as_array() else {
        return Vec::new();
    };
    rules
        .iter()
        .filter_map(|rule| {
            let tools = rule_tools(rule)?;
            if !tools.iter().any(|name| name == tool) {
                return None;
            }
            let pattern = rule.get("pattern").and_then(Value::as_str)?.trim();
            if pattern.is_empty() || !pattern_matches(tool, pattern, args) {
                return None;
            }
            let level = rule.get("action").and_then(parse_action).or_else(|| {
                match rule.get("severity").and_then(Value::as_str) {
                    Some("LOW") | Some("low") => Some(PermissionLevel::Allow),
                    Some("MEDIUM") | Some("medium") => Some(PermissionLevel::Ask),
                    Some("HIGH") | Some("high") | Some("CRITICAL") | Some("critical") => {
                        Some(PermissionLevel::Ask)
                    }
                    _ => None,
                }
            })?;
            let id = rule.get("id").and_then(Value::as_str).unwrap_or("?");
            Some((level, format!("{config_key}[{id}]")))
        })
        .collect()
}

fn approval_override_hits(rules: &Value, tool: &str, args: &Value) -> Vec<String> {
    let Some(rules) = rules.as_array() else {
        return Vec::new();
    };
    rules
        .iter()
        .filter_map(|rule| {
            if rule.get("action").and_then(Value::as_str) != Some("allow")
                || rule.get("match_type").and_then(Value::as_str) == Some("path")
            {
                return None;
            }
            let tools = rule_tools(rule)?;
            if !tools.iter().any(|name| name == tool)
                || !pattern_matches(tool, rule.get("pattern")?.as_str()?, args)
            {
                return None;
            }
            Some(format!(
                "approval_overrides[{}]",
                rule.get("id").and_then(Value::as_str).unwrap_or("?")
            ))
        })
        .collect()
}

fn finalize(hits: &[(PermissionLevel, String)], prefix: &str) -> (PermissionLevel, String) {
    let level = strictest(&hits.iter().map(|(level, _)| *level).collect::<Vec<_>>());
    let contributing = hits
        .iter()
        .filter(|(candidate, _)| *candidate == level)
        .map(|(_, rule)| rule.as_str())
        .collect::<Vec<_>>()
        .join("+");
    (level, format!("tiered_policy:{prefix}:{contributing}"))
}

fn baseline(config: &Value, tool: &str) -> Option<(PermissionLevel, String)> {
    let raw = config.get("tools")?.get(tool)?;
    if let Some(level) = parse_action(raw) {
        return Some((level, format!("tiered_policy:tools.{tool}")));
    }
    raw.get("*")
        .and_then(parse_action)
        .map(|level| (level, format!("tiered_policy:tools.{tool}.*")))
}

fn shell_floor(tool: &str, args: &Value) -> Option<(PermissionLevel, String)> {
    if !SHELL_TOOLS.contains(&tool) {
        return None;
    }
    let parsed = crate::shell_ast::parse_shell_for_permission(command_text(args));
    if parsed.kind != "simple" || parsed.flags.has_risky_structure() {
        return Some((
            PermissionLevel::Ask,
            "tiered_policy:shell_ast:structure_guard".to_string(),
        ));
    }
    None
}

fn apply_floor(
    result: (PermissionLevel, String),
    floor: Option<&(PermissionLevel, String)>,
) -> (PermissionLevel, String) {
    let Some(floor) = floor else {
        return result;
    };
    let level = strictest(&[result.0, floor.0]);
    if level == result.0 {
        result
    } else {
        (level, format!("{}|{}", floor.1, result.1))
    }
}

/// 返回最终权限及 matched-rule 摘要。
///
/// 优先级：显式整工具 deny > 参数 deny > approval override > 参数命中 >
/// 整工具策略 > 默认策略。未知工具或空配置返回 `ask`。
pub fn evaluate_tiered_policy(
    config: &Value,
    tool: &str,
    args: &Value,
) -> (PermissionLevel, String) {
    let floor = shell_floor(tool, args);
    let explicit = baseline(config, tool);
    if explicit
        .as_ref()
        .is_some_and(|(level, _)| *level == PermissionLevel::Deny)
    {
        return explicit.expect("checked above");
    }
    let rules = rule_hits(
        "rules",
        config.get("rules").unwrap_or(&Value::Null),
        tool,
        args,
    );
    if rules
        .iter()
        .any(|(level, _)| *level == PermissionLevel::Deny)
    {
        return apply_floor(finalize(&rules, "rules"), floor.as_ref());
    }
    let overrides = approval_override_hits(
        config.get("approval_overrides").unwrap_or(&Value::Null),
        tool,
        args,
    );
    if !overrides.is_empty() {
        return apply_floor(
            (
                PermissionLevel::Allow,
                format!("tiered_policy:approval_overrides:{}", overrides.join("+")),
            ),
            floor.as_ref(),
        );
    }
    if !rules.is_empty() {
        return apply_floor(finalize(&rules, "rules"), floor.as_ref());
    }
    if let Some(explicit) = explicit {
        return apply_floor(explicit, floor.as_ref());
    }
    if let Some(default) = config
        .get("defaults")
        .and_then(|value| value.get("*"))
        .and_then(parse_action)
    {
        return apply_floor(
            (default, "tiered_policy:defaults.*".to_string()),
            floor.as_ref(),
        );
    }

    apply_floor(
        (
            PermissionLevel::Ask,
            "tiered_policy:fallback(no_config)".to_string(),
        ),
        floor.as_ref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn explicit_tool_policy_beats_default_and_rule_layers() {
        let config = json!({
            "tools": {"bash": "deny"},
            "defaults": {"*": "allow"},
            "rules": [{"id": "danger", "tools": ["bash"], "pattern": "rm *", "action": "allow"}]
        });
        let (level, rule) =
            evaluate_tiered_policy(&config, "bash", &json!({"command": "rm -rf /"}));
        assert_eq!(level, PermissionLevel::Deny);
        assert!(rule.contains("tools.bash"));
    }

    #[test]
    fn matching_user_rule_is_parameter_scoped() {
        let config = json!({
            "tools": {},
            "defaults": {"*": "deny"},
            "rules": [{"id": "git", "tools": ["bash"], "pattern": "git *", "action": "allow"}]
        });
        assert_eq!(
            evaluate_tiered_policy(&config, "bash", &json!({"command": "git status"})).0,
            PermissionLevel::Allow
        );
        assert_eq!(
            evaluate_tiered_policy(&config, "bash", &json!({"command": "python app.py"})).0,
            PermissionLevel::Deny
        );
    }

    #[test]
    fn approval_override_allows_matching_command_but_not_path_rule() {
        let config = json!({
            "tools": {"bash": "ask"},
            "approval_overrides": [
                {"id": "trusted", "tools": ["bash"], "pattern": "cargo test", "action": "allow"},
                {"id": "path", "tools": ["read_file"], "match_type": "path", "pattern": "tmp/*", "action": "allow"}
            ]
        });
        let command = evaluate_tiered_policy(&config, "bash", &json!({"command": "cargo test"}));
        let path = evaluate_tiered_policy(&config, "read_file", &json!({"path": "tmp/a"}));
        assert_eq!(command.0, PermissionLevel::Allow);
        assert!(command.1.contains("approval_overrides"));
        assert_eq!(path.0, PermissionLevel::Ask);
    }

    #[test]
    fn shell_ast_floor_escalates_chained_allow_to_ask() {
        let config = json!({"tools": {"bash": "allow"}});
        let (level, rule) = evaluate_tiered_policy(
            &config,
            "bash",
            &json!({"command": "echo ok && echo again"}),
        );
        assert_eq!(level, PermissionLevel::Ask);
        assert!(rule.contains("shell_ast"));
    }
    #[test]
    fn network_rule_matches_url_argument() {
        let config = json!({
            "tools": {"mcp_fetch_webpage": "allow"},
            "rules": [{
                "id": "blocked-host",
                "tools": ["mcp_fetch_webpage"],
                "pattern": "https://blocked.example/*",
                "action": "deny"
            }]
        });
        let (level, rule) = evaluate_tiered_policy(
            &config,
            "mcp_fetch_webpage",
            &json!({"url": "https://blocked.example/private"}),
        );
        assert_eq!(level, PermissionLevel::Deny);
        assert!(rule.contains("blocked-host"));
    }
}
