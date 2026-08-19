//! 经验提交服务纯逻辑(对齐 experience/submission.py 的 _build_record_from_evolve_item + 响应信封)。
//!
//! 纯逻辑:归一化 evolve 草稿条目 → 记录(id=ev_{8hex}/UTC ISO 时间戳/score=0.6/append patch)+
//! 提交响应信封(applied↔partial 状态/retry_request_id/record_ids 前 applied 条);store 校验留待集成。

use crate::experience_query::RecordView;
use crate::experience_types::utc_iso_now;
use crate::tool_metadata::unique_hex;
use serde_json::{Map, Value, json};

/// 从归一化 evolve 草稿条目构建的记录(对齐 EvolutionRecord.make + _build_record_from_evolve_item)。
#[derive(Debug, Clone, PartialEq)]
pub struct BuiltEvolveRecord {
    pub id: String,
    pub source: String,
    pub timestamp: String,
    pub context: String,
    pub summary: String,
    pub target: String,
    pub section: String,
    pub content: String,
    pub script_filename: Option<String>,
    pub script_language: Option<String>,
    pub script_purpose: Option<String>,
    pub score: f64,
}

impl BuiltEvolveRecord {
    /// 转为查询视图(供 PendingChange 载荷使用)。
    pub fn to_record_view(&self) -> RecordView {
        RecordView {
            id: self.id.clone(),
            summary: Some(self.summary.clone()),
            score: self.score,
            timestamp: self.timestamp.clone(),
            target: self.target.clone(),
            section: self.section.clone(),
            content: self.content.clone(),
        }
    }
}

fn str_field(item: &Map<String, Value>, key: &str) -> String {
    item.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn opt_str_field(item: &Map<String, Value>, key: &str) -> Option<String> {
    let v = str_field(item, key);
    if v.is_empty() { None } else { Some(v) }
}

/// 构建记录(对齐 _build_record_from_evolve_item;item 为已归一化草稿条目)。
pub fn build_record_from_evolve_item(item: &Value, source: &str) -> BuiltEvolveRecord {
    let obj = item.as_object().cloned().unwrap_or_default();
    let summary = str_field(&obj, "summary");
    BuiltEvolveRecord {
        id: format!("ev_{}", unique_hex().chars().take(8).collect::<String>()),
        source: source.to_string(),
        timestamp: utc_iso_now(),
        context: summary.clone(),
        summary,
        target: str_field(&obj, "target"),
        section: str_field(&obj, "section"),
        content: str_field(&obj, "content"),
        script_filename: opt_str_field(&obj, "script_filename"),
        script_language: opt_str_field(&obj, "script_language"),
        script_purpose: opt_str_field(&obj, "script_purpose"),
        score: 0.6,
    }
}

/// 提交响应信封(对齐 apply_prepared_evolve_submission 的返回组装)。
pub fn build_evolve_submission_response(
    records: &[BuiltEvolveRecord],
    applied_count: usize,
    pending_count: usize,
    errors: &[String],
    request_id: &str,
    subject_payload: &Value,
) -> Value {
    let status = if pending_count == 0 && errors.is_empty() {
        "applied"
    } else {
        "partial"
    };
    let record_ids: Vec<Value> = records
        .iter()
        .take(applied_count)
        .map(|r| Value::String(r.id.clone()))
        .collect();
    json!({
        "success": status == "applied",
        "operation": "evolve",
        "status": status,
        "request_id": request_id,
        "retry_request_id": if status == "partial" { Value::String(request_id.to_string()) } else { Value::Null },
        "subject": subject_payload.clone(),
        "applied_count": applied_count,
        "pending_count": pending_count,
        "record_ids": record_ids,
        "errors": errors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item() -> Value {
        json!({
            "summary": "总结",
            "content": "内容",
            "target": "body",
            "section": "Troubleshooting",
            "reason": "原因",
        })
    }

    #[test]
    fn build_record_shape() {
        let record = build_record_from_evolve_item(&item(), "agent_evolve_tool");
        assert!(record.id.starts_with("ev_"));
        assert_eq!(record.source, "agent_evolve_tool");
        assert_eq!(record.context, "总结");
        assert_eq!(record.summary, "总结");
        assert_eq!(record.target, "body");
        assert_eq!(record.section, "Troubleshooting");
        assert_eq!(record.content, "内容");
        assert_eq!(record.score, 0.6);
        assert!(record.timestamp.contains('T'));
        assert!(record.timestamp.ends_with("+00:00"));
        assert!(record.script_filename.is_none());
    }

    #[test]
    fn build_record_with_script_fields() {
        let mut it = item();
        if let Some(obj) = it.as_object_mut() {
            obj.insert("script_filename".to_string(), json!("x.py"));
            obj.insert("script_language".to_string(), json!("python"));
            obj.insert("script_purpose".to_string(), json!("校验"));
        }
        let record = build_record_from_evolve_item(&it, "src");
        assert_eq!(record.script_filename.as_deref(), Some("x.py"));
        assert_eq!(record.script_language.as_deref(), Some("python"));
        assert_eq!(record.script_purpose.as_deref(), Some("校验"));
        let view = record.to_record_view();
        assert_eq!(view.id, record.id);
        assert_eq!(view.target, "body");
    }

    #[test]
    fn response_applied_envelope() {
        let records = vec![
            build_record_from_evolve_item(&item(), "s"),
            build_record_from_evolve_item(&item(), "s"),
        ];
        let resp = build_evolve_submission_response(
            &records,
            2,
            0,
            &[],
            "req1",
            &json!({"kind": "skill", "name": "sk"}),
        );
        assert_eq!(resp["success"], true);
        assert_eq!(resp["status"], "applied");
        assert_eq!(resp["request_id"], "req1");
        assert!(resp["retry_request_id"].is_null());
        assert_eq!(resp["record_ids"].as_array().unwrap().len(), 2);
        assert_eq!(resp["applied_count"], 2);
        assert_eq!(resp["subject"]["name"], "sk");
    }

    #[test]
    fn response_partial_envelope() {
        let records = vec![build_record_from_evolve_item(&item(), "s")];
        let errors = vec!["store down".to_string()];
        let resp = build_evolve_submission_response(&records, 0, 1, &errors, "req2", &json!({}));
        assert_eq!(resp["success"], false);
        assert_eq!(resp["status"], "partial");
        assert_eq!(resp["retry_request_id"], "req2");
        assert!(resp["record_ids"].as_array().unwrap().is_empty());
        assert_eq!(resp["errors"][0], "store down");
    }
}
