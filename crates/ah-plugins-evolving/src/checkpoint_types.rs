//! 检查点类型(对齐 checkpointing/types.py)。
//!
//! 纯逻辑:UsageStats(展示/使用/正负反馈计数 + 时间戳)+ EvolutionPatch(action/target/section
//! 校验,可选字段序列化)+ EvolutionRecord(make/is_pending/to_dict/from_dict)+ EvolutionLog 容器。

use crate::experience_types::utc_iso_now;
use crate::protocols::{VALID_PATCH_ACTIONS, VALID_SECTIONS};
use crate::tool_metadata::unique_hex;
use serde_json::{Map, Value, json};

/// 经验使用统计(对齐 UsageStats)。
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct UsageStats {
    pub times_presented: u64,
    pub times_used: u64,
    pub times_positive: u64,
    pub times_negative: u64,
    pub last_presented_at: Option<String>,
    pub last_evaluated_at: Option<String>,
}

impl UsageStats {
    /// 序列化(空 Option 省略;对齐 to_dict)。
    pub fn to_dict(&self) -> Value {
        let mut payload = Map::new();
        payload.insert("times_presented".to_string(), json!(self.times_presented));
        payload.insert("times_used".to_string(), json!(self.times_used));
        payload.insert("times_positive".to_string(), json!(self.times_positive));
        payload.insert("times_negative".to_string(), json!(self.times_negative));
        if let Some(v) = &self.last_presented_at {
            payload.insert("last_presented_at".to_string(), json!(v));
        }
        if let Some(v) = &self.last_evaluated_at {
            payload.insert("last_evaluated_at".to_string(), json!(v));
        }
        Value::Object(payload)
    }

    /// 反序列化(对齐 from_dict;缺失字段取默认)。
    pub fn from_dict(data: &Map<String, Value>) -> Self {
        let u64_field = |key: &str| data.get(key).and_then(|v| v.as_u64()).unwrap_or(0);
        let str_field = |key: &str| {
            data.get(key)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        };
        Self {
            times_presented: u64_field("times_presented"),
            times_used: u64_field("times_used"),
            times_positive: u64_field("times_positive"),
            times_negative: u64_field("times_negative"),
            last_presented_at: str_field("last_presented_at"),
            last_evaluated_at: str_field("last_evaluated_at"),
        }
    }
}

/// 一条生成的演进变更(对齐 EvolutionPatch)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct EvolutionPatch {
    pub section: String,
    pub action: String,
    pub content: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_filename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_purpose: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keywords: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

impl EvolutionPatch {
    /// 构造并校验(对齐 __post_init__)。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        section: String,
        action: String,
        content: String,
        target: String,
        skip_reason: Option<String>,
        merge_target: Option<String>,
        script_filename: Option<String>,
        script_language: Option<String>,
        script_purpose: Option<String>,
    ) -> Result<Self, String> {
        if !VALID_PATCH_ACTIONS.contains(&action.as_str()) {
            return Err(format!("invalid evolution patch action: {action}"));
        }
        let target = normalize_target(&target)?;
        if action != "skip" && !VALID_SECTIONS.contains(&section.as_str()) {
            return Err(format!("invalid evolution patch section: {section}"));
        }
        Ok(Self {
            section,
            action,
            content,
            target,
            skip_reason,
            merge_target,
            script_filename,
            script_language,
            script_purpose,
            keywords: None,
            summary: None,
        })
    }

    /// 序列化(可选字段非空才带出;对齐 to_dict)。
    pub fn to_dict(&self) -> Value {
        let mut payload = Map::new();
        payload.insert("section".to_string(), json!(self.section));
        payload.insert("action".to_string(), json!(self.action));
        payload.insert("content".to_string(), json!(self.content));
        payload.insert("target".to_string(), json!(self.target));
        for (key, value) in [
            ("skip_reason", &self.skip_reason),
            ("merge_target", &self.merge_target),
            ("script_filename", &self.script_filename),
            ("script_language", &self.script_language),
            ("script_purpose", &self.script_purpose),
        ] {
            if let Some(v) = value
                && !v.is_empty()
            {
                payload.insert(key.to_string(), json!(v));
            }
        }
        Value::Object(payload)
    }

    /// 反序列化(对齐 from_dict;默认 target=body/section=Troubleshooting/action=append)。
    pub fn from_dict(data: &Map<String, Value>) -> Result<Self, String> {
        let str_field = |key: &str, default: &str| {
            data.get(key)
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(default)
                .to_string()
        };
        let opt_field = |key: &str| {
            data.get(key)
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
        };
        let target_raw = str_field("target", "body");
        let target = normalize_target(&target_raw)?;
        Ok(Self {
            section: str_field("section", "Troubleshooting"),
            action: str_field("action", "append"),
            content: str_field("content", ""),
            target,
            skip_reason: opt_field("skip_reason"),
            merge_target: opt_field("merge_target"),
            script_filename: opt_field("script_filename"),
            script_language: opt_field("script_language"),
            script_purpose: opt_field("script_purpose"),
            keywords: None,
            summary: None,
        })
    }
}

