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
