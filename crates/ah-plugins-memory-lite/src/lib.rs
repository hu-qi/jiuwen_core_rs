//! # ah-plugins-memory-lite
//!
//! Real lite-memory primitives (aligned with
//! core/memory/lite/{frontmatter,types,conflict_types}.py):
//! - parse/validate/enrich/rebuild coding-memory frontmatter;
//! - MemoryChunk / WriteMode / WriteResult data models.

use std::collections::BTreeMap;
use std::sync::Arc;

use ah_contracts::keys::MEMORY_LITE;
use ah_contracts::memory_lite::VALID_TYPES;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 解析 frontmatter(对齐 parse_frontmatter;无 frontmatter 返回 None)。
pub fn parse_frontmatter(content: &str) -> Option<BTreeMap<String, String>> {
    let content = content.trim();
    if !content.starts_with("---") {
        return None;
    }
    let end = content[3..].find("---")? + 3;
    let mut result = BTreeMap::new();
    for line in content[3..end].trim().lines() {
        if let Some(colon) = line.find(':') {
            let key = line[..colon].trim().to_string();
            let value = line[colon + 1..].trim().to_string();
            result.insert(key, value);
        }
    }
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

/// 校验 frontmatter(对齐 validate_frontmatter;返回 (ok, message))。
pub fn validate_frontmatter(fm: &BTreeMap<String, String>) -> (bool, String) {
    for field in ["name", "description", "type"] {
        if fm.get(field).map(|v| v.is_empty()).unwrap_or(true) {
            return (false, format!("Missing required field: {field}"));
        }
    }
    if let Some(t) = fm.get("type")
        && !VALID_TYPES.contains(&t.as_str())
    {
        return (false, format!("type must be one of: {:?}", VALID_TYPES));
    }
    (true, String::new())
}

/// 丰富 frontmatter(对齐 enrich_frontmatter;is_edit 控制 created_at 补全)。
/// today 注入以便确定性测试。
pub fn enrich_frontmatter(
    mut fm: BTreeMap<String, String>,
    is_edit: bool,
    today: &str,
) -> BTreeMap<String, String> {
    if !is_edit {
        fm.entry("created_at".to_string())
            .or_insert_with(|| today.to_string());
    }
    fm.insert("updated_at".to_string(), today.to_string());
    fm
}

/// 重建带 frontmatter 的文件内容(对齐 rebuild_content_with_frontmatter)。
pub fn rebuild_content_with_frontmatter(content: &str, fm: &BTreeMap<String, String>) -> String {
    let body = extract_body(content);
    let mut fm_lines = vec!["---".to_string()];
    for (key, value) in fm {
        fm_lines.push(format!("{key}: {value}"));
    }
    fm_lines.push("---".to_string());
    let mut parts = vec![fm_lines.join("\n")];
    if !body.is_empty() {
        parts.push(body);
    }
    parts.join("\n\n")
}

/// 提取 frontmatter 后的正文(对齐 _extract_body)。
pub fn extract_body(content: &str) -> String {
    let content = content.trim();
    if !content.starts_with("---") {
        return content.to_string();
    }
    let Some(end) = content[3..].find("---") else {
        return String::new();
    };
    let end = end + 3;
    content[end + 3..].trim().to_string()
}

/// 映射 tool offset/limit 到 read_file line_range(对齐 _line_range_to_fs_read;
/// 1-based 文件行;-1 表示读到 EOF;offset 为 None 返回 None)。
pub fn line_range_to_fs_read(
    first_line: Option<usize>,
    line_cap: Option<usize>,
) -> Option<(usize, i64)> {
    match first_line {
        None => None,
        Some(fl) => match line_cap {
            Some(cap) => Some((fl, (fl + cap - 1) as i64)),
            None => Some((fl, -1)),
        },
    }
}

/// 行视图切片(对齐 _view_lines;first_line 1-based;返回 (excerpt, total, start_idx, end_idx, truncated))。
pub fn view_lines(
    all_lines: &[String],
    first_line: Option<usize>,
    line_cap: Option<usize>,
) -> (String, usize, usize, usize, bool) {
    let total = all_lines.len();
    let start_idx = match first_line {
        Some(fl) => fl.saturating_sub(1).min(total),
        None => 0,
    };
    let end_idx = match line_cap {
        None => total,
        Some(cap) => (start_idx + cap).min(total),
    };
    let text = all_lines[start_idx..end_idx].join("\n");
    let cut = line_cap.is_some() && end_idx < total;
    (text, total, start_idx, end_idx, cut)
}

/// 向量转二进制 blob(对齐 vector_to_blob;f32 小端,与 Python struct '<f' 对齐)。
pub fn vector_to_blob(embedding: &[f64]) -> Vec<u8> {
    let mut out = Vec::with_capacity(embedding.len() * 4);
    for v in embedding {
        out.extend_from_slice(&(*v as f32).to_le_bytes());
    }
    out
}

/// 二进制 blob 转向量(对齐 blob_to_vector)。
pub fn blob_to_vector(blob: &[u8]) -> Vec<f64> {
    blob.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f64)
        .collect()
}

