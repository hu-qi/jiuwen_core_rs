//! 文件路径防护(对齐 Python harness/security/file_guard.py)。
//!
//! Pipeline B:独立于工具级 tiered_policy(Pipeline A)。`file_guard.enabled`
//! 关闭时整层不参与判定。判定入口为 [`FileGuardChecker`],支持
//! legacy(develop,外部目录等价检查)与 native(显式 defaults/workspace/paths)
//! 两种语义,与 Python `normalize_path_guard_config` + `FileGuardChecker`
//! 确定性部分 1:1。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use ah_contracts::security::{
    EffectiveFileGuardConfig, FileGuardAction, FileGuardAxisDefaults, FileGuardMatch,
    FileGuardMode, FileGuardPathRule, PermissionLevel, PermissionResult, axis_from_star,
    compile_path_entry, level_for_action, looks_like_path, match_glob, parse_level, strictest,
};
use serde_json::Value;

/// 写类工具(对齐 `_WRITE_PATH_TOOLS`)。
pub const WRITE_PATH_TOOLS: &[&str] = &[
    "write_file",
    "edit_file",
    "write_text_file",
    "write",
    "search_replace",
];

/// 路径感知 shell 命令(对齐 `_PATH_AWARE_COMMANDS`)。
pub const PATH_AWARE_COMMANDS: &[&str] = &[
    "cd", "rm", "cp", "mv", "mkdir", "touch", "chmod", "chown", "cat", "ls", "dir", "type", "del",
    "rd", "copy", "move", "md", "head", "tail", "more", "less", "vim", "nano", "gedit", "notepad",
];