/// 归一化 target 字符串(对齐 EvolutionTarget 枚举值校验)。
fn normalize_target(raw: &str) -> Result<String, String> {
    let target = raw.trim();
    match target {
        "description" | "body" | "script" => Ok(target.to_string()),
        _ => Err(format!("invalid evolution patch target: {target}")),
    }
}

/// 一条已存储演进记录(对齐 EvolutionRecord)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct EvolutionRecord {
    pub id: String,
    pub source: String,
    pub timestamp: String,
    pub context: String,
    pub change: EvolutionPatch,
    #[serde(default)]
    pub applied: bool,
    #[serde(default = "default_score")]
    pub score: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_stats: Option<UsageStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

fn default_score() -> f64 {
    0.6
}

impl EvolutionRecord {
    /// 构造(对齐 make:id=ev_{8hex}/UTC ISO/usage_stats 默认)。
    pub fn make(
        source: &str,
        context: &str,
        change: EvolutionPatch,
        score: f64,
        skill_version: Option<String>,
        summary: Option<String>,
    ) -> Self {
        Self {
            id: format!("ev_{}", unique_hex().chars().take(8).collect::<String>()),
            source: source.to_string(),
            timestamp: utc_iso_now(),
            context: context.to_string(),
            change,
            applied: false,
            score,
            usage_stats: Some(UsageStats::default()),
            skill_version,
            summary,
        }
    }

    /// 待应用(对齐 is_pending)。
    pub fn is_pending(&self) -> bool {
        !self.applied
    }

    /// 序列化(对齐 to_dict)。
    pub fn to_dict(&self) -> Value {
        let mut payload = Map::new();
        payload.insert("id".to_string(), json!(self.id));
        payload.insert("source".to_string(), json!(self.source));
        payload.insert("timestamp".to_string(), json!(self.timestamp));
        payload.insert("context".to_string(), json!(self.context));
        payload.insert("change".to_string(), self.change.to_dict());
        payload.insert("applied".to_string(), json!(self.applied));
        payload.insert("score".to_string(), json!(self.score));
        if let Some(stats) = &self.usage_stats {
            payload.insert("usage_stats".to_string(), stats.to_dict());
        }
        if let Some(v) = &self.skill_version {
            payload.insert("skill_version".to_string(), json!(v));
        }
        if let Some(v) = &self.summary
            && !v.is_empty()
        {
            payload.insert("summary".to_string(), json!(v));
        }
        Value::Object(payload)
    }

    /// 反序列化(对齐 from_dict)。
    pub fn from_dict(data: &Map<String, Value>) -> Result<Self, String> {
        let str_field = |key: &str, default: &str| {
            data.get(key)
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(default)
                .to_string()
        };
        let opt_field = |key: &str| {
            data.get(key)
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
        };
        let usage_stats = match data.get("usage_stats") {
            Some(Value::Object(o)) => Some(UsageStats::from_dict(o)),
            _ => Some(UsageStats::default()),
        };
        let change = data
            .get("change")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        let id_default = format!("ev_{}", unique_hex().chars().take(8).collect::<String>());
        Ok(Self {
            id: str_field("id", &id_default),
            source: str_field("source", "unknown"),
            timestamp: str_field("timestamp", ""),
            context: str_field("context", ""),
            change: EvolutionPatch::from_dict(&change)?,
            applied: data
                .get("applied")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            score: data.get("score").and_then(|v| v.as_f64()).unwrap_or(0.6),
            usage_stats,
            skill_version: opt_field("skill_version"),
            summary: opt_field("summary"),
        })
    }
}

/// 进化日志容器(对齐 EvolutionLog;entries 为 EvolutionRecord)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct EvolutionLog {
    pub skill_id: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub entries: Vec<EvolutionRecord>,
}

fn default_version() -> String {
    "1.0.0".to_string()
}

impl EvolutionLog {
    /// 空日志(对齐 empty)。
    pub fn empty(skill_id: &str) -> Self {
        Self {
            skill_id: skill_id.to_string(),
            version: default_version(),
            updated_at: utc_iso_now(),
            entries: Vec::new(),
        }
    }

    /// pending 条目(对齐 pending_entries)。
    pub fn pending_entries(&self) -> Vec<&EvolutionRecord> {
        self.entries.iter().filter(|e| e.is_pending()).collect()
    }

    /// 序列化(对齐 to_dict)。
    pub fn to_dict(&self) -> Value {
        let entries: Vec<Value> = self.entries.iter().map(|e| e.to_dict()).collect();
        json!({
            "skill_id": self.skill_id,
            "version": self.version,
            "updated_at": self.updated_at,
            "entries": entries,
        })
    }