/// 会话文件名是否为今天或昨天(对齐 _is_recent_session_file;北京时区 +8)。
/// today 注入以便确定性测试。
pub fn is_recent_session_file(filename: &str, today: &str) -> bool {
    // YYYY-MM-DD.md
    if filename.len() != 13 || !filename.ends_with(".md") {
        return false;
    }
    let date_part = &filename[..10];
    // validate date format loosely (digits + dashes)
    let ok =
        date_part.len() == 10 && date_part.as_bytes()[4] == b'-' && date_part.as_bytes()[7] == b'-';
    if !ok {
        return false;
    }
    let Some(today_days) = days_from_civil(today) else {
        return false;
    };
    let Some(file_days) = days_from_civil(date_part) else {
        return false;
    };
    file_days == today_days || file_days == today_days - 1
}

/// 混合检索结果融合重排(对齐 _merge_hybrid_results;加权融合后按分数降序)。
pub fn merge_hybrid_results(
    vector_results: Vec<(String, f64)>,
    keyword_results: Vec<(String, f64)>,
    vector_weight: f64,
    text_weight: f64,
) -> Vec<(String, f64)> {
    use std::collections::HashMap;
    let mut by_id: HashMap<String, (f64, f64)> = HashMap::new();
    for (id, score) in vector_results {
        by_id.entry(id).or_insert((0.0, 0.0)).0 = score;
    }
    for (id, score) in keyword_results {
        let e = by_id.entry(id).or_insert((0.0, 0.0));
        e.1 = score;
    }
    let mut results: Vec<(String, f64)> = by_id
        .into_iter()
        .map(|(id, (v, t))| (id, vector_weight * v + text_weight * t))
        .collect();
    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    results
}

/// 把 YYYY-MM-DD 转为从 1970-01-01 起的天数(非法返回 None)。
fn days_from_civil(date: &str) -> Option<i64> {
    let b = date.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let year: i64 = date[0..4].parse().ok()?;
    let month: i64 = date[5..7].parse().ok()?;
    let day: i64 = date[8..10].parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // Howard Hinnant's days_from_civil
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146097 + doe - 719468)
}

/// 记忆是否启用(对齐 is_memory_enabled;MEMORY_ENABLED env,默认 true)。
pub fn is_memory_enabled() -> bool {
    match std::env::var("MEMORY_ENABLED") {
        Ok(v) => matches!(v.trim().to_lowercase().as_str(), "true" | "1" | "yes"),
        Err(_) => true,
    }
}

/// 前台服务(供插件注册;frontmatter 为纯函数,此处聚合 API)。
pub struct MemoryLiteService;

impl Seam for MemoryLiteService {}

impl MemoryLiteService {
    /// 便捷:解析 + 校验(失败返回 None)。
    pub fn parse_validated(content: &str) -> Option<BTreeMap<String, String>> {
        let fm = parse_frontmatter(content)?;
        let (ok, _) = validate_frontmatter(&fm);
        if ok { Some(fm) } else { None }
    }

