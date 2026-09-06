//! 经验索引只读查询(对齐 experience/query.py 的纯逻辑部分)。
//!
//! 纯逻辑:字面量 OR 查询词拆分 + 目标/章节过滤 + 排序 + 游标分页 + 索引/读取条目映射;
//! store 加载与 subject 归一化留待集成(调用方传入已加载记录视图)。

use serde_json::{Value, json};

/// 记录视图(查询所需字段;对齐 EvolutionRecord 的相关子集)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RecordView {
    pub id: String,
    pub summary: Option<String>,
    pub score: f64,
    pub timestamp: String,
    pub target: String,
    pub section: String,
    pub content: String,
}

/// 拆分字面量 OR 查询表达式(对齐 split_experience_index_query_terms)。
pub fn split_experience_index_query_terms(query: Option<&str>) -> Vec<String> {
    let mut terms = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for raw in query.unwrap_or("").to_lowercase().split('|') {
        let term = raw.trim();
        if term.is_empty() || seen.contains(term) {
            continue;
        }
        seen.insert(term.to_string());
        terms.push(term.to_string());
    }
    terms
}

/// 过滤记录(对齐 filter_experience_index_records:target/section/query + 排序)。
pub fn filter_experience_index_records(
    records: &[RecordView],
    target: Option<&str>,
    section: Option<&str>,
    query: Option<&str>,
    sort: &str,
) -> Result<Vec<RecordView>, String> {
    let query_terms = split_experience_index_query_terms(query);
    let mut filtered: Vec<RecordView> = Vec::new();
    for record in records {
        if let Some(t) = target
            && record.target != t
        {
            continue;
        }
        if let Some(s) = section
            && record.section != s
        {
            continue;
        }
        if !query_terms.is_empty() {
            let haystack = format!(
                "{} {} {} {}",
                record.summary.as_deref().unwrap_or(""),
                record.id,
                record.target,
                record.section,
            )
            .to_lowercase();
            if !query_terms.iter().any(|term| haystack.contains(term)) {
                continue;
            }
        }
        filtered.push(record.clone());
    }

    match sort {
        "updated_desc" => {
            filtered.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
            Ok(filtered)
        }
        "score_desc" => {
            filtered.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| b.timestamp.cmp(&a.timestamp))
            });
            Ok(filtered)
        }
        _ => Err("sort must be score_desc or updated_desc".to_string()),
    }
}

/// 游标分页(对齐 _paginate_experience_index)。
pub fn paginate_experience_index(
    records: &[RecordView],
    cursor: Option<&str>,
    limit: usize,
) -> (Vec<RecordView>, Option<String>) {
    let start = cursor.and_then(|c| c.parse::<usize>().ok()).unwrap_or(0);
    let end = start + limit;
    let page = records
        .iter()
        .skip(start)
        .take(limit)
        .cloned()
        .collect::<Vec<_>>();
    let next_cursor = if end < records.len() {
        Some(end.to_string())
    } else {
        None
    };
    (page, next_cursor)
}

/// 索引条目(对齐 _to_index_item)。
pub fn to_index_item(record: &RecordView) -> Value {
    json!({
        "record_id": record.id,
        "summary": record.summary.clone().unwrap_or_default(),
        "target": record.target,
        "section": record.section,
        "score": record.score,
        "updated_at": record.timestamp,
    })
}