/// 路径类工具(对齐 `_PATH_TOOLS`)。
pub const PATH_TOOLS: &[&str] = &[
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

/// 判定 `_has_axis_dict`:dict 且含 read/write/exec 任一键。
fn has_axis_dict(raw: Option<&Value>) -> bool {
    matches!(raw, Some(Value::Object(map)) if map.contains_key("read") || map.contains_key("write") || map.contains_key("exec"))
}

/// 判定是否进入 Native 语义(对齐 `_has_native_file_guard`)。
fn has_native_file_guard(fg: &serde_json::Map<String, Value>, perms: &Value) -> bool {
    if has_axis_dict(fg.get("defaults")) || has_axis_dict(fg.get("workspace")) {
        return true;
    }
    let paths = fg.get("paths");
    let Some(paths) = paths.and_then(Value::as_array) else {
        return false;
    };
    if paths.iter().any(|p| {
        p.as_object()
            .and_then(|o| o.get("match"))
            .and_then(Value::as_str)
            == Some("glob")
    }) {
        return true;
    }
    let ext = perms.get("external_directory");
    let has_ext = matches!(ext, Some(Value::Object(_)) | Some(Value::String(_)))
        && !matches!(ext, Some(Value::Null));
    !has_ext
}

/// 显式 enabled(对齐 `_explicit_enabled`)。
fn explicit_enabled(fg: &serde_json::Map<String, Value>) -> Option<bool> {
    fg.get("enabled").and_then(Value::as_bool)
}

/// 编译 workspace 轴规则(对齐 `_compile_workspace_axis_rule`)。
fn compile_workspace_axis_rule(
    workspace_cfg: Option<&Value>,
    workspace_root: Option<&str>,
) -> Option<FileGuardPathRule> {
    let workspace_cfg = workspace_cfg?;
    if !has_axis_dict(Some(workspace_cfg)) {
        return None;
    }
    let root = workspace_root?;
    let obj = workspace_cfg.as_object()?;
    compile_path_entry(
        root,
        obj.get("read")
            .and_then(Value::as_str)
            .map(|s| parse_level(s, PermissionLevel::Ask)),
        obj.get("write")
            .and_then(Value::as_str)
            .map(|s| parse_level(s, PermissionLevel::Ask)),
        obj.get("exec")
            .and_then(Value::as_str)
            .map(|s| parse_level(s, PermissionLevel::Ask)),
        FileGuardMatch::Prefix,
        PermissionLevel::Ask,
    )
}

/// 编译配置为生效视图(对齐 `normalize_path_guard_config`)。
pub fn normalize_path_guard_config(
    permissions: &Value,
    workspace_root: Option<&str>,
    trusted_dirs: &[&str],
) -> EffectiveFileGuardConfig {
    let perms = permissions.as_object().cloned().unwrap_or_default();
    let fg_raw = perms.get("file_guard");
    let fg: serde_json::Map<String, Value> = match fg_raw {
        Some(Value::Object(map)) => map.clone(),
        _ => serde_json::Map::new(),
    };
    let ext = perms.get("external_directory");
    let has_ext = matches!(ext, Some(Value::Object(_)) | Some(Value::String(_)));
    let has_trusted = !trusted_dirs.is_empty();

    let enabled = match explicit_enabled(&fg) {
        Some(explicit) => explicit,
        None => has_ext || has_trusted,
    };

    let native = has_native_file_guard(&fg, permissions);

    if !enabled {
        return EffectiveFileGuardConfig {
            enabled: false,
            mode: if native {
                FileGuardMode::Native
            } else {
                FileGuardMode::Legacy
            },
            defaults: FileGuardAxisDefaults {
                read: PermissionLevel::Ask,
                write: PermissionLevel::Ask,
                exec: PermissionLevel::Ask,
            },
            paths: vec![],
            workspace_root: workspace_root.map(str::to_string),
        };
    }

    if native {
        normalize_native(&fg, ext, workspace_root, trusted_dirs)
    } else {
        normalize_legacy(ext, workspace_root, trusted_dirs, &fg)
    }
}

fn normalize_legacy(
    ext: Option<&Value>,
    workspace_root: Option<&str>,
    trusted_dirs: &[&str],
    fg: &serde_json::Map<String, Value>,
) -> EffectiveFileGuardConfig {
    let mut star_action = "ask".to_string();
    let mut allow_prefixes: Vec<(String, String)> = Vec::new();
    if let Some(Value::String(s)) = ext {
        star_action = s.clone();
    } else if let Some(Value::Object(map)) = ext {
        if let Some(Value::String(s)) = map.get("*") {
            star_action = s.clone();
        }
        for (key, action) in map {
            if key == "*" {
                continue;
            }
            if let Value::String(a) = action
                && matches!(a.as_str(), "allow" | "ask" | "deny")
            {
                allow_prefixes.push((key.clone(), a.clone()));
            }
        }
    }

    let defaults = axis_from_star(&star_action, PermissionLevel::Ask);
    let mut rules: Vec<FileGuardPathRule> = Vec::new();

    // 隐式 workspace 全放行(仅 Legacy)。
    if let Some(ws) = workspace_root
        && let Some(rule) = compile_path_entry(
            ws,
            Some(PermissionLevel::Allow),
            Some(PermissionLevel::Allow),
            Some(PermissionLevel::Allow),
            FileGuardMatch::Prefix,
            PermissionLevel::Ask,
        )
    {
        rules.push(rule);
    }
    for td in trusted_dirs {
        if let Some(rule) = compile_path_entry(
            td,
            Some(PermissionLevel::Allow),
            Some(PermissionLevel::Allow),
            Some(PermissionLevel::Ask),
            FileGuardMatch::Prefix,
            PermissionLevel::Ask,
        ) {
            rules.push(rule);
        }
    }
    for (key, action) in &allow_prefixes {
        let (r, w, e) = if action == "allow" {
            (
                PermissionLevel::Allow,
                PermissionLevel::Allow,
                PermissionLevel::Ask,
            )
        } else {
            let level = parse_level(action, PermissionLevel::Ask);
            (level, level, level)
        };
        if let Some(rule) = compile_path_entry(
            key,
            Some(r),
            Some(w),
            Some(e),
            FileGuardMatch::Prefix,
            PermissionLevel::Ask,
        ) {
            rules.push(rule);
        }
    }

    // /add-dir 写入的 file_guard.paths(无 glob)并入 Legacy。
    if let Some(Value::Array(paths)) = fg.get("paths") {
        for item in paths {
            let Some(obj) = item.as_object() else {
                continue;
            };
            if obj.get("match").and_then(Value::as_str) == Some("glob") {
                continue;
            }
            let Some(path_v) = obj.get("path").and_then(Value::as_str) else {
                continue;
            };
            if path_v.trim().is_empty() {
                continue;
            }
            let read = obj
                .get("read")
                .and_then(Value::as_str)
                .map(|s| parse_level(s, PermissionLevel::Allow))
                .unwrap_or(PermissionLevel::Allow);
            let write = obj
                .get("write")
                .and_then(Value::as_str)
                .map(|s| parse_level(s, PermissionLevel::Allow))
                .unwrap_or(PermissionLevel::Allow);
            let exec = obj
                .get("exec")
                .and_then(Value::as_str)
                .map(|s| parse_level(s, PermissionLevel::Ask))
                .unwrap_or(PermissionLevel::Ask);
            if let Some(rule) = compile_path_entry(
                path_v,
                Some(read),
                Some(write),
                Some(exec),
                FileGuardMatch::Prefix,
                PermissionLevel::Ask,
            ) {
                rules.push(rule);
            }
        }
    }

    EffectiveFileGuardConfig {
        enabled: true,
        mode: FileGuardMode::Legacy,
        defaults,
        paths: rules,
        workspace_root: workspace_root.map(str::to_string),
    }
}

fn normalize_native(
    fg: &serde_json::Map<String, Value>,
    ext: Option<&Value>,
    workspace_root: Option<&str>,
    trusted_dirs: &[&str],
) -> EffectiveFileGuardConfig {
    let raw_defaults = match fg.get("defaults") {
        Some(Value::Object(map)) => map.clone(),
        _ => serde_json::Map::new(),
    };
    let defaults = if !raw_defaults.contains_key("read")
        && !raw_defaults.contains_key("write")
        && !raw_defaults.contains_key("exec")
    {
        match ext {
            Some(Value::Object(map)) => axis_from_star(
                map.get("*").and_then(Value::as_str).unwrap_or("ask"),
                PermissionLevel::Ask,
            ),
            Some(Value::String(s)) => axis_from_star(s, PermissionLevel::Ask),
            _ => FileGuardAxisDefaults {
                read: PermissionLevel::Ask,
                write: PermissionLevel::Ask,
                exec: PermissionLevel::Ask,
            },
        }
    } else {
        FileGuardAxisDefaults {
            read: raw_defaults
                .get("read")
                .and_then(Value::as_str)
                .map(|s| parse_level(s, PermissionLevel::Ask))
                .unwrap_or(PermissionLevel::Ask),
            write: raw_defaults
                .get("write")
                .and_then(Value::as_str)
                .map(|s| parse_level(s, PermissionLevel::Ask))
                .unwrap_or(PermissionLevel::Ask),
            exec: raw_defaults
                .get("exec")
                .and_then(Value::as_str)
                .map(|s| parse_level(s, PermissionLevel::Ask))
                .unwrap_or(PermissionLevel::Ask),
        }
    };

    let mut rules: Vec<FileGuardPathRule> = Vec::new();
    if let Some(Value::Array(raw_paths)) = fg.get("paths") {
        for item in raw_paths {
            let Some(obj) = item.as_object() else {
                continue;
            };
            let Some(path_v) = obj.get("path").and_then(Value::as_str) else {
                continue;
            };
            if path_v.trim().is_empty() {
                continue;
            }
            let match_v = obj.get("match").and_then(Value::as_str);
            let r#match = if match_v == Some("glob") {
                FileGuardMatch::Glob
            } else {
                FileGuardMatch::Prefix
            };
            if let Some(rule) = compile_path_entry(
                path_v,
                obj.get("read")
                    .and_then(Value::as_str)
                    .map(|s| parse_level(s, PermissionLevel::Ask)),
                obj.get("write")
                    .and_then(Value::as_str)
                    .map(|s| parse_level(s, PermissionLevel::Ask)),
                obj.get("exec")
                    .and_then(Value::as_str)
                    .map(|s| parse_level(s, PermissionLevel::Ask)),
                r#match,
                PermissionLevel::Ask,
            ) {
                rules.push(rule);
            }
        }
    }

    if let Some(rule) = compile_workspace_axis_rule(fg.get("workspace"), workspace_root) {
        rules.push(rule);
    }

    for td in trusted_dirs {
        if let Some(rule) = compile_path_entry(
            td,
            Some(PermissionLevel::Allow),
            Some(PermissionLevel::Allow),
            Some(PermissionLevel::Ask),
            FileGuardMatch::Prefix,
            PermissionLevel::Ask,
        ) {
            rules.push(rule);
        }
    }

    // 旧 external_directory 具名键作迁移源。
    if let Some(Value::Object(ext_map)) = ext {
        let existing: HashSet<String> = rules
            .iter()
            .filter(|r| r.r#match == FileGuardMatch::Prefix)
            .map(|r| r.path.clone())
            .collect();
        for (key, action) in ext_map {
            if key == "*" {
                continue;
            }
            let Value::String(action) = action else {
                continue;
            };
            if !matches!(action.as_str(), "allow" | "ask" | "deny") {
                continue;
            }
            let key_norm = key.replace('\\', "/");
            if existing.contains(&key_norm) {
                continue;
            }
            let (r, w, e) = if action == "allow" {
                (
                    PermissionLevel::Allow,
                    PermissionLevel::Allow,
                    PermissionLevel::Ask,
                )
            } else {
                let level = parse_level(action, PermissionLevel::Ask);
                (level, level, level)
            };
            if let Some(rule) = compile_path_entry(
                key,
                Some(r),
                Some(w),
                Some(e),
                FileGuardMatch::Prefix,
                PermissionLevel::Ask,
            ) {
                rules.push(rule);
            }
        }
    }

    EffectiveFileGuardConfig {
        enabled: true,
        mode: FileGuardMode::Native,
        defaults,
        paths: rules,
        workspace_root: workspace_root.map(str::to_string),
    }
}

/// 从 shell 命令抽取路径(对齐 `_extract_paths_from_command`)。
pub fn extract_paths_from_command(command: &str, workdir: &Path) -> Vec<PathBuf> {
    let command = command.trim();
    if command.is_empty() {
        return vec![];
    }
    let tokens = crate::shell_ast::shlex_split_posix(command)
        .unwrap_or_else(|| command.split_whitespace().map(str::to_string).collect());
    if tokens.is_empty() {
        return vec![];
    }
    let cmd = tokens[0].to_lowercase();
    if !PATH_AWARE_COMMANDS.contains(&cmd.as_str()) {
        return vec![];
    }
    let base = workdir.to_path_buf();
    let mut paths: Vec<PathBuf> = Vec::new();
    for tok in tokens.iter().skip(1) {
        let tok = tok.trim().trim_matches('"').trim_matches('\'');
        if tok.is_empty() || tok.starts_with('-') {
            continue;
        }
        if !looks_like_path(tok) {
            continue;
        }
        let p = PathBuf::from(tok);
        let p = if p.is_absolute() { p } else { base.join(tok) };
        paths.push(normalize_path_components(&p));
    }
    paths
}

/// 规范化路径组件(去掉 `.` 组件;对齐 Python `Path` 行为)。
fn normalize_path_components(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        use std::path::Component;
        match component {
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    }
}

/// legacy 路径抽取(对齐 `extract_paths_legacy`)。
pub fn extract_paths_legacy(tool_name: &str, tool_args: &Value, workspace: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();
    let args = tool_args.as_object().cloned().unwrap_or_default();
    if matches!(tool_name, "mcp_exec_command" | "bash" | "create_terminal") {
        let workdir = args.get("workdir").and_then(Value::as_str).unwrap_or("");
        let workdir_resolved = if workdir.is_empty() {
            workspace.to_path_buf()
        } else {
            workspace.join(workdir)
        };
        let cmd = args
            .get("command")
            .or_else(|| args.get("cmd"))
            .and_then(Value::as_str)
            .unwrap_or("");
        paths = extract_paths_from_command(cmd, &workdir_resolved);
    } else if PATH_TOOLS.contains(&tool_name) {
        for (k, v) in &args {
            if !k.starts_with("_") {
                let Some(s) = v.as_str() else {
                    continue;
                };
                let raw = s.trim().trim_matches('"').trim_matches('\'');
                if raw.is_empty() || !looks_like_path(raw) {
                    continue;
                }
                let p = PathBuf::from(raw);
                let p = if p.is_absolute() {
                    p
                } else {
                    workspace.join(raw)
                };
                paths.push(normalize_path_components(&p));
            }
        }
    }
    paths
}

/// 工具默认动作(对齐 `_tool_default_action`)。
pub fn tool_default_action(tool_name: &str) -> FileGuardAction {
    if WRITE_PATH_TOOLS.contains(&tool_name) {
        FileGuardAction::Write
    } else {
        FileGuardAction::Read
    }
}

/// 路径防护判定器(对齐 `FileGuardChecker`)。
pub struct FileGuardChecker {
    effective: EffectiveFileGuardConfig,
}

impl FileGuardChecker {
    pub fn new(effective: EffectiveFileGuardConfig) -> Self {
        Self { effective }
    }

    /// 从权限配置构建(对齐 `build_file_guard_checker`);未启用返回 None。
    pub fn build(
        permissions: &Value,
        workspace_root: Option<&str>,
        trusted_dirs: &[&str],
    ) -> Option<Self> {
        let effective = normalize_path_guard_config(permissions, workspace_root, trusted_dirs);
        if !effective.enabled {
            return None;
        }
        Some(Self::new(effective))
    }

    pub fn enabled(&self) -> bool {
        self.effective.enabled
    }

    pub fn mode(&self) -> FileGuardMode {
        self.effective.mode
    }

    /// 评估路径层;无意见或全部 ALLOW 时返回 None(对齐 `evaluate`)。
    pub fn evaluate(&self, tool_name: &str, tool_args: &Value) -> Option<PermissionResult> {
        if !self.effective.enabled {
            return None;
        }
        let workspace = self.effective.workspace_root.as_ref()?;
        let workspace_path = PathBuf::from(workspace);

        let accesses: Vec<(PathBuf, FileGuardAction)> =
            if self.effective.mode == FileGuardMode::Native {
                extract_accesses_native(tool_name, tool_args, &workspace_path)
            } else {
                let paths = extract_paths_legacy(tool_name, tool_args, &workspace_path);
                let action = tool_default_action(tool_name);
                paths.into_iter().map(|p| (p, action)).collect()
            };

        if accesses.is_empty() {
            return None;
        }

        let mut overall = PermissionLevel::Allow;
        let mut hit_external: Vec<String> = Vec::new();
        let mut matched_bits: Vec<String> = Vec::new();

        for (p, action) in &accesses {
            let (level, rule_id) = self.resolve_one(p, *action);
            overall = strictest(&[overall, level]);
            if level != PermissionLevel::Allow {
                hit_external.push(p.to_string_lossy().to_string());
                if let Some(rule_id) = rule_id {
                    matched_bits.push(rule_id);
                }
            }
        }

        if overall == PermissionLevel::Allow {
            return None;
        }

        let (reason, matched) = if self.effective.mode == FileGuardMode::Legacy {
            let sample = accesses[0].0.to_string_lossy().to_string();
            let reason = if overall == PermissionLevel::Deny {
                format!("Access to paths outside workspace is denied: {sample}")
            } else {
                format!("Access to paths outside workspace requires approval: {sample}")
            };
            (reason, "external_directory.*".to_string())
        } else {
            let hint = hit_external
                .first()
                .cloned()
                .unwrap_or_else(|| accesses[0].0.to_string_lossy().to_string());
            let reason = if overall == PermissionLevel::Deny {
                format!("file_guard denied: {hint}")
            } else {
                format!("file_guard requires approval: {hint}")
            };
            let matched = if matched_bits.is_empty() {
                "file_guard".to_string()
            } else {
                matched_bits.join("|")
            };
            (reason, matched)
        };

        Some(PermissionResult {
            permission: overall,
            reason: Some(reason),
            matched_rule: Some(matched),
            external_paths: if hit_external.is_empty() {
                None
            } else {
                Some(hit_external)
            },
        })
    }

    /// 收集本次判定为 ASK 的 (path, action)(对齐 `collect_ask_accesses`)。
    pub fn collect_ask_accesses(
        &self,
        tool_name: &str,
        tool_args: &Value,
    ) -> Vec<(String, FileGuardAction)> {
        if !self.effective.enabled {
            return vec![];
        }
        let Some(workspace) = self.effective.workspace_root.as_ref() else {
            return vec![];
        };
        let workspace_path = PathBuf::from(workspace);
        let accesses: Vec<(PathBuf, FileGuardAction)> =
            if self.effective.mode == FileGuardMode::Native {
                extract_accesses_native(tool_name, tool_args, &workspace_path)
            } else {
                let paths = extract_paths_legacy(tool_name, tool_args, &workspace_path);
                let action = tool_default_action(tool_name);
                paths.into_iter().map(|p| (p, action)).collect()
            };
        let mut out: Vec<(String, FileGuardAction)> = Vec::new();
        let mut seen: HashSet<(String, FileGuardAction)> = HashSet::new();
        for (p, action) in accesses {
            let (level, _rule_id) = self.resolve_one(&p, action);
            if level != PermissionLevel::Ask {
                continue;
            }
            let key = (p.to_string_lossy().to_string(), action);
            if seen.insert(key.clone()) {
                out.push(key);
            }
        }
        out
    }

    fn resolve_one(
        &self,
        path: &Path,
        action: FileGuardAction,
    ) -> (PermissionLevel, Option<String>) {
        let path_posix = path.to_string_lossy().replace('\\', "/");
        let mut best_prefix: Option<(usize, FileGuardPathRule)> = None;
        let mut glob_hits: Vec<PermissionLevel> = Vec::new();

        for rule in &self.effective.paths {
            if rule.r#match == FileGuardMatch::Glob {
                let pattern = rule.path.replace('\\', "/");
                if match_glob(&pattern, &path_posix) {
                    glob_hits.push(level_for_action(rule, action));
                }
                continue;
            }
            // prefix。
            let prefix = rule.path.trim_end_matches('/');
            let prefix = prefix.to_string();
            if path_posix == prefix || path_posix.starts_with(&format!("{prefix}/")) {
                let ln = prefix.len();
                if best_prefix.is_none() || ln > best_prefix.as_ref().unwrap().0 {
                    best_prefix = Some((ln, rule.clone()));
                }
            }
        }

        let mut candidates: Vec<PermissionLevel> = Vec::new();
        let mut rule_id: Option<String> = None;
        if let Some((_, rule)) = &best_prefix {
            candidates.push(level_for_action(rule, action));
            rule_id = Some(format!("file_guard:prefix:{}", rule.path));
        }
        candidates.extend(glob_hits.iter().copied());
        if !glob_hits.is_empty() && rule_id.is_none() {
            rule_id = Some("file_guard:glob".to_string());
        }

        if !candidates.is_empty() {
            let level = strictest(&candidates);
            return (level, rule_id);
        }

        // 未命中 paths → defaults。
        let defaults = self.effective.defaults;
        let level = match action {
            FileGuardAction::Write => defaults.write,
            FileGuardAction::Exec => defaults.exec,
            FileGuardAction::Read => defaults.read,
        };
        (level, Some("file_guard:defaults".to_string()))
    }
}

