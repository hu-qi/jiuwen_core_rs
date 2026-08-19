//! Context 动态 section(对齐 harness/prompts/sections/context.py)。
//!
//! 纯逻辑:模板检测/agent 名字清洗/身份已填判断 + 上下文内容组装 + 单文件 section;
//! 文件读取与缓存(sys_operation)留待集成,组装接受已解析内容。

use ah_contracts::prompt_builder::{PromptSection, section_name};
use std::collections::BTreeMap;

pub const CONTEXT_HEADER_CN: &str = "# 项目上下文\n\n以下文件已加载到上下文中，无需再次读取。\n\n";
pub const CONTEXT_HEADER_EN: &str = concat!(
    "# Project Context\n\nThe following files are already loaded into context, so you do not need to ",
    "read them again.\n\n",
);

pub fn context_file_titles_cn() -> BTreeMap<&'static str, &'static str> {
    BTreeMap::from([
        ("AGENT.md", "## AGENT.md - 智能体配置"),
        ("SOUL.md", "## SOUL.md - 灵魂与价值观"),
        ("HEARTBEAT.md", "## HEARTBEAT.md - 心跳任务"),
        ("USER.md", "## USER.md - 用户信息"),
        ("IDENTITY.md", "## IDENTITY.md - 身份凭证"),
        ("MEMORY.md", "## MEMORY.md - 长期记忆"),
    ])
}

pub fn context_file_titles_en() -> BTreeMap<&'static str, &'static str> {
    BTreeMap::from([
        ("AGENT.md", "## AGENT.md - Agent Configuration"),
        ("SOUL.md", "## SOUL.md - Soul & Values"),
        ("HEARTBEAT.md", "## HEARTBEAT.md - Heartbeat Tasks"),
        ("USER.md", "## USER.md - User Information"),
        ("IDENTITY.md", "## IDENTITY.md - Identity Credentials"),
        ("MEMORY.md", "## MEMORY.md - Long-term Memory"),
    ])
}

/// 固定上下文文件(对齐 CONTEXT_FILES)。
pub const CONTEXT_FILES: [&str; 5] = [
    "AGENT.md",
    "SOUL.md",
    "HEARTBEAT.md",
    "USER.md",
    "IDENTITY.md",
];

/// 单文件 section 名(对齐 CONTEXT_SECTION_BY_FILE)。
pub fn context_section_by_file(file_key: &str) -> Option<&'static str> {
    match file_key {
        "AGENT.md" => Some("context.agent"),
        "SOUL.md" => Some("context.soul"),
        "HEARTBEAT.md" => Some("context.heartbeat"),
        "USER.md" => Some("context.user"),
        "IDENTITY.md" => Some("context.identity"),
        _ => None,
    }
}

pub const DAILY_MEMORY_GUIDANCE_CN: &str = concat!(
    "每日记忆不会自动注入系统提示词。涉及今天、昨天、之前、继续、上次、记忆、偏好、历史",
    "等上下文时，先调用 `read_memory` 读取 `memory/daily_memory/YYYY-MM-DD.md`，",
    "或使用 `memory_search` 检索相关记忆。\n\n",
);
pub const DAILY_MEMORY_GUIDANCE_EN: &str = concat!(
    "Daily memory is not automatically injected into the system prompt. When context involves ",
    "today, yesterday, earlier, continue, last time, memory, preferences, history, or similar ",
    "historical context, first call `read_memory` to read ",
    "`memory/daily_memory/YYYY-MM-DD.md`, or use `memory_search` to retrieve relevant memories.\n\n",
);

/// 未填充模板标记短语(对齐 _TEMPLATE_MARKERS)。
pub const TEMPLATE_MARKERS: [&str; 6] = [
    "此处应保存的内容",
    "What should be saved here",
    "在你们的第一次对话中填写",
    "Fill this in during your first",
    "在这里添加你需要",
    "Add your periodic tasks here",
];

/// 去除 HTML 注释(<!-- ... -->;对齐 re.sub(r'<!--.*?-->'))。
fn strip_html_comments(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut rest = content;
    loop {
        match rest.find("<!--") {
            Some(start) => {
                out.push_str(&rest[..start]);
                let after = &rest[start + 4..];
                match after.find("-->") {
                    Some(end) => rest = &after[end + 3..],
                    None => {
                        out.push_str(after);
                        break;
                    }
                }
            }
            None => {
                out.push_str(rest);
                break;
            }
        }
    }
    out
}

