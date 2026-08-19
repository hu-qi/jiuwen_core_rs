//! 检查点类型(对齐 checkpointing/types.py 的 UsageStats/EvolutionLog 纯数据部分)。
//!
//! 纯逻辑:UsageStats(展示/使用/正负反馈计数 + 时间戳,to_dict 省略空 Option)+
//! EvolutionLog 容器(skill_id/version/updated_at/entries,pending 过滤,to_dict/from_dict/empty);
//! 记录以 JSON 承载(entries),is_pending = applied == false。

use crate::experience_types::utc_iso_now;
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

/// 进化日志容器(对齐 EvolutionLog;entries 为记录 JSON)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct EvolutionLog {
    pub skill_id: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub entries: Vec<Value>,
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

    /// pending 条目(applied == false;对齐 pending_entries)。
    pub fn pending_entries(&self) -> Vec<&Value> {
        self.entries
            .iter()
            .filter(|e| !e.get("applied").and_then(|v| v.as_bool()).unwrap_or(false))
            .collect()
    }

    /// 序列化(对齐 to_dict)。
    pub fn to_dict(&self) -> Value {
        json!({
            "skill_id": self.skill_id,
            "version": self.version,
            "updated_at": self.updated_at,
            "entries": self.entries,
        })
    }

    /// 反序列化(对齐 from_dict;缺失字段取默认)。
    pub fn from_dict(data: &Map<String, Value>) -> Self {
        let str_field = |key: &str| {
            data.get(key)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        let entries = data
            .get("entries")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        Self {
            skill_id: str_field("skill_id"),
            version: {
                let v = str_field("version");
                if v.is_empty() { default_version() } else { v }
            },
            updated_at: str_field("updated_at"),
            entries,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn usage_stats_roundtrip_and_defaults() {
        let stats = UsageStats::default();
        let d = stats.to_dict();
        assert_eq!(d["times_presented"], 0);
        assert!(!d.as_object().unwrap().contains_key("last_presented_at"));
        let full = UsageStats {
            times_presented: 3,
            times_used: 2,
            times_positive: 1,
            times_negative: 1,
            last_presented_at: Some("t".to_string()),
            last_evaluated_at: None,
        };
        let d2 = full.to_dict();
        assert_eq!(d2["times_presented"], 3);
        assert_eq!(d2["last_presented_at"], "t");
        let parsed = UsageStats::from_dict(d2.as_object().unwrap());
        assert_eq!(parsed.times_used, 2);
        assert_eq!(parsed.last_presented_at.as_deref(), Some("t"));
        let partial = UsageStats::from_dict(&Map::new());
        assert_eq!(partial, UsageStats::default());
    }

    #[test]
    fn evolution_log_empty_and_serialization() {
        let log = EvolutionLog::empty("sk");
        assert_eq!(log.skill_id, "sk");
        assert_eq!(log.version, "1.0.0");
        assert!(log.entries.is_empty());
        assert!(!log.updated_at.is_empty());
        let d = log.to_dict();
        assert_eq!(d["skill_id"], "sk");
        let parsed = EvolutionLog::from_dict(d.as_object().unwrap());
        assert_eq!(parsed, log);
    }

    #[test]
    fn pending_entries_filter() {
        let mut log = EvolutionLog::empty("sk");
        log.entries = vec![
            json!({"id": "r1", "applied": false}),
            json!({"id": "r2", "applied": true}),
            json!({"id": "r3"}),
        ];
        let pending = log.pending_entries();
        assert_eq!(pending.len(), 2); // r1 与 r3(applied 缺失视为 false)
        assert_eq!(pending[0]["id"], "r1");
    }

    #[test]
    fn from_dict_defaults() {
        let parsed = EvolutionLog::from_dict(&Map::new());
        assert_eq!(parsed.skill_id, "");
        assert_eq!(parsed.version, "1.0.0");
        assert!(parsed.entries.is_empty());
    }
}
