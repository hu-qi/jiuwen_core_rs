//! 存储投影/渲染纯逻辑(对齐 checkpointing/store_projection.py 的确定性部分)。
//!
//! 纯逻辑:章节文件名/摘要归一/经验索引表/脚本资产表/SKILL.md 描述提取 + pending 文本格式化;
//! 文件渲染与 store 留待集成。

use serde_json::Value;

/// 投影记录视图(渲染所需字段;对齐 EvolutionRecord 子集)。
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectionRecord {
    pub id: String,
    pub summary: Option<String>,
    pub score: f64,
    pub timestamp: String,
    pub target: String,
    pub section: String,
    pub content: String,
    pub script_purpose: Option<String>,
    pub script_language: Option<String>,
    pub script_filename: Option<String>,
}

/// 章节文件名(对齐 _section_filename)。
pub fn section_filename(section: &str) -> String {
    format!("{}.md", section.to_lowercase().replace(' ', "_"))
}

/// 摘要文本归一(对齐 _normalize_summary_text:去标题前缀/| 替换/空白折叠/截断)。
pub fn normalize_summary_text(text: &str, max_chars: usize) -> String {
    let mut value = text.trim().to_string();
    // 去除行首 #{1,6} 标题前缀
    let trimmed_start = value.trim_start();
    let hashes = trimmed_start.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) {
        let after = &trimmed_start[hashes..];
        let after = after.trim_start();
        value = after.to_string();
    }
    value = value.replace('|', " ");
    value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if value.chars().count() > max_chars {
        let clipped: String = value.chars().take(max_chars - 3).collect();
        return format!("{}...", clipped.trim_end());
    }
    value
}

/// 记录摘要(对齐 _record_summary:summary → script_purpose → 内容首行 → id)。
pub fn record_summary(record: &ProjectionRecord) -> String {
    if let Some(s) = &record.summary
        && !s.is_empty()
    {
        return normalize_summary_text(s, 96);
    }
    if record.target == "script"
        && let Some(p) = &record.script_purpose
        && !p.is_empty()
    {
        return normalize_summary_text(p, 96);
    }
    let first_line = record.content.lines().next().unwrap_or("").to_string();
    let normalized = normalize_summary_text(&first_line, 96);
    if normalized.is_empty() {
        record.id.clone()
    } else {
        normalized
    }
}

/// 经验索引表(对齐 _format_experience_index_table)。
pub fn format_experience_index_table(records: &[ProjectionRecord]) -> Vec<String> {
    if records.is_empty() {
        return Vec::new();
    }
    let mut ordered: Vec<&ProjectionRecord> = records.iter().collect();
    ordered.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    ordered.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    ordered.sort_by(|a, b| a.section.cmp(&b.section));
    let mut lines = vec![
        "### Experience Index".to_string(),
        String::new(),
        "| Summary | Type | Score | Detail |".to_string(),
        "|---------|------|-------|--------|".to_string(),
    ];
    for record in ordered {
        let detail_path = format!(
            "evolution/{}#{}",
            section_filename(&record.section),
            record.id
        );
        lines.push(format!(
            "| {} | {} | {:.2} | [{}]({}) |",
            record_summary(record),
            record.section,
            record.score,
            detail_path,
            detail_path,
        ));
    }
    lines.push(String::new());
    lines
}

/// 脚本资产表(对齐 _format_script_assets_table)。
pub fn format_script_assets_table(records: &[ProjectionRecord]) -> Vec<String> {
    if records.is_empty() {
        return Vec::new();
    }
    let mut ordered: Vec<&ProjectionRecord> = records.iter().collect();
    ordered.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    ordered.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut lines = vec![
        "### Script Assets".to_string(),
        String::new(),
        "| Summary | Language | Score | Index | Source |".to_string(),
        "|---------|----------|-------|-------|--------|".to_string(),
    ];
    for record in ordered {
        let filename = record
            .script_filename
            .clone()
            .unwrap_or_else(|| record.id.clone());
        let source = format!("evolution/scripts/{filename}");
        let language = record
            .script_language
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        lines.push(format!(
            "| {} | {} | {:.2} | [evolution/scripts/_index.md](evolution/scripts/_index.md) | [{}]({}) |",
            record_summary(record),
            language,
            record.score,
            source,
            source,
        ));
    }
    lines.push(String::new());
    lines
}