/// 去除 Markdown 标题行(^#{1,6}\s+.*$;对齐 re.sub(MULTILINE))。
fn strip_markdown_headings(text: &str) -> String {
    text.lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            let hashes = trimmed.chars().take_while(|c| *c == '#').count();
            !(1..=6).contains(&hashes)
                || !trimmed[hashes..]
                    .chars()
                    .next()
                    .map(|c| c.is_whitespace())
                    .unwrap_or(false)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 未填充模板检测(对齐 _is_unfilled_template:长度/注释/标记/标题 规则)。
pub fn is_unfilled_template(content: &str, max_template_len: usize) -> bool {
    if content.chars().count() > max_template_len {
        return false;
    }
    let text = strip_html_comments(content);
    if text.trim().is_empty() {
        return true;
    }
    if TEMPLATE_MARKERS.iter().any(|m| content.contains(m)) {
        return true;
    }
    let no_headings = strip_markdown_headings(&text);
    no_headings.trim().is_empty()
}

/// Agent 名字清洗(对齐 _clean_agent_name:去权威/见 IDENTITY.md 后缀 + 引号标点)。
pub fn clean_agent_name(raw_name: &str) -> String {
    let mut name = raw_name.trim().to_string();
    // 去除尾随 (…权威…)/(…见 IDENTITY.md…) 括号后缀
    if let Some(end) = find_trailing_bracket(&name) {
        let inner = &name[end..];
        let inner_trimmed = inner.trim_end_matches(['）', ')']).trim_start();
        let has_authority = inner_trimmed.contains("权威");
        let has_identity = inner_trimmed.contains("IDENTITY.md")
            || inner_trimmed.contains("IDENTITY") && inner_trimmed.contains("见");
        if has_authority || has_identity {
            name.truncate(end);
            name = name.trim_end().to_string();
        }
    }
    name.trim_matches([
        '`', '\"', '\'', '“', '”', '‘', '’', '。', '；', ';', '，', ',',
    ])
    .to_string()
}

/// 从末尾找括号起点:返回开括号字符索引(括号内容必须延伸到字符串末尾)。
pub fn find_trailing_bracket(s: &str) -> Option<usize> {
    let chars: Vec<char> = s.chars().collect();
    let mut i = chars.len();
    while i > 0 {
        let c = chars[i - 1];
        if c == ')' || c == '）' {
            let mut depth = 1i32;
            let mut j = i - 1;
            while j > 0 {
                let d = chars[j - 1];
                if d == '(' || d == '（' {
                    depth -= 1;
                    if depth == 0 {
                        return Some(chars[..j - 1].iter().map(|ch| ch.len_utf8()).sum());
                    }
                } else if d == ')' || d == '）' {
                    depth += 1;
                }
                j -= 1;
            }
            return None;
        }
        i -= 1;
    }
    None
}

pub fn identity_has_filled_name(content: &str) -> bool {
    for line in content.lines() {
        let trimmed = line.trim_start();
        let mut rest = trimmed;
        // 可选 - 或 * 前缀
        if let Some(after) = rest.strip_prefix(['-', '*']) {
            rest = after.trim_start();
        }
        // 可选 ** 加粗前缀
        if let Some(after) = rest.strip_prefix("**") {
            rest = after;
        }
        // 名字 或 Name 前缀
        let name_val = if let Some(after) = rest.strip_prefix("名字") {
            after
        } else if let Some(after) = rest.strip_prefix("Name") {
            after
        } else {
            continue;
        };
        // 分隔符 ：或 :
        let val = if let Some(after) = name_val.strip_prefix(['：', ':']) {
            after
        } else {
            continue;
        };
        let val = val.trim_start();
        let val = val.strip_prefix("**").unwrap_or(val);
        let cleaned = clean_agent_name(val);
        if !cleaned.is_empty() && !cleaned.starts_with("_(") {
            return true;
        }
    }
    false
}

/// 上下文内容组装(对齐 _build_context_content;files 为已解析内容,None 跳过)。
pub fn build_context_content(
    files: &[(&str, Option<String>)],
    language: &str,
    include_daily_memory: bool,
    extra_content: Option<&str>,
) -> String {
    let header = if language == "en" {
        CONTEXT_HEADER_EN
    } else {
        CONTEXT_HEADER_CN
    };
    let titles = if language == "en" {
        context_file_titles_en()
    } else {
        context_file_titles_cn()
    };
    let mut parts = vec![header.to_string()];

    for (file_key, content) in files {
        let Some(content) = content else { continue };
        let title = titles
            .get(file_key)
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("## {file_key}"));
        parts.push(format!("{title}\n\n{content}\n\n"));
    }

    if language == "cn" {
        parts.push("[以下文件仅在有实际内容时注入，空文件跳过]\n\n".to_string());
    } else {
        parts.push(
            "[The following files are injected only when they contain real content; ".to_string()
                + "empty files are skipped]\n\n",
        );
    }

    if include_daily_memory {
        parts.push(if language == "en" {
            DAILY_MEMORY_GUIDANCE_EN.to_string()
        } else {
            DAILY_MEMORY_GUIDANCE_CN.to_string()
        });
    }

    if let Some(extra) = extra_content {
        parts.push(extra.to_string());
    }

    parts.concat()
}

/// 构建 context section(priority=80;对齐 build_context_section)。
pub fn build_context_section(
    files: &[(&str, Option<String>)],
    language: &str,
    tools_content: Option<&str>,
    include_daily_memory: bool,
) -> PromptSection {
    let content = build_context_content(files, language, include_daily_memory, tools_content);
    let mut map = BTreeMap::new();
    map.insert(language.to_string(), content);
    PromptSection::new(section_name::CONTEXT, map, 80)
}

