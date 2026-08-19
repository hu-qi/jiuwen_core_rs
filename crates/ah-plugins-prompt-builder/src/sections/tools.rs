//! Tools 列表内容 section(对齐 harness/prompts/sections/context.py 的 build_tools_content)。
//!
//! 纯逻辑:按首选顺序/分组/覆盖摘要渲染工具列表 + 去重规则 + bash/task_tool 使用原则。

use ah_contracts::prompt_builder::{PromptSection, section_name};
use std::collections::BTreeMap;

/// 隐藏工具(对齐 hidden_tools:cron_*)。
pub const HIDDEN_TOOLS: [&str; 7] = [
    "cron_list_jobs",
    "cron_get_job",
    "cron_create_job",
    "cron_update_job",
    "cron_delete_job",
    "cron_toggle_job",
    "cron_preview_job",
];

/// 首选顺序工具(对齐 preferred_order)。
pub const PREFERRED_ORDER: [&str; 13] = [
    "paid_search",
    "free_search",
    "fetch_webpage",
    "image_ocr",
    "visual_question_answering",
    "audio_transcription",
    "audio_question_answering",
    "audio_metadata",
    "video_understanding",
    "session_new",
    "session_cancel",
    "session_list",
    "cron",
];

/// 文件组(label, 用途描述;对齐 grouped_labels)。
pub fn file_groups_cn() -> [(&'static [&'static str], &'static str, &'static str); 2] {
    [
        (
            &["read_file", "write_file", "edit_file"],
            "read_file / write_file / edit_file",
            "文件读写编辑",
        ),
        (
            &["glob", "list_files", "grep"],
            "glob / list_files / grep",
            "文件搜索",
        ),
    ]
}

pub fn file_groups_en() -> [(&'static [&'static str], &'static str, &'static str); 2] {
    [
        (
            &["read_file", "write_file", "edit_file"],
            "read_file / write_file / edit_file",
            "Read, write, and edit files",
        ),
        (
            &["glob", "list_files", "grep"],
            "glob / list_files / grep",
            "Search files and file contents",
        ),
    ]
}

/// 记忆组(对齐 memory_group)。
pub const MEMORY_GROUP_NAMES: [&str; 5] = [
    "memory_search",
    "memory_get",
    "write_memory",
    "edit_memory",
    "read_memory",
];
pub const MEMORY_GROUP_LABEL: &str =
    "memory_search / memory_get / write_memory / edit_memory / read_memory";
pub const MEMORY_GROUP_CN: &str = "记忆系统";
pub const MEMORY_GROUP_EN: &str = "Memory system";

/// 摘要覆盖(对齐 summary_overrides)。
pub fn summary_overrides_cn() -> BTreeMap<&'static str, &'static str> {
    BTreeMap::from([
        ("paid_search", "付费联网搜索（配置 API 时优先使用）"),
        ("free_search", "免费搜索（DuckDuckGo 等）"),
        ("fetch_webpage", "抓取网页文本内容"),
        ("image_ocr", "读取图片中的文字"),
        ("visual_question_answering", "理解图片内容并回答问题"),
        ("audio_transcription", "转写音频文件"),
        ("audio_question_answering", "理解音频内容并回答"),
        ("audio_metadata", "识别音频时长和歌曲信息"),
        ("video_understanding", "分析视频内容"),
        ("session_new", "创建多个协程任务（子 agent 异步运行）"),
        ("session_cancel", "取消正在运行的协程"),
        ("session_list", "查看所有协程状态"),
        ("cron", "管理定时任务与提醒"),
        ("bash", "执行 Shell 命令"),
        ("code", "执行 Python 或 JavaScript 代码"),
        ("list_skill", "列出可用技能"),
        ("task_tool", "启动临时子代理处理复杂任务"),
    ])
}

