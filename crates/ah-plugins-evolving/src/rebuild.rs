//! 经验重建上下文准备(对齐 experience/rebuild.py 的纯逻辑部分)。
//!
//! 纯逻辑:分数/跳过原因过滤 + (score,timestamp) 排序 + 记录数/字符预算裁剪 + 溢出索引;
//! store 加载与归档留待集成。

use serde_json::{Value, json};

/// 重建记录视图(rebuild 所需字段;对齐 EvolutionRecord 子集)。
#[derive(Debug, Clone, PartialEq)]
pub struct RebuildRecord {
    pub id: String,
    pub summary: Option<String>,
    pub score: f64,
    pub timestamp: String,
    pub target: String,
    pub section: String,
    pub content: String,
    pub skip_reason: Option<String>,
}

/// 过滤重建记录(对齐 _filter_rebuild_records:score >= min_score 且无 skip_reason)。
pub fn filter_rebuild_records(records: &[RebuildRecord], min_score: f64) -> Vec<RebuildRecord> {
    records
        .iter()
        .filter(|r| r.score >= min_score && r.skip_reason.is_none())
        .cloned()
        .collect()
}

/// 索引条目(对齐 _to_index_item)。
pub fn to_index_item(record: &RebuildRecord) -> Value {
    json!({
        "record_id": record.id,
        "summary": record.summary.clone().unwrap_or_default(),
        "target": record.target,
        "section": record.section,
        "score": record.score,
        "updated_at": record.timestamp,
    })
}

/// 重建条目(索引 + 裁剪内容;对齐 _to_rebuild_item)。
pub fn to_rebuild_item(record: &RebuildRecord, content: &str) -> Value {
    let mut item = to_index_item(record);
    if let Some(obj) = item.as_object_mut() {
        obj.insert("content".to_string(), Value::String(content.to_string()));
    }
    item
}

/// 构建重建上下文 payload(对齐 _build_rebuild_context_payload)。
pub fn build_rebuild_context_payload(
    records: &[RebuildRecord],
    max_records: usize,
    max_chars: usize,
) -> Value {
    let mut sorted_records = records.to_vec();
    sorted_records.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.timestamp.cmp(&a.timestamp))
    });
    let mut overflow: Vec<RebuildRecord> = if sorted_records.len() > max_records {
        sorted_records[max_records..].to_vec()
    } else {
        Vec::new()
    };
    let included: Vec<RebuildRecord> = sorted_records.iter().take(max_records).cloned().collect();
    let mut items: Vec<Value> = Vec::new();
    let mut used_chars = 0usize;

    for (index, record) in included.iter().enumerate() {
        let remaining = max_chars.saturating_sub(used_chars);
        if remaining == 0 {
            overflow = included[index..].to_vec();
            overflow.extend(sorted_records.iter().skip(max_records).cloned());
            break;
        }
        let clipped: String = record.content.chars().take(remaining).collect();
        used_chars += clipped.chars().count();
        items.push(to_rebuild_item(record, &clipped));
    }

    json!({
        "records": items,
        "overflow_index": {"items": overflow.iter().map(to_index_item).collect::<Vec<_>>()},
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rec(id: &str, score: f64, ts: &str, content: &str, skip: Option<&str>) -> RebuildRecord {
        RebuildRecord {
            id: id.to_string(),
            summary: Some(format!("summary {id}")),
            score,
            timestamp: ts.to_string(),
            target: "body".to_string(),
            section: "Troubleshooting".to_string(),
            content: content.to_string(),
            skip_reason: skip.map(|s| s.to_string()),
        }
    }

    #[test]
    fn filter_by_score_and_skip_reason() {
        let records = vec![
            rec("r1", 0.8, "t1", "c", None),
            rec("r2", 0.4, "t2", "c", None),
            rec("r3", 0.9, "t3", "c", Some("duplicate")),
        ];
        let filtered = filter_rebuild_records(&records, 0.5);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "r1");
    }

    #[test]
    fn payload_sorts_by_score_then_timestamp() {
        let records = vec![
            rec("r1", 0.5, "2026-01-01", "c1", None),
            rec("r2", 0.9, "2026-01-03", "c2", None),
            rec("r3", 0.9, "2026-01-02", "c3", None),
        ];
        let payload = build_rebuild_context_payload(&records, 10, 1000);
        let items = payload["records"].as_array().unwrap();
        assert_eq!(items[0]["record_id"], "r2");
        assert_eq!(items[1]["record_id"], "r3");
        assert_eq!(items[2]["record_id"], "r1");
        assert_eq!(
            payload["overflow_index"]["items"].as_array().unwrap().len(),
            0
        );
    }

    #[test]
    fn record_cap_moves_tail_to_overflow() {
        let records: Vec<RebuildRecord> = (0..5)
            .map(|i| {
                rec(
                    &format!("r{i}"),
                    1.0 - i as f64 * 0.1,
                    &format!("t{i}"),
                    "c",
                    None,
                )
            })
            .collect();
        let payload = build_rebuild_context_payload(&records, 3, 10_000);
        assert_eq!(payload["records"].as_array().unwrap().len(), 3);
        let overflow = payload["overflow_index"]["items"].as_array().unwrap();
        assert_eq!(overflow.len(), 2);
        assert_eq!(overflow[0]["record_id"], "r3");
        assert_eq!(overflow[1]["record_id"], "r4");
    }

    #[test]
    fn char_budget_clips_and_extends_overflow() {
        let records = vec![
            rec("r0", 1.0, "t0", "xxxx", None),
            rec("r1", 0.9, "t1", "yyyy", None),
            rec("r2", 0.8, "t2", "zzzz", None),
        ];
        // 预算 6 字符:r0 4 字符 + r1 裁剪 2 字符;r2 进溢出
        let payload = build_rebuild_context_payload(&records, 10, 6);
        let items = payload["records"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["content"], "xxxx");
        assert_eq!(items[1]["content"], "yy");
        let overflow = payload["overflow_index"]["items"].as_array().unwrap();
        assert_eq!(overflow.len(), 1);
        assert_eq!(overflow[0]["record_id"], "r2");
    }

    #[test]
    fn zero_budget_drops_all_to_overflow() {
        let records = vec![rec("r0", 1.0, "t0", "c", None)];
        let payload = build_rebuild_context_payload(&records, 10, 0);
        assert!(payload["records"].as_array().unwrap().is_empty());
        assert_eq!(
            payload["overflow_index"]["items"].as_array().unwrap().len(),
            1
        );
    }

    #[test]
    fn rebuild_item_includes_content() {
        let item = to_rebuild_item(&rec("r1", 0.8, "t", "content", None), "clipped");
        assert_eq!(item["record_id"], "r1");
        assert_eq!(item["content"], "clipped");
        assert_eq!(item["score"], json!(0.8));
    }
}
