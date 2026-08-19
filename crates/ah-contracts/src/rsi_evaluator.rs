//! rsi_evaluator seam:RSI 评估的确定性轨迹工具。
//!
//! 对齐 `openjiuwen/rsi/evaluator/trajectory_paths.py` + `trajectory_usage.py`:
//! - **bounded 轨迹**:`truncate_text` / `truncate_json_like` / `bounded_messages`
//!   (头尾各半保留 + 省略计数)/ `tool_summary` / `safe_role_file_stem` /
//!   `bounded_trajectory_dict`(LLM/tool detail 有界化);
//! - **usage 提取**:`collect_successful_tool_names` / `collect_successful_skill_names`
//!   (仅成功完成的调用)/ `collect_pre_edit_successful_usage`(首次持久编辑前)。
//!
//! 契约零实现:文件 IO / JSONL 读取由插件或调用方注入。

use crate::seam::Seam;

/// 角色轨迹目录名(对齐 ROLE_TRAJECTORY_DIR_NAME = "tr")。
pub const ROLE_TRAJECTORY_DIR_NAME: &str = "tr";
/// 轨迹事件文件名(对齐 TRAJECTORY_EVENTS_FILE_NAME)。
pub const TRAJECTORY_EVENTS_FILE_NAME: &str = "trajectory_events.jsonl";
/// 最多保存的 LLM 消息数(对齐 MAX_SAVED_LLM_MESSAGES = 4)。
pub const MAX_SAVED_LLM_MESSAGES: usize = 4;
/// 保存文本最大字符数(对齐 MAX_SAVED_TEXT_CHARS = 1200)。
pub const MAX_SAVED_TEXT_CHARS: usize = 1200;
/// 工具结果最大字符数(对齐 MAX_SAVED_TOOL_RESULT_CHARS = 1200)。
pub const MAX_SAVED_TOOL_RESULT_CHARS: usize = 1200;

/// 截断文本(对齐 `_truncate_text`):超长追加 `...[truncated N chars]`。
pub fn truncate_text(value: Option<&str>, max_chars: usize) -> Option<String> {
    match value {
        None => None,
        Some(v) => {
            if v.len() <= max_chars {
                Some(v.to_string())
            } else {
                Some(format!(
                    "{}...[truncated {} chars]",
                    &v[..max_chars],
                    v.len() - max_chars
                ))
            }
        }
    }
}

/// 递归截断 JSON 值中的文本(对齐 `_truncate_json_like`)。
pub fn truncate_json_like(value: serde_json::Value, max_text_chars: usize) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) => {
            if s.len() <= max_text_chars {
                serde_json::Value::String(s)
            } else {
                serde_json::Value::String(format!(
                    "{}...[truncated {} chars]",
                    &s[..max_text_chars],
                    s.len() - max_text_chars
                ))
            }
        }
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .into_iter()
                .map(|i| truncate_json_like(i, max_text_chars))
                .collect(),
        ),
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, truncate_json_like(v, max_text_chars)))
                .collect(),
        ),
        other => other,
    }
}

/// 有界消息列表(对齐 `_bounded_messages`):≤4 全保留,否则头尾各半。
pub fn bounded_messages(messages: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let kept: Vec<serde_json::Value> = if messages.len() <= MAX_SAVED_LLM_MESSAGES {
        messages.to_vec()
    } else {
        let head_count = MAX_SAVED_LLM_MESSAGES / 2;
        let tail_count = MAX_SAVED_LLM_MESSAGES - head_count;
        let mut out = Vec::new();
        out.extend_from_slice(&messages[..head_count]);
        out.extend_from_slice(&messages[messages.len() - tail_count..]);
        out
    };
    kept.into_iter()
        .map(|m| truncate_json_like(m, MAX_SAVED_TEXT_CHARS))
        .collect()
}