/// 单文件 section 映射(对齐 build_context_file_sections)。
pub fn build_context_file_sections(
    files: &[(&str, Option<String>)],
    language: &str,
) -> BTreeMap<String, PromptSection> {
    let titles = if language == "en" {
        context_file_titles_en()
    } else {
        context_file_titles_cn()
    };
    let mut sections = BTreeMap::new();
    for (file_key, content) in files {
        let Some(section_name) = context_section_by_file(file_key) else {
            continue;
        };
        let Some(content) = content else { continue };
        let title = titles
            .get(file_key)
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("## {file_key}"));
        let mut map = BTreeMap::new();
        map.insert(language.to_string(), format!("{title}\n\n{content}\n"));
        sections.insert(
            section_name.to_string(),
            PromptSection::new(section_name, map, 80),
        );
    }
    sections
}

/// 从 task_tool 描述提取子代理行(对齐 _extract_task_tool_agent_lines)。
pub fn extract_task_tool_agent_lines(description: &str, language: &str) -> Vec<String> {
    if description.is_empty() {
        return Vec::new();
    }
    let (marker, stop_marker) = if language == "en" {
        (
            "Available agent types and the tools they have access to:",
            "Important:",
        )
    } else {
        ("可用代理类型及对应工具：", "重要：")
    };
    let Some(idx) = description.find(marker) else {
        return Vec::new();
    };
    let mut body = &description[idx + marker.len()..];
    if let Some(stop) = body.find(stop_marker) {
        body = &body[..stop];
    }
    let mut lines = Vec::new();
    for raw in body.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("- ") {
            lines.push(line.to_string());
        } else {
            lines.push(format!("- {line}"));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unfilled_template_detection() {
        assert!(is_unfilled_template("<!-- 注释 -->", 500));
        assert!(is_unfilled_template("# 标题\n## 二级", 500));
        assert!(is_unfilled_template("此处应保存的内容...", 500));
        assert!(!is_unfilled_template("# 标题\n真实内容。", 500));
        assert!(!is_unfilled_template("x".repeat(600).as_str(), 500));
        assert!(!is_unfilled_template("AGENT.md 配置内容 123", 500));
    }

    #[test]
    fn clean_agent_name_strips_suffix_and_quotes() {
        assert_eq!(clean_agent_name("小明（权威）"), "小明");
        assert_eq!(clean_agent_name("Alice (see IDENTITY.md)"), "Alice");
        assert_eq!(clean_agent_name("\"Bob\""), "Bob");
        assert_eq!(clean_agent_name("  Charlie  "), "Charlie");
        assert_eq!(clean_agent_name("Alice (说明)"), "Alice (说明)");
    }

    #[test]
    fn identity_filled_name_detection() {
        assert!(identity_has_filled_name("名字：小明\n其他内容"));
        assert!(identity_has_filled_name("- Name: Alice\n"));
        assert!(!identity_has_filled_name("名字：_(待填写)_\n"));
        assert!(!identity_has_filled_name("# 只有标题\n"));
    }

    #[test]
    fn context_content_assembly() {
        let files = vec![
            ("AGENT.md", Some("agent 配置内容".to_string())),
            ("SOUL.md", None),
        ];
        let cn = build_context_content(&files, "cn", true, None);
        assert!(cn.starts_with("# 项目上下文"));
        assert!(cn.contains("## AGENT.md - 智能体配置\n\nagent 配置内容\n\n"));
        assert!(!cn.contains("SOUL.md"));
        assert!(cn.contains("每日记忆不会自动注入"));
        let en = build_context_content(&files, "en", false, None);
        assert!(en.starts_with("# Project Context"));
        assert!(!en.contains("每日记忆"));
    }

    #[test]
    fn context_section_priority() {
        let files = vec![("AGENT.md", Some("内容".to_string()))];
        let section = build_context_section(&files, "cn", None, true);
        assert_eq!(section.name, section_name::CONTEXT);
        assert_eq!(section.priority, 80);
        assert!(section.render("cn").contains("内容"));
    }

    #[test]
    fn per_file_sections() {
        let files = vec![
            ("AGENT.md", Some("a".to_string())),
            ("USER.md", Some("u".to_string())),
            ("MEMORY.md", Some("m".to_string())),
        ];
        let sections = build_context_file_sections(&files, "cn");
        assert_eq!(sections.len(), 2);
        assert!(sections.contains_key("context.agent"));
        assert!(sections.contains_key("context.user"));
        assert!(!sections.contains_key("context.memory"));
    }

    #[test]
    fn task_tool_agent_lines_extraction() {
        let desc_cn = "... 可用代理类型及对应工具：\nagent_a\n- agent_b\n\n重要：其他";
        let lines = extract_task_tool_agent_lines(desc_cn, "cn");
        assert_eq!(lines, vec!["- agent_a", "- agent_b"]);
        assert!(extract_task_tool_agent_lines("无标记", "cn").is_empty());
        let desc_en = "... Available agent types and the tools they have access to:\nagent_x\n\nImportant: note";
        assert_eq!(
            extract_task_tool_agent_lines(desc_en, "en"),
            vec!["- agent_x"],
        );
    }
}