/// 从 SKILL.md 提取 description(对齐 extract_description_from_skill_md)。
pub fn extract_description_from_skill_md(content: &str) -> String {
    if !content.starts_with("---") {
        return String::new();
    }
    let mut parts = content.splitn(3, "---");
    let _ = parts.next();
    let Some(front_matter) = parts.next() else {
        return String::new();
    };
    for line in front_matter.lines() {
        if let Some(rest) = line.strip_prefix("description:") {
            let value = rest.trim().trim_matches('"').trim_matches('\'');
            return value.to_string();
        }
    }
    String::new()
}

/// description pending 文本(对齐 format_desc_experience_text)。
pub fn format_desc_experience_text(pending: &[ProjectionRecord], max_items: usize) -> String {
    if pending.is_empty() {
        return String::new();
    }
    let mut sorted: Vec<&ProjectionRecord> = pending.iter().collect();
    sorted.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    sorted
        .iter()
        .take(max_items)
        .map(|r| format!("- {}", r.content))
        .collect::<Vec<_>>()
        .join("\n")
}

/// body pending 文本(对齐 format_body_experience_text)。
pub fn format_body_experience_text(name: &str, pending: &[ProjectionRecord]) -> String {
    if pending.is_empty() {
        return String::new();
    }
    let mut lines = vec![format!("\n\n# Skill '{name}' body 演进经验\n")];
    for (index, record) in pending.iter().enumerate() {
        lines.push(format!(
            "{}. **[{}]** {}",
            index + 1,
            record.section,
            record.content
        ));
    }
    lines.join("\n")
}

