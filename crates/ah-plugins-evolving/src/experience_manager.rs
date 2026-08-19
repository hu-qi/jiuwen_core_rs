//! 经验管理器纯逻辑(对齐 skill_experience_manager.py 的 build_local_apply_preview 等)。
//!
//! 纯逻辑:ApplyResult 列表 → LocalApplyPreview(跳过未应用/lifecycle_stage 校验/记录合并/
//! change_type 传递)+ 预览 → PendingChange(request_id_prefix 前缀/change_type 覆盖);
//! store/operator 留待集成。

use crate::experience_query::RecordView;
use crate::experience_types::PendingChange;
use crate::lifecycle::LocalApplyPreview;
use crate::protocols::{LOCAL_APPLY_COMPLETED, SKILL_EXPERIENCE_ENTRY};
use crate::tool_metadata::unique_hex;
use serde_json::Value;

/// 从记录 JSON 提取 RecordView(change.target/section/content 嵌套)。
fn record_view_from_json(v: &Value) -> Option<RecordView> {
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
    let target = change
        .get("target")
        .and_then(|s| s.as_str())
        .unwrap_or("body")
        .to_string();
    let section = change
        .get("section")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let content = change
        .get("content")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    Some(RecordView {
        id,
        summary,
        score,
        timestamp,
        target,
        section,
        content,
    })
}

/// 从 ApplyResult 列表构建本地应用预览(对齐 build_local_apply_preview)。
pub fn build_local_apply_preview(
    skill_name: &str,
    apply_results: &[Value],
) -> Result<LocalApplyPreview, String> {
    let mut records: Vec<RecordView> = Vec::new();
    let mut change_type = SKILL_EXPERIENCE_ENTRY.to_string();

    for result in apply_results {
        let Some(obj) = result.as_object() else {
            continue;
        };
        let applied = obj
            .get("applied")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !applied {
            continue;
        }
        if let Some(stage) = obj.get("lifecycle_stage").and_then(|v| v.as_str())
            && !stage.is_empty()
            && stage != LOCAL_APPLY_COMPLETED
        {
            return Err(format!(
                "unsupported apply lifecycle stage for {skill_name}: {stage}"
            ));
        }
        if let Some(items) = obj.get("records").and_then(|v| v.as_array()) {
            for item in items {
                if let Some(record) = record_view_from_json(item) {
                    records.push(record);
                }
            }
        }
        if let Some(ct) = obj.get("change_type").and_then(|v| v.as_str())
            && !ct.is_empty()
        {
            change_type = ct.to_string();
        }
    }

    Ok(LocalApplyPreview::new(
        skill_name,
        records,
        apply_results.to_vec(),
        Some(change_type),
    ))
}

/// 从预览构建待审批快照(对齐 _make_pending_change_from_preview + make_pending_change)。
pub fn make_pending_change_from_preview(
    preview: &LocalApplyPreview,
    request_id_prefix: Option<&str>,
    subject_kind: Option<String>,
    is_shared_records: bool,
) -> PendingChange {
    let mut pending = if is_shared_records {
        PendingChange::make_for_shared_records(
            &preview.skill_name,
            &preview.records,
            subject_kind,
            None,
        )
    } else {
        PendingChange::make(&preview.skill_name, &preview.records, subject_kind, None)
    };
    pending.change_type = preview.change_type.clone();
    if let Some(prefix) = request_id_prefix
        && !prefix.is_empty()
    {
        let hex: String = unique_hex().chars().take(8).collect();
        pending.change_id = format!("{prefix}_{hex}");
    }
    pending
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn applied_result(id: &str, records: Value, change_type: Option<&str>) -> Value {
        let mut result = json!({
            "operator_id": format!("skill_experience_{id}"),
            "target": "experiences",
            "applied": true,
            "records": records,
            "lifecycle_stage": "local_apply_completed",
        });
        if let Some(ct) = change_type {
            result["change_type"] = json!(ct);
        }
        result
    }

    fn record_json(id: &str) -> Value {
        json!({
            "id": format!("ev_{id}"),
            "source": "s",
            "timestamp": "t",
            "context": "c",
            "change": {"target": "body", "section": "Troubleshooting", "content": "content", "action": "append"},
            "applied": false,
            "score": 0.7,
            "summary": format!("sum {id}"),
        })
    }

    #[test]
    fn preview_skips_not_applied_and_collects_records() {
        let results = vec![
            json!({"applied": false}),
            applied_result("a", json!([record_json("r1"), record_json("r2")]), None),
        ];
        let preview = build_local_apply_preview("sk", &results).unwrap();
        assert_eq!(preview.records.len(), 2);
        assert_eq!(preview.records[0].id, "ev_r1");
        assert_eq!(preview.records[0].target, "body");
        assert_eq!(preview.records[0].content, "content");
        assert_eq!(preview.apply_results.len(), 2);
        assert_eq!(preview.change_type, "skill_experience_entry");
    }

    #[test]
    fn preview_takes_last_change_type() {
        let results = vec![applied_result(
            "a",
            json!([record_json("r1")]),
            Some("experience_entry"),
        )];
        let preview = build_local_apply_preview("sk", &results).unwrap();
        assert_eq!(preview.change_type, "experience_entry");
    }

    #[test]
    fn preview_rejects_unsupported_stage() {
        let mut bad = applied_result("a", json!([]), None);
        bad["lifecycle_stage"] = json!("other");
        let err = build_local_apply_preview("sk", &[bad]).unwrap_err();
        assert!(err.contains("unsupported apply lifecycle stage for sk: other"));
    }

    #[test]
    fn pending_from_preview_with_prefix() {
        let preview = build_local_apply_preview(
            "sk",
            &[applied_result("a", json!([record_json("r1")]), None)],
        )
        .unwrap();
        let pending = make_pending_change_from_preview(&preview, Some("agent_evolve"), None, false);
        assert!(pending.change_id.starts_with("agent_evolve_"));
        assert_eq!(pending.change_type, "skill_experience_entry");
        assert_eq!(pending.payload.len(), 1);
        assert!(!pending.is_shared_records);
        let shared = make_pending_change_from_preview(&preview, None, None, true);
        assert!(shared.is_shared_records);
        assert!(!shared.change_id.starts_with("agent_evolve_"));
    }
}