    /// 反序列化(对齐 from_dict)。
    pub fn from_dict(data: &Map<String, Value>) -> Result<Self, String> {
        let str_field = |key: &str, default: &str| {
            data.get(key)
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(default)
                .to_string()
        };
        let mut entries = Vec::new();
        if let Some(items) = data.get("entries").and_then(|v| v.as_array()) {
            for item in items {
                if let Some(obj) = item.as_object() {
                    entries.push(EvolutionRecord::from_dict(obj)?);
                }
            }
        }
        Ok(Self {
            skill_id: str_field("skill_id", ""),
            version: str_field("version", "1.0.0"),
            updated_at: str_field("updated_at", ""),
            entries,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn patch() -> EvolutionPatch {
        EvolutionPatch::new(
            "Troubleshooting".to_string(),
            "append".to_string(),
            "content".to_string(),
            "body".to_string(),
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap()
    }

    #[test]
    fn patch_validation_errors() {
        assert!(
            EvolutionPatch::new(
                "T".to_string(),
                "bogus".to_string(),
                "c".to_string(),
                "body".to_string(),
                None,
                None,
                None,
                None,
                None
            )
            .is_err()
        );
        assert!(
            EvolutionPatch::new(
                "T".to_string(),
                "append".to_string(),
                "c".to_string(),
                "other".to_string(),
                None,
                None,
                None,
                None,
                None
            )
            .is_err()
        );
        assert!(
            EvolutionPatch::new(
                "NotASection".to_string(),
                "append".to_string(),
                "c".to_string(),
                "body".to_string(),
                None,
                None,
                None,
                None,
                None
            )
            .is_err()
        );
        // skip 动作不校验 section
        assert!(
            EvolutionPatch::new(
                "".to_string(),
                "skip".to_string(),
                "c".to_string(),
                "body".to_string(),
                None,
                None,
                None,
                None,
                None
            )
            .is_ok()
        );
    }

    #[test]
    fn patch_serialization_roundtrip() {
        let p = patch();
        let d = p.to_dict();
        assert_eq!(d["action"], "append");
        assert_eq!(d["target"], "body");
        assert!(!d.as_object().unwrap().contains_key("skip_reason"));
        let parsed = EvolutionPatch::from_dict(d.as_object().unwrap()).unwrap();
        assert_eq!(parsed, p);
        let defaults = EvolutionPatch::from_dict(&Map::new()).unwrap();
        assert_eq!(defaults.target, "body");
        assert_eq!(defaults.section, "Troubleshooting");
        assert_eq!(defaults.action, "append");
    }

    #[test]
    fn record_make_and_pending() {
        let record = EvolutionRecord::make("src", "ctx", patch(), 0.6, None, None);
        assert!(record.id.starts_with("ev_"));
        assert!(record.is_pending());
        assert!(!record.timestamp.is_empty());
        assert_eq!(record.score, 0.6);
        assert!(record.usage_stats.is_some());
    }

    #[test]
    fn record_serialization_roundtrip() {
        let record = EvolutionRecord::make(
            "src",
            "ctx",
            patch(),
            0.8,
            Some("v1".to_string()),
            Some("sum".to_string()),
        );
        let d = record.to_dict();
        assert_eq!(d["id"], record.id);
        assert_eq!(d["source"], "src");
        assert_eq!(d["score"], json!(0.8));
        assert_eq!(d["change"]["target"], "body");
        assert_eq!(d["summary"], "sum");
        let parsed = EvolutionRecord::from_dict(d.as_object().unwrap()).unwrap();
        assert_eq!(parsed, record);
    }

    #[test]
    fn usage_stats_roundtrip_and_defaults() {
        let full = UsageStats {
            times_presented: 3,
            times_used: 2,
            times_positive: 1,
            times_negative: 1,
            last_presented_at: Some("t".to_string()),
            last_evaluated_at: None,
        };
        let d = full.to_dict();
        assert_eq!(d["times_presented"], 3);
        assert!(!d.as_object().unwrap().contains_key("last_evaluated_at"));
        let parsed = UsageStats::from_dict(d.as_object().unwrap());
        assert_eq!(parsed, full);
        assert_eq!(UsageStats::from_dict(&Map::new()), UsageStats::default());
    }

    #[test]
    fn log_empty_serialization_and_pending() {
        let mut log = EvolutionLog::empty("sk");
        assert_eq!(log.version, "1.0.0");
        let mut r1 = EvolutionRecord::make("s", "c", patch(), 0.6, None, None);
        r1.applied = true;
        log.entries = vec![
            r1,
            EvolutionRecord::make("s", "c", patch(), 0.6, None, None),
        ];
        assert_eq!(log.pending_entries().len(), 1);
        let d = log.to_dict();
        assert_eq!(d["entries"].as_array().unwrap().len(), 2);
        let parsed = EvolutionLog::from_dict(d.as_object().unwrap()).unwrap();
        assert_eq!(parsed, log);
        let empty = EvolutionLog::from_dict(&Map::new()).unwrap();
        assert_eq!(empty.skill_id, "");
        assert_eq!(empty.version, "1.0.0");
    }
}