/// 记录 → 投影视图(从 JSON 提取;用于 store 集成点)。
pub fn projection_record_from_json(v: &Value) -> Option<ProjectionRecord> {
    let obj = v.as_object()?;
    let id = obj.get("id")?.as_str()?.to_string();
    let summary = obj
        .get("summary")
        .and_then(|s| s.as_str())
        .map(|s| s.to_string());
    let score = obj.get("score").and_then(|s| s.as_f64()).unwrap_or(0.6);
    let timestamp = obj
        .get("timestamp")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let change = obj.get("change")?.as_object()?;
    let str_f = |k: &str, d: &str| {
        change
            .get(k)
            .and_then(|v| v.as_str())
            .unwrap_or(d)
            .to_string()
    };
    let opt_f = |k: &str| {
        change
            .get(k)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    };
    Some(ProjectionRecord {
        id,
        summary,
        score,
        timestamp,
        target: str_f("target", "body"),
        section: str_f("section", ""),
        content: str_f("content", ""),
        script_purpose: opt_f("script_purpose"),
        script_language: opt_f("script_language"),
        script_filename: opt_f("script_filename"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rec(
        id: &str,
        summary: Option<&str>,
        score: f64,
        ts: &str,
        target: &str,
        section: &str,
        content: &str,
    ) -> ProjectionRecord {
        ProjectionRecord {
            id: id.to_string(),
            summary: summary.map(|s| s.to_string()),
            score,
            timestamp: ts.to_string(),
            target: target.to_string(),
            section: section.to_string(),
            content: content.to_string(),
            script_purpose: None,
            script_language: None,
            script_filename: None,
        }
    }

    #[test]
    fn section_filename_and_summary_normalization() {
        assert_eq!(section_filename("Troubleshooting"), "troubleshooting.md");
        assert_eq!(
            section_filename("Team Collaboration"),
            "team_collaboration.md"
        );
        assert_eq!(
            normalize_summary_text("# 标题 | 内容  \n  折叠", 96),
            "标题 内容 折叠"
        );
        let long = "x".repeat(100);
        let clipped = normalize_summary_text(&long, 96);
        assert!(clipped.ends_with("..."));
        assert!(clipped.chars().count() <= 96);
    }

    #[test]
    fn record_summary_precedence() {
        let with_summary = rec("r1", Some("  summary  "), 0.5, "t", "body", "S", "content");
        assert_eq!(record_summary(&with_summary), "summary");
        let mut script = rec("r2", None, 0.5, "t", "script", "Scripts", "line1\nline2");
        script.script_purpose = Some("purpose".to_string());
        assert_eq!(record_summary(&script), "purpose");
        let plain = rec("r3", None, 0.5, "t", "body", "S", "first line\nsecond");
        assert_eq!(record_summary(&plain), "first line");
        let empty = rec("r4", None, 0.5, "t", "body", "S", "");
        assert_eq!(record_summary(&empty), "r4");
    }

    #[test]
    fn experience_index_table_format() {
        let records = vec![
            rec(
                "r1",
                Some("alpha"),
                0.9,
                "t3",
                "body",
                "Troubleshooting",
                "c",
            ),
            rec("r2", Some("beta"), 0.5, "t1", "body", "Instructions", "c"),
            rec("r3", Some("gamma"), 0.7, "t2", "body", "Instructions", "c"),
        ];
        let lines = format_experience_index_table(&records);
        assert_eq!(lines[0], "### Experience Index");
        // 排序:timestamp 降序 → score 降序 → section 升序(Instructions 在 Troubleshooting 前,稳定排序保持 r3 在 r2 前)
        assert!(lines[4].contains("gamma")); // r3: score 0.7 + Instructions
        assert!(lines[4].contains("0.70"));
        assert!(lines[4].contains("evolution/instructions.md#r3"));
        assert!(lines[6].contains("alpha")); // r1: Troubleshooting 排最后
        assert!(lines[6].contains("evolution/troubleshooting.md#r1"));
        assert_eq!(lines.last().unwrap(), "");
        assert!(format_experience_index_table(&[]).is_empty());
    }

    #[test]
    fn script_assets_table_format() {
        let mut r = rec("r1", Some("sum"), 0.8, "t", "script", "Scripts", "c");
        r.script_filename = Some("x.py".to_string());
        r.script_language = Some("python".to_string());
        let lines = format_script_assets_table(&[r]);
        assert_eq!(lines[0], "### Script Assets");
        assert!(lines[4].contains("python"));
        assert!(lines[4].contains("evolution/scripts/x.py"));
    }

    #[test]
    fn extract_description_from_frontmatter() {
        let content = "---\nname: sk\ndescription: \"desc value\"\n---\nbody";
        assert_eq!(extract_description_from_skill_md(content), "desc value");
        assert_eq!(extract_description_from_skill_md("no frontmatter"), "");
        assert_eq!(extract_description_from_skill_md("---\nname: x\n---"), "");
    }

    #[test]
    fn pending_text_formatters() {
        let records = vec![
            rec("r1", None, 0.5, "t", "body", "Troubleshooting", "内容一"),
            rec(
                "r2",
                None,
                0.9,
                "t",
                "description",
                "Instructions",
                "内容二",
            ),
        ];
        let desc = format_desc_experience_text(&records, 5);
        // 按分数降序
        assert!(desc.starts_with("- 内容二"));
        let body = format_body_experience_text("sk", &records);
        assert!(body.contains("# Skill 'sk' body 演进经验"));
        assert!(body.contains("1. **[Troubleshooting]** 内容一"));
    }

    #[test]
    fn projection_record_from_json_test() {
        let v = json!({
            "id": "ev_x",
            "summary": "s",
            "score": 0.7,
            "timestamp": "t",
            "change": {"target": "script", "section": "Scripts", "content": "c", "script_purpose": "p", "script_language": "python", "script_filename": "x.py"},
        });
        let r = projection_record_from_json(&v).unwrap();
        assert_eq!(r.target, "script");
        assert_eq!(r.script_purpose.as_deref(), Some("p"));
        assert_eq!(r.script_filename.as_deref(), Some("x.py"));
    }
}