/// 工具摘要(对齐 `_tool_summary`):取 function.name / name / tool_name。
pub fn tool_summary(tool: Option<&serde_json::Value>) -> serde_json::Value {
    let tool = match tool {
        Some(t) => t,
        None => {
            return serde_json::json!({"name": "unknown"});
        }
    };
    let mut name: Option<String> = None;
    if let Some(function) = tool.get("function").and_then(|f| f.as_object()) {
        name = function
            .get("name")
            .and_then(|n| n.as_str())
            .map(str::to_string);
    }
    let name = name.or_else(|| {
        tool.get("name")
            .and_then(|n| n.as_str())
            .map(str::to_string)
    });
    let name = name.or_else(|| {
        tool.get("tool_name")
            .and_then(|n| n.as_str())
            .map(str::to_string)
    });
    let name = name.unwrap_or_else(|| "unknown".to_string());
    let name = truncate_text(Some(&name), MAX_SAVED_TEXT_CHARS).unwrap_or_default();
    let mut summary = serde_json::Map::new();
    summary.insert("name".to_string(), serde_json::Value::String(name));
    if let Some(tool_type) = tool.get("type").and_then(|t| t.as_str()) {
        summary.insert(
            "type".to_string(),
            serde_json::Value::String(tool_type.to_string()),
        );
    }
    serde_json::Value::Object(summary)
}

/// 安全角色文件名(对齐 `_safe_role_file_stem`):非法字符 → `_`,strip,空回退。
pub fn safe_role_file_stem(value: Option<&str>) -> String {
    let raw = value.unwrap_or("unknown");
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned
        .trim_matches(|c| c == '_' || c == '.' || c == '-')
        .to_string();
    if cleaned.is_empty() {
        "unknown".to_string()
    } else {
        cleaned
    }
}

/// LLM detail 有界化(对齐 `_bound_llm_detail`)。
pub fn bound_llm_detail(detail: &mut serde_json::Value) {
    if let Some(messages) = detail.get("messages").and_then(|m| m.as_array()) {
        let bounded = bounded_messages(messages);
        let omitted = messages.len().saturating_sub(bounded.len());
        detail["messages"] = serde_json::Value::Array(bounded);
        if omitted > 0 {
            let meta = detail.get_mut("meta").and_then(|m| m.as_object_mut());
            match meta {
                Some(m) => {
                    m.insert(
                        "omitted_message_count".to_string(),
                        serde_json::json!(omitted),
                    );
                }
                None => {
                    let mut new_meta = serde_json::Map::new();
                    new_meta.insert(
                        "omitted_message_count".to_string(),
                        serde_json::json!(omitted),
                    );
                    detail["meta"] = serde_json::Value::Object(new_meta);
                }
            }
        }
    }
    if let Some(tools) = detail.get("tools").and_then(|t| t.as_array()) {
        detail["tools"] =
            serde_json::Value::Array(tools.iter().map(|t| tool_summary(Some(t))).collect());
    }
    if let Some(response) = detail.get("response") {
        detail["response"] = truncate_json_like(response.clone(), MAX_SAVED_TEXT_CHARS);
    }
}

/// tool detail 有界化(对齐 `_bound_tool_detail`)。
pub fn bound_tool_detail(detail: &mut serde_json::Value) {
    if let Some(args) = detail.get("call_args") {
        detail["call_args"] = truncate_json_like(args.clone(), MAX_SAVED_TOOL_RESULT_CHARS);
    }
    if let Some(result) = detail.get("call_result") {
        detail["call_result"] = truncate_json_like(result.clone(), MAX_SAVED_TOOL_RESULT_CHARS);
    }
    if let Some(desc) = detail.get("tool_description") {
        detail["tool_description"] = truncate_json_like(desc.clone(), MAX_SAVED_TEXT_CHARS);
    }
    if detail.get("tool_schema").is_some() {
        detail["tool_schema"] = serde_json::json!({"omitted": true});
    }
}