/// 读取条目(对齐 _to_read_item:内容截断 + 截断标志)。
pub fn to_read_item(record: &RecordView, max_content_chars: usize) -> Value {
    let content: String = record.content.chars().take(max_content_chars).collect();
    let truncated = record.content.chars().count() > max_content_chars;
    let mut item = to_index_item(record);
    if let Some(obj) = item.as_object_mut() {
        obj.insert("content".to_string(), Value::String(content));
        obj.insert("content_truncated".to_string(), Value::Bool(truncated));
    }
    item
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rec(id: &str, target: &str, section: &str, score: f64, ts: &str) -> RecordView {
        RecordView {
            id: id.to_string(),
            summary: Some(format!("summary {id}")),
            score,
            timestamp: ts.to_string(),
            target: target.to_string(),
            section: section.to_string(),
            content: format!("content {id}"),
        }
    }

    #[test]
    fn query_terms_split() {
        assert_eq!(
            split_experience_index_query_terms(Some("Foo | bar | foo |  | baz")),
            vec!["foo", "bar", "baz"],
        );
        assert!(split_experience_index_query_terms(None).is_empty());
        assert!(split_experience_index_query_terms(Some("  ")).is_empty());
    }

    #[test]
    fn filter_by_target_and_section() {
        let records = vec![
            rec("r1", "body", "Troubleshooting", 0.8, "2026-01-01"),
            rec("r2", "description", "Instructions", 0.9, "2026-01-02"),
        ];
        let filtered =
            filter_experience_index_records(&records, Some("body"), None, None, "score_desc")
                .unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "r1");
        let s = filter_experience_index_records(
            &records,
            None,
            Some("Instructions"),
            None,
            "score_desc",
        )
        .unwrap();
        assert_eq!(s[0].id, "r2");
    }

    #[test]
    fn filter_by_query_any_term() {
        let records = vec![
            rec("r1", "body", "Troubleshooting", 0.8, "t1"),
            rec("r2", "script", "Scripts", 0.5, "t2"),
        ];
        let filtered = filter_experience_index_records(
            &records,
            None,
            None,
            Some("R1 | scripts"),
            "score_desc",
        )
        .unwrap();
        assert_eq!(filtered.len(), 2);
        let none = filter_experience_index_records(&records, None, None, Some("zzz"), "score_desc")
            .unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn sort_orders() {
        let records = vec![
            rec("r1", "body", "Troubleshooting", 0.8, "2026-01-01"),
            rec("r2", "body", "Troubleshooting", 0.8, "2026-01-03"),
            rec("r3", "body", "Troubleshooting", 0.9, "2026-01-02"),
        ];
        let score =
            filter_experience_index_records(&records, None, None, None, "score_desc").unwrap();
        assert_eq!(
            score.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["r3", "r2", "r1"]
        );
        let updated =
            filter_experience_index_records(&records, None, None, None, "updated_desc").unwrap();
        assert_eq!(
            updated.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["r2", "r3", "r1"]
        );
        let err = filter_experience_index_records(&records, None, None, None, "bogus").unwrap_err();
        assert_eq!(err, "sort must be score_desc or updated_desc");
    }

    #[test]
    fn pagination_with_cursor() {
        let records: Vec<RecordView> = (0..5)
            .map(|i| rec(&format!("r{i}"), "body", "S", 0.5, "t"))
            .collect();
        let (page1, next) = paginate_experience_index(&records, None, 2);
        assert_eq!(page1.len(), 2);
        assert_eq!(next.as_deref(), Some("2"));
        let (page2, next2) = paginate_experience_index(&records, next.as_deref(), 2);
        assert_eq!(page2.len(), 2);
        assert_eq!(next2.as_deref(), Some("4"));
        let (page3, next3) = paginate_experience_index(&records, next2.as_deref(), 2);
        assert_eq!(page3.len(), 1);
        assert_eq!(next3, None);
    }

    #[test]
    fn index_item_shape() {
        let item = to_index_item(&rec("r1", "body", "Troubleshooting", 0.75, "ts"));
        assert_eq!(item["record_id"], "r1");
        assert_eq!(item["target"], "body");
        assert_eq!(item["section"], "Troubleshooting");
        assert_eq!(item["score"], json!(0.75));
        assert_eq!(item["updated_at"], "ts");
    }

    #[test]
    fn read_item_truncation() {
        let mut r = rec("r1", "body", "S", 0.5, "t");
        r.content = "x".repeat(100);
        let item = to_read_item(&r, 30);
        assert_eq!(item["content"].as_str().unwrap().chars().count(), 30);
        assert_eq!(item["content_truncated"], json!(true));
        let short = to_read_item(&r, 200);
        assert_eq!(short["content_truncated"], json!(false));
    }
}