pub fn summary_overrides_en() -> BTreeMap<&'static str, &'static str> {
    BTreeMap::from([
        ("paid_search", "Paid web search (preferred when configured)"),
        ("free_search", "Free web search"),
        ("fetch_webpage", "Fetch webpage text"),
        ("image_ocr", "Read text from images"),
        (
            "visual_question_answering",
            "Understand images and answer questions",
        ),
        ("audio_transcription", "Transcribe audio"),
        (
            "audio_question_answering",
            "Understand audio and answer questions",
        ),
        (
            "audio_metadata",
            "Identify audio duration and song metadata",
        ),
        ("video_understanding", "Analyze video content"),
        ("session_new", "Create async sub-agent sessions"),
        ("session_cancel", "Cancel a running sub-agent session"),
        ("session_list", "List sub-agent session status"),
        ("cron", "Manage scheduled jobs and reminders"),
        ("bash", "Run shell commands"),
        ("code", "Run Python or JavaScript code"),
        ("list_skill", "List available skills"),
        ("task_tool", "Launch a temporary sub-agent for complex work"),
    ])
}

/// 工具摘要(覆盖优先,否则取描述;对齐 _tool_summary)。
pub fn tool_summary(
    name: &str,
    tool_descriptions: &BTreeMap<String, String>,
    language: &str,
) -> String {
    let overrides = if language == "en" {
        summary_overrides_en()
    } else {
        summary_overrides_cn()
    };
    if let Some(ov) = overrides.get(name) {
        return ov.to_string();
    }
    tool_descriptions
        .get(name)
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// 构建工具列表内容(对齐 build_tools_content;无可用工具返回 None)。
pub fn build_tools_content(
    tool_descriptions: &BTreeMap<String, String>,
    language: &str,
) -> Option<String> {
    if tool_descriptions.is_empty() {
        return None;
    }
    let header = if language == "en" {
        "# Available Tools"
    } else {
        "# 可用工具"
    };
    let mut lines: Vec<String> = vec![header.to_string(), String::new()];
    let mut rendered: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let cn = language == "cn";

    for name in PREFERRED_ORDER {
        if tool_descriptions.contains_key(name) && !HIDDEN_TOOLS.contains(&name) {
            lines.push(format!(
                "- {name}: {}",
                tool_summary(name, tool_descriptions, language)
            ));
            rendered.insert(name.to_string());
        }
    }

    let groups = if cn {
        file_groups_cn()
    } else {
        file_groups_en()
    };
    for (group_names, label, summary) in groups {
        let existing: Vec<&str> = group_names
            .iter()
            .copied()
            .filter(|n| tool_descriptions.contains_key(*n) && !HIDDEN_TOOLS.contains(n))
            .collect();
        if existing.len() == group_names.len() {
            lines.push(format!("- {label}: {summary}"));
            for n in existing {
                rendered.insert(n.to_string());
            }
        } else {
            for n in existing {
                lines.push(format!(
                    "- {n}: {}",
                    tool_summary(n, tool_descriptions, language)
                ));
                rendered.insert(n.to_string());
            }
        }
    }

    for name in ["bash", "code"] {
        if tool_descriptions.contains_key(name)
            && !HIDDEN_TOOLS.contains(&name)
            && !rendered.contains(name)
        {
            lines.push(format!(
                "- {name}: {}",
                tool_summary(name, tool_descriptions, language)
            ));
            rendered.insert(name.to_string());
        }
    }

    if tool_descriptions.contains_key("list_skill") && !rendered.contains("list_skill") {
        lines.push(format!(
            "- list_skill: {}",
            tool_summary("list_skill", tool_descriptions, language)
        ));
        rendered.insert("list_skill".to_string());
    }

    let memory_existing: Vec<&str> = MEMORY_GROUP_NAMES
        .iter()
        .copied()
        .filter(|n| tool_descriptions.contains_key(*n) && !HIDDEN_TOOLS.contains(n))
        .collect();
    if memory_existing.len() == MEMORY_GROUP_NAMES.len() {
        let summary = if cn { MEMORY_GROUP_CN } else { MEMORY_GROUP_EN };
        lines.push(format!("- {MEMORY_GROUP_LABEL}: {summary}"));
        for n in memory_existing {
            rendered.insert(n.to_string());
        }
    } else {
        for n in memory_existing {
            lines.push(format!(
                "- {n}: {}",
                tool_summary(n, tool_descriptions, language)
            ));
            rendered.insert(n.to_string());
        }
    }

    if tool_descriptions.contains_key("task_tool") && !rendered.contains("task_tool") {
        lines.push(format!(
            "- task_tool: {}",
            tool_summary("task_tool", tool_descriptions, language)
        ));
        rendered.insert("task_tool".to_string());
    }

    // 工具调用去重规则
    if cn {
        lines.extend([
            String::new(),
            "## 工具调用去重规则".to_string(),
            String::new(),
            "- 调用工具前先检查本轮对话中是否已经用相同参数调用过同一工具；如果已有结果，优先基于已有结果继续推理，不要重复调用".to_string(),
            "- 如果上一次工具结果为空、无匹配或没有提供新信息，不要用完全相同的参数再次调用；应调整查询条件、换用更合适的工具，或直接说明当前结果不足".to_string(),
            "- 只有当任务确实需要分步执行、状态已经变化、参数不同，或前一次结果明确要求继续获取下一部分信息时，才可以再次调用同一工具".to_string(),
        ]);
    } else {
        lines.extend([
            String::new(),
            "## Tool Call Deduplication Rules".to_string(),
            String::new(),
            "- Before calling a tool, check whether the same tool has already been called with the same ".to_string()
                + "arguments in this turn; if a result already exists, reason from that result instead of "
                + "repeating the call",
            "- If the previous tool result was empty, had no matches, or added no new information, do not ".to_string()
                + "call again with identical arguments; adjust the query, use a better-suited tool, or explain "
                + "that the current result is insufficient",
            "- Call the same tool again only when the task genuinely requires multiple steps, state has ".to_string()
                + "changed, arguments differ, or the previous result clearly asks you to fetch the next part of "
                + "the information",
        ]);
    }

    // bash 使用原则
    if rendered.contains("bash") {
        if cn {
            lines.extend([
                String::new(),
                "## bash 使用原则".to_string(),
                String::new(),
                "- 优先使用专用工具完成文件搜索、内容搜索、读取、编辑和写入，不要用 bash 替代 `glob` / `grep` / ".to_string()
                    + "`read_file` / `edit_file` / `write_file`",
                "- 独立命令尽量并行调用；多步依赖命令才在单次调用里用 `&&` 串联，仅在不关心前序失败时才用 `;`".to_string(),
                "- 长时间运行命令使用 `background: true`，不要用 `sleep` 轮询等待".to_string(),
                "- 尽量使用绝对路径并避免频繁 `cd`；路径包含空格时使用双引号".to_string(),
                "- 执行破坏性 Git 操作前先考虑更安全的替代方案".to_string(),
            ]);
        } else {
            lines.extend([
                String::new(),
                "## bash Guidelines".to_string(),
                String::new(),
                "- Prefer dedicated tools for file search, content search, reading, editing, and writing ".to_string()
                    + "instead of using bash as a substitute for `glob` / `grep` / `read_file` / `edit_file` / "
                    + "`write_file`",
                "- Run independent commands in parallel; only chain dependent commands with `&&`, and ".to_string()
                    + "use `;` only when earlier failures do not matter",
                "- Use `background: true` for long-running commands instead of polling with `sleep`".to_string(),
                "- Prefer absolute paths and avoid frequent `cd`; quote paths with spaces using double quotes".to_string(),
                "- Consider safer alternatives before destructive Git operations".to_string(),
            ]);
        }
    }

    // task_tool 使用原则 + 代理类型行
    if rendered.contains("task_tool") {
        if cn {
            lines.extend([
                String::new(),
                "## task_tool 使用原则".to_string(),
                String::new(),
                "- 任务复杂、多步骤、可独立执行时使用".to_string(),
                "- 独立任务尽量并行执行".to_string(),
                "- 简单任务直接执行，不使用子代理".to_string(),
            ]);
        } else {
            lines.extend([
                String::new(),
                "## task_tool Guidelines".to_string(),
                String::new(),
                "- Use it for complex, multi-step, independent tasks".to_string(),
                "- Run independent tasks in parallel when possible".to_string(),
                "- Execute simple tasks directly without spawning a sub-agent".to_string(),
            ]);
        }
        let agent_lines = crate::sections::context::extract_task_tool_agent_lines(
            tool_descriptions
                .get("task_tool")
                .map(|s| s.as_str())
                .unwrap_or(""),
            language,
        );
        if !agent_lines.is_empty() {
            if cn {
                lines.push(String::new());
                lines.push("可用代理类型：".to_string());
            } else {
                lines.push(String::new());
                lines.push("Available agent types:".to_string());
            }
            lines.extend(agent_lines);
        }
    }

    // 剩余工具(首行描述)
    for (name, desc) in tool_descriptions {
        if rendered.contains(name) || HIDDEN_TOOLS.contains(&name.as_str()) {
            continue;
        }
        let first_line = desc.trim().lines().next().unwrap_or("").trim();
        let overrides = if cn {
            summary_overrides_cn()
        } else {
            summary_overrides_en()
        };
        let summary = overrides.get(name.as_str()).copied().unwrap_or(first_line);
        lines.push(format!("- {name}: {summary}"));
    }

    Some(format!("{}\n", lines.join("\n")))
}

/// 构建 tools section(priority=30;对齐 build_tools_section)。
pub fn build_tools_section(
    tool_descriptions: &BTreeMap<String, String>,
    language: &str,
) -> Option<PromptSection> {
    let content = build_tools_content(tool_descriptions, language)?;
    let mut map = BTreeMap::new();
    map.insert(language.to_string(), content);
    Some(PromptSection::new(section_name::TOOLS, map, 30))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descs() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("bash".to_string(), "Run shell commands".to_string()),
            ("code".to_string(), "Run code".to_string()),
            ("read_file".to_string(), "Read a file".to_string()),
            ("write_file".to_string(), "Write a file".to_string()),
            ("edit_file".to_string(), "Edit a file".to_string()),
            ("cron".to_string(), "Cron manager".to_string()),
            ("cron_list_jobs".to_string(), "hidden".to_string()),
            (
                "custom_tool".to_string(),
                "Custom tool\nsecond line".to_string(),
            ),
        ])
    }

    #[test]
    fn empty_returns_none() {
        assert!(build_tools_content(&BTreeMap::new(), "cn").is_none());
        assert!(build_tools_section(&BTreeMap::new(), "cn").is_none());
    }

    #[test]
    fn preferred_order_and_hidden() {
        let content = build_tools_content(&descs(), "cn").unwrap();
        assert!(content.starts_with("# 可用工具"));
        assert!(content.contains("- cron: 管理定时任务与提醒"));
        assert!(!content.contains("cron_list_jobs"));
    }

    #[test]
    fn file_group_renders_label_when_complete() {
        let content = build_tools_content(&descs(), "cn").unwrap();
        assert!(content.contains("- read_file / write_file / edit_file: 文件读写编辑"));
    }

    #[test]
    fn bash_guidelines_present_when_bash_rendered() {
        let content = build_tools_content(&descs(), "cn").unwrap();
        assert!(content.contains("## bash 使用原则"));
        assert!(content.contains("## 工具调用去重规则"));
    }

    #[test]
    fn remaining_tools_use_first_line() {
        let content = build_tools_content(&descs(), "cn").unwrap();
        assert!(content.contains("- custom_tool: Custom tool"));
        assert!(!content.contains("second line"));
    }

    #[test]
    fn english_content() {
        let content = build_tools_content(&descs(), "en").unwrap();
        assert!(content.starts_with("# Available Tools"));
        assert!(content.contains("## Tool Call Deduplication Rules"));
        assert!(content.contains("## bash Guidelines"));
        assert!(content.contains("- cron: Manage scheduled jobs and reminders"));
    }

    #[test]
    fn tools_section_build() {
        let section = build_tools_section(&descs(), "cn").unwrap();
        assert_eq!(section.name, section_name::TOOLS);
        assert_eq!(section.priority, 30);
        assert!(section.render("cn").contains("可用工具"));
    }
}