    /// 默认记忆配置。
    pub fn default_settings() -> ah_contracts::memory_lite::MemorySettings {
        ah_contracts::memory_lite::MemorySettings::default()
    }
}

/// memory-lite 插件:注册 frontmatter 服务。
pub struct MemoryLitePlugin;

impl Plugin for MemoryLitePlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-memory-lite"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![MEMORY_LITE]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let svc = Arc::new(MemoryLiteService);
        Ok(vec![ctx.register(MEMORY_LITE, svc)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::memory_lite::WriteResult;

    fn fm(map: &[(&str, &str)]) -> BTreeMap<String, String> {
        map.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn parse_returns_none_without_frontmatter() {
        assert!(parse_frontmatter("just body").is_none());
        assert!(parse_frontmatter("  \nplain").is_none());
    }

    #[test]
    fn parse_extracts_key_values() {
        let c = "---\nname: X\ndescription: D\ntype: user\n---\nbody";
        let fm = parse_frontmatter(c).unwrap();
        assert_eq!(fm.get("name").unwrap(), "X");
        assert_eq!(fm.get("type").unwrap(), "user");
        assert_eq!(fm.len(), 3);
    }

    #[test]
    fn parse_handles_missing_end_marker() {
        assert!(parse_frontmatter("---\nname: X").is_none());
    }

    #[test]
    fn validate_requires_fields_and_type() {
        let (ok, msg) = validate_frontmatter(&fm(&[("name", "X")]));
        assert!(!ok && msg.contains("description"));
        let (ok, msg) = validate_frontmatter(&fm(&[
            ("name", "X"),
            ("description", "D"),
            ("type", "bogus"),
        ]));
        assert!(!ok && msg.contains("type"));
        let (ok, _) = validate_frontmatter(&fm(&[
            ("name", "X"),
            ("description", "D"),
            ("type", "reference"),
        ]));
        assert!(ok);
    }

    #[test]
    fn enrich_sets_created_and_updated() {
        let fm1 = enrich_frontmatter(fm(&[]), false, "2026-08-18");
        assert_eq!(fm1.get("created_at").unwrap(), "2026-08-18");
        assert_eq!(fm1.get("updated_at").unwrap(), "2026-08-18");
        let fm2 = enrich_frontmatter(fm(&[("created_at", "2026-01-01")]), true, "2026-08-18");
        assert_eq!(fm2.get("created_at").unwrap(), "2026-01-01");
        assert_eq!(fm2.get("updated_at").unwrap(), "2026-08-18");
    }

    #[test]
    fn rebuild_preserves_body() {
        let content = "---\nname: X\n---\n\nbody text";
        let fm = parse_frontmatter(content).unwrap();
        let rebuilt = rebuild_content_with_frontmatter(content, &fm);
        assert!(rebuilt.contains("name: X"));
        assert!(rebuilt.ends_with("body text"));
        assert!(rebuilt.starts_with("---"));
    }

    #[test]
    fn extract_body_without_frontmatter_returns_all() {
        assert_eq!(extract_body("plain body"), "plain body");
        assert_eq!(extract_body("---\na: b\n---\n\nBODY"), "BODY");
    }

    #[test]
    fn line_range_maps_offset_and_cap() {
        assert_eq!(line_range_to_fs_read(None, Some(10)), None);
        assert_eq!(line_range_to_fs_read(Some(3), None), Some((3, -1)));
        assert_eq!(line_range_to_fs_read(Some(3), Some(5)), Some((3, 7)));
    }

    #[test]
    fn view_lines_slices_and_reports_truncation() {
        let lines: Vec<String> = (1..=10).map(|i| i.to_string()).collect();
        // first_line=2, cap=3 -> lines 2,3,4 (1-based) -> indices 1..4
        let (text, total, start, end, cut) = view_lines(&lines, Some(2), Some(3));
        assert_eq!(total, 10);
        assert_eq!(start, 1);
        assert_eq!(end, 4);
        assert_eq!(text, "2\n3\n4");
        assert!(cut);
        // no cap -> all
        let (_, _, start, end, cut) = view_lines(&lines, None, None);
        assert_eq!(start, 0);
        assert_eq!(end, 10);
        assert!(!cut);
        // cap beyond end -> no truncation
        let (_, _, _, end, cut) = view_lines(&lines, Some(9), Some(5));
        assert_eq!(end, 10);
        assert!(!cut);
    }

    #[test]
    fn vector_blob_roundtrip() {
        let v = vec![1.5, -2.25, 0.0, 3.75];
        let blob = vector_to_blob(&v);
        assert_eq!(blob.len(), 16);
        let back = blob_to_vector(&blob);
        assert_eq!(back.len(), 4);
        for (a, b) in v.iter().zip(back.iter()) {
            assert!((a - b).abs() < 1e-5);
        }
    }

    #[test]
    fn recent_session_file_checks_today_yesterday() {
        assert!(is_recent_session_file("2026-08-18.md", "2026-08-18"));
        assert!(is_recent_session_file("2026-08-17.md", "2026-08-18"));
        assert!(!is_recent_session_file("2026-08-16.md", "2026-08-18"));
        assert!(!is_recent_session_file("notes.md", "2026-08-18"));
        assert!(!is_recent_session_file("2026-13-01.md", "2026-08-18"));
    }

    #[test]
    fn hybrid_merge_reranks_by_weight() {
        let vector = vec![("a".to_string(), 0.9), ("b".to_string(), 0.5)];
        let keyword = vec![("b".to_string(), 0.8), ("c".to_string(), 0.6)];
        let merged = merge_hybrid_results(vector, keyword, 0.7, 0.3);
        // b: 0.7*0.5+0.3*0.8 = 0.59; a: 0.7*0.9 = 0.63; c: 0.3*0.6 = 0.18
        assert_eq!(merged[0].0, "a");
        assert_eq!(merged[1].0, "b");
        assert_eq!(merged[2].0, "c");
        assert!((merged[1].1 - 0.59).abs() < 1e-9);
    }

    #[test]
    fn memory_settings_defaults_match_python() {
        let s = ah_contracts::memory_lite::MemorySettings::default();
        assert_eq!(s.model, "text-embedding-v3");
        assert_eq!(
            s.sources,
            vec!["memory".to_string(), "sessions".to_string()]
        );
        assert_eq!(s.chunking_tokens, 256);
        assert_eq!(s.chunking_overlap, 32);
        assert_eq!(s.query_max_results, 10);
        assert_eq!(s.query_min_score, 0.3);
        assert!(s.hybrid_enabled);
        assert_eq!(s.hybrid_vector_weight, 0.7);
        assert_eq!(s.hybrid_text_weight, 0.3);
        assert_eq!(s.store_path, "memory.db");
        assert_eq!(s.sync_watch_debounce_ms, 2000);
        assert_eq!(s.cache_max_entries, 10000);
    }

    #[test]
    fn memory_settings_with_overrides() {
        use ah_contracts::memory_lite::MemorySettings;
        let base = MemorySettings::default();
        let overridden = base.with_overrides(&[
            ("model", serde_json::json!("custom-model")),
            ("sources", serde_json::json!(["a", "b"])),
            ("unknown_key", serde_json::json!(42)),
        ]);
        assert_eq!(overridden.model, "custom-model");
        assert_eq!(overridden.sources, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(overridden.chunking_tokens, 256); // untouched
    }

    #[test]
    fn write_result_to_dict_omits_defaults() {
        let r = WriteResult::new(
            true,
            "/m/x.md",
            ah_contracts::memory_lite::WriteMode::Create,
        );
        let d = r.to_dict();
        assert_eq!(d["success"], serde_json::json!(true));
        assert_eq!(d["mode"], serde_json::json!("create"));
        assert!(d.get("note").is_none());
        let mut r2 = r.clone();
        r2.conflict_detected = true;
        r2.conflicting_files = vec!["a".to_string()];
        let d2 = r2.to_dict();
        assert_eq!(d2["conflict_detected"], serde_json::json!(true));
        assert_eq!(d2["conflicting_files"], serde_json::json!(["a"]));
    }
}