/// 步骤是否成功完成的工具调用。
fn step_tool_succeeded(step: &serde_json::Value) -> bool {
    let detail = match step.get("detail") {
        Some(d) if d.is_object() => d,
        _ => return false,
    };
    if detail.get("tool_name").is_none() {
        return false;
    }
    // 成功:无 error(缺失或 null 均视为无错误)。
    match detail.get("error") {
        None => true,
        Some(e) => e.is_null(),
    }
}

/// 规范工具名(对齐 `_canonical_tool_name`):strip + lower + `-`→`_` +
/// 去 `_tool` 后缀;再取命名空间末段。
pub fn canonical_tool_name(tool_name: &str) -> String {
    let normalized = tool_name.trim().to_lowercase().replace('-', "_");
    let stripped = normalized.strip_suffix("_tool").unwrap_or(&normalized);
    stripped.rsplit('.').next().unwrap_or("").to_string()
}

/// 收集成功完成的工具名(对齐 `collect_successful_tool_names`)。
pub fn collect_successful_tool_names(value: &serde_json::Value, names: &mut Vec<String>) {
    let steps = value.get("steps").and_then(|s| s.as_array());
    if let Some(steps) = steps {
        for step in steps {
            if !step_tool_succeeded(step) {
                continue;
            }
            if let Some(detail) = step.get("detail").and_then(|d| d.as_object()) {
                let tool_name = detail
                    .get("tool_name")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !tool_name.is_empty() && !names.contains(&tool_name) {
                    names.push(tool_name);
                }
            }
        }
    }
}

/// 收集成功完成的技能名(对齐 `collect_successful_skill_names`):仅 skill_tool。
pub fn collect_successful_skill_names(value: &serde_json::Value, names: &mut Vec<String>) {
    let steps = value.get("steps").and_then(|s| s.as_array());
    if let Some(steps) = steps {
        for step in steps {
            if !step_tool_succeeded(step) {
                continue;
            }
            let detail = match step.get("detail").and_then(|d| d.as_object()) {
                Some(d) => d,
                None => continue,
            };
            let tool_name = detail
                .get("tool_name")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .trim();
            if canonical_tool_name(tool_name) != "skill" {
                continue;
            }
            for key in ["call_args", "arguments", "tool_input", "input"] {
                add_skill_name_from_args(detail.get(key), names);
            }
        }
    }
}

/// 从 skill_tool 参数提取技能名(对齐 `_add_skill_name_from_args`)。
pub fn add_skill_name_from_args(args: Option<&serde_json::Value>, names: &mut Vec<String>) {
    let args = match args {
        Some(a) => a,
        None => return,
    };
    // 参数可能是 dict(skill_name 键)或字符串。
    let skill_name = match args {
        serde_json::Value::Object(map) => map
            .get("skill_name")
            .or_else(|| map.get("name"))
            .and_then(|v| v.as_str())
            .map(str::to_string),
        serde_json::Value::String(s) => Some(s.clone()),
        _ => None,
    };
    if let Some(name) = skill_name {
        let name = name.trim().to_string();
        if !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
    }
}

/// 首次持久编辑前收集成功 usage(对齐 `collect_pre_edit_successful_usage`)。
///
/// 返回首个成功持久编辑步的索引(None = 无编辑)。
pub fn collect_pre_edit_successful_usage(
    value: &serde_json::Value,
    mut tool_names: Option<&mut Vec<String>>,
    mut skill_names: Option<&mut Vec<String>>,
) -> Option<usize> {
    let steps = value.get("steps").and_then(|s| s.as_array());
    let mut first_edit: Option<usize> = None;
    if let Some(steps) = steps {
        for (idx, step) in steps.iter().enumerate() {
            if !step_tool_succeeded(step) {
                continue;
            }
            let detail = match step.get("detail").and_then(|d| d.as_object()) {
                Some(d) => d,
                None => continue,
            };
            if first_edit.is_none() {
                if let Some(tool_names) = tool_names.as_mut() {
                    let tool_name = detail
                        .get("tool_name")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    if !tool_name.is_empty() && !tool_names.contains(&tool_name) {
                        tool_names.push(tool_name);
                    }
                }
                if let Some(skill_names) = skill_names.as_mut() {
                    let tool_name = detail
                        .get("tool_name")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .trim();
                    if canonical_tool_name(tool_name) == "skill" {
                        for key in ["call_args", "arguments", "tool_input", "input"] {
                            add_skill_name_from_args(detail.get(key), skill_names);
                        }
                    }
                }
            }
            if first_edit.is_none() && is_persistent_edit_step(step) {
                first_edit = Some(idx);
            }
        }
    }
    first_edit
}