/// Native 访问抽取(对齐 files/extract.extract_accesses_native 的路径+动作部分;
/// 来源(src)信息不参与判定)。
fn extract_accesses_native(
    tool_name: &str,
    tool_args: &Value,
    workspace: &Path,
) -> Vec<(PathBuf, FileGuardAction)> {
    // Native 复用 legacy 抽取(路径级一致;无多来源细分)。
    let mut accesses: Vec<(PathBuf, FileGuardAction)> = Vec::new();
    for p in extract_paths_legacy(tool_name, tool_args, workspace) {
        let action = tool_default_action(tool_name);
        accesses.push((p, action));
    }
    accesses
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn disabled_config_returns_none_checker() {
        let checker = FileGuardChecker::build(&json!({}), Some("/ws"), &[]);
        assert!(checker.is_none(), "no file_guard -> disabled");
    }

    #[test]
    fn legacy_enabled_via_external_directory() {
        let perms = json!({
            "external_directory": {"*": "ask", "/data": "allow"}
        });
        let effective = normalize_path_guard_config(&perms, Some("/ws"), &[]);
        assert!(effective.enabled);
        assert_eq!(effective.mode, FileGuardMode::Legacy);
        // workspace 隐式放行 + /data 放行。
        let checker = FileGuardChecker::build(&perms, Some("/ws"), &[]).expect("checker");
        // 工作区内路径放行。
        let result = checker.evaluate("read_file", &json!({"path": "/ws/a.txt"}));
        assert!(result.is_none(), "workspace implicit allow");
        // 工作区外 → ASK(external_directory.* ask)。
        let result = checker
            .evaluate("read_file", &json!({"path": "/etc/passwd"}))
            .expect("external ask");
        assert!(result.needs_approval());
        assert_eq!(result.matched_rule.as_deref(), Some("external_directory.*"));
        assert!(
            result
                .reason
                .as_deref()
                .unwrap()
                .contains("requires approval")
        );
    }

    #[test]
    fn legacy_deny_outside_workspace() {
        let perms = json!({
            "external_directory": {"*": "deny"}
        });
        let checker = FileGuardChecker::build(&perms, Some("/ws"), &[]).expect("checker");
        let result = checker
            .evaluate("read_file", &json!({"path": "/etc/passwd"}))
            .expect("deny");
        assert!(result.is_denied());
        assert!(result.reason.as_deref().unwrap().contains("is denied"));
    }

    #[test]
    fn native_defaults_and_prefix_rules() {
        let perms = json!({
            "file_guard": {
                "enabled": true,
                "defaults": {"read": "ask", "write": "deny", "exec": "ask"},
                "paths": [
                    {"path": "/data/public", "read": "allow", "write": "allow", "exec": "deny"}
                ]
            }
        });
        let checker = FileGuardChecker::build(&perms, Some("/ws"), &[]).expect("checker");
        assert_eq!(checker.mode(), FileGuardMode::Native);

        // 命中前缀规则 → read allow。
        let result = checker.evaluate("read_file", &json!({"path": "/data/public/x.txt"}));
        assert!(result.is_none(), "prefix allow");

        // 写命中规则 → rule.write=allow。
        let result = checker.evaluate("write_file", &json!({"path": "/data/public/x.txt"}));
        assert!(result.is_none(), "rule write allow");

        // 未命中规则路径 → defaults.write=deny。
        let result = checker
            .evaluate("write_file", &json!({"path": "/ws/other.txt"}))
            .expect("write deny");
        assert!(result.is_denied(), "defaults write deny");
        assert_eq!(result.matched_rule.as_deref(), Some("file_guard:defaults"));

        // 工作区外未命中 → defaults(read=ask)。
        let result = checker
            .evaluate("read_file", &json!({"path": "/etc/passwd"}))
            .expect("ask");
        assert!(result.needs_approval());
        assert_eq!(result.matched_rule.as_deref(), Some("file_guard:defaults"));
    }

    #[test]
    fn native_glob_rule_matches() {
        let perms = json!({
            "file_guard": {
                "enabled": true,
                "defaults": {"read": "ask"},
                "paths": [
                    {"path": "**/*.md", "read": "allow", "match": "glob"}
                ]
            }
        });
        let checker = FileGuardChecker::build(&perms, Some("/ws"), &[]).expect("checker");
        // 任意深度 .md 放行。
        let result = checker.evaluate("read_file", &json!({"path": "/ws/docs/readme.md"}));
        assert!(result.is_none(), "glob allow");
        // 非 .md → ask(未命中 glob → defaults)。
        let result = checker
            .evaluate("read_file", &json!({"path": "/ws/docs/readme.txt"}))
            .expect("ask");
        assert!(result.needs_approval());
        assert_eq!(result.matched_rule.as_deref(), Some("file_guard:defaults"));
    }

    #[test]
    fn shell_command_path_extraction() {
        let paths = extract_paths_from_command("rm -rf /tmp/build", Path::new("/ws"));
        assert!(
            paths
                .iter()
                .any(|p| p.to_string_lossy().ends_with("tmp/build"))
        );
        // 非路径感知命令 → 空。
        assert!(extract_paths_from_command("echo hello", Path::new("/ws")).is_empty());
        // 相对路径拼工作目录。
        let paths = extract_paths_from_command("cat ./a.txt", Path::new("/ws"));
        assert!(
            paths
                .iter()
                .any(|p| p.to_string_lossy().ends_with("ws/a.txt"))
        );
    }

    #[test]
    fn collect_ask_accesses_returns_ask_only() {
        let perms = json!({
            "file_guard": {
                "enabled": true,
                "defaults": {"read": "ask", "write": "ask"},
                "paths": [{"path": "/allow", "read": "allow"}]
            }
        });
        let checker = FileGuardChecker::build(&perms, Some("/ws"), &[]).expect("checker");
        let asks = checker.collect_ask_accesses(
            "read_file",
            &json!({"path": "/ws/a.txt", "file_path": "/outside.txt"}),
        );
        assert!(!asks.is_empty());
        // ASK 项路径带动作。
        assert!(
            asks.iter()
                .any(|(p, a)| p.contains("outside") && *a == FileGuardAction::Read)
        );
    }
}