/// 是否持久编辑步(对齐 `_is_persistent_edit_step` 的核心判据)。
pub fn is_persistent_edit_step(step: &serde_json::Value) -> bool {
    let tool_name = step
        .get("detail")
        .and_then(|d| d.get("tool_name"))
        .and_then(|t| t.as_str())
        .unwrap_or("");
    matches!(
        canonical_tool_name(tool_name).as_str(),
        "write_file" | "edit_file" | "remove_file" | "create_file"
    )
}

/// 轨迹有界工具 Seam(Service Definition)。
pub trait RsiTrajectoryTools: Seam {
    /// 有界化整条轨迹 dict(对齐 `_bounded_trajectory_dict`)。
    fn bounded_trajectory_dict(&self, data: serde_json::Value) -> serde_json::Value;

    /// 从轨迹收集成功工具名。
    fn collect_successful_tool_names(&self, value: &serde_json::Value, names: &mut Vec<String>);

    /// 从轨迹收集成功技能名。
    fn collect_successful_skill_names(&self, value: &serde_json::Value, names: &mut Vec<String>);
}

/// 整条轨迹有界化(对齐 `_bounded_trajectory_dict`)。
pub fn bounded_trajectory_dict(data: serde_json::Value) -> serde_json::Value {
    let mut bounded = truncate_json_like(data, MAX_SAVED_TEXT_CHARS);
    let steps = bounded.get("steps").and_then(|s| s.as_array()).cloned();
    if let Some(steps) = steps {
        let new_steps: Vec<serde_json::Value> = steps
            .into_iter()
            .map(|mut step| {
                if let Some(detail) = step.get("detail").and_then(|d| d.as_object()).cloned() {
                    let mut new_detail = serde_json::Value::Object(detail);
                    if new_detail.get("messages").is_some() {
                        bound_llm_detail(&mut new_detail);
                    } else if new_detail.get("tool_name").is_some() {
                        bound_tool_detail(&mut new_detail);
                    }
                    if let Some(obj) = step.as_object_mut() {
                        obj.insert("detail".to_string(), new_detail);
                    }
                }
                step
            })
            .collect();
        if let Some(obj) = bounded.as_object_mut() {
            obj.insert("steps".to_string(), serde_json::Value::Array(new_steps));
        }
    }
    bounded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_text_appends_suffix() {
        assert_eq!(truncate_text(Some("short"), 10), Some("short".to_string()));
        let long = "a".repeat(20);
        let t = truncate_text(Some(&long), 10).expect("truncated");
        assert!(t.starts_with("aaaaaaaaaa...[truncated 10 chars]"));
    }

    #[test]
    fn truncate_json_recursive() {
        let value = serde_json::json!({"a": "x".repeat(30), "b": [1, {"c": "y".repeat(30)}]});
        let t = truncate_json_like(value, 10);
        assert!(t["a"].as_str().unwrap().contains("[truncated"));
        assert!(t["b"][1]["c"].as_str().unwrap().contains("[truncated"));
    }

    #[test]
    fn bounded_messages_head_tail() {
        let msgs: Vec<serde_json::Value> = (0..6).map(|i| serde_json::json!({"i": i})).collect();
        let bounded = bounded_messages(&msgs);
        assert_eq!(bounded.len(), 4);
        assert_eq!(bounded[0]["i"], 0);
        assert_eq!(bounded[3]["i"], 5);
        // ≤4 全保留。
        let small: Vec<serde_json::Value> = (0..3).map(|i| serde_json::json!({"i": i})).collect();
        assert_eq!(bounded_messages(&small).len(), 3);
    }

    #[test]
    fn tool_summary_extracts_name() {
        let tool = serde_json::json!({"function": {"name": "bash"}, "type": "function"});
        let summary = tool_summary(Some(&tool));
        assert_eq!(summary["name"], "bash");
        assert_eq!(summary["type"], "function");
        // 无 function → 回退 name。
        let tool2 = serde_json::json!({"name": "read_file"});
        assert_eq!(tool_summary(Some(&tool2))["name"], "read_file");
    }

    #[test]
    fn safe_role_stem_sanitizes() {
        assert_eq!(safe_role_file_stem(Some("leader")), "leader");
        assert_eq!(safe_role_file_stem(Some("  my role! ")), "my_role");
        assert_eq!(safe_role_file_stem(None), "unknown");
    }

    #[test]
    fn bound_llm_and_tool_details() {
        let mut detail = serde_json::json!({
            "messages": [{"role": "user", "content": "x".repeat(1500)}],
            "tools": [{"function": {"name": "bash"}}],
            "response": "y".repeat(1500),
        });
        bound_llm_detail(&mut detail);
        assert!(
            detail["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("[truncated")
        );
        assert_eq!(detail["tools"][0]["name"], "bash");
        assert!(detail["response"].as_str().unwrap().contains("[truncated"));

        let mut tool_detail = serde_json::json!({
            "tool_name": "bash",
            "call_args": {"cmd": "z".repeat(1500)},
            "tool_schema": {"type": "object"},
        });
        bound_tool_detail(&mut tool_detail);
        assert!(
            tool_detail["call_args"]["cmd"]
                .as_str()
                .unwrap()
                .contains("[truncated")
        );
        assert_eq!(
            tool_detail["tool_schema"],
            serde_json::json!({"omitted": true})
        );
    }

    #[test]
    fn collect_usage_from_trajectory() {
        let traj = serde_json::json!({
            "steps": [
                {"detail": {"tool_name": "read_file", "error": null}},
                {"detail": {"tool_name": "bash", "error": "boom"}},
                {"detail": {"tool_name": "skill_tool", "call_args": {"skill_name": "my-skill"}}},
                {"detail": {"tool_name": "write_file"}},
            ]
        });
        let mut tools = Vec::new();
        collect_successful_tool_names(&traj, &mut tools);
        // read_file 成功、bash 失败跳过、skill_tool 成功、write_file 成功。
        assert!(tools.contains(&"read_file".to_string()));
        assert!(!tools.contains(&"bash".to_string()));
        assert!(tools.contains(&"write_file".to_string()));

        let mut skills = Vec::new();
        collect_successful_skill_names(&traj, &mut skills);
        assert_eq!(skills, vec!["my-skill".to_string()]);

        // pre-edit:write_file 是首个编辑,其前成功调用 read_file 计入。
        let mut pre_tools = Vec::new();
        let edit_idx = collect_pre_edit_successful_usage(&traj, Some(&mut pre_tools), None);
        assert_eq!(edit_idx, Some(3));
        assert!(pre_tools.contains(&"read_file".to_string()));
    }

    #[test]
    fn canonical_tool_name_last_segment() {
        // skill_tool → skill;命名空间末段;大小写/连字符归一。
        assert_eq!(canonical_tool_name("skill_tool"), "skill");
        assert_eq!(canonical_tool_name("team.SKILL-tool"), "skill");
        assert_eq!(canonical_tool_name("bash"), "bash");
        assert_eq!(canonical_tool_name("team.read_file"), "read_file");
    }
}
