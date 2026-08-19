//! 在线经验生命周期(对齐 experience/lifecycle.py 类型 + common.py 的提交/精简计数)。
//!
//! 纯逻辑:LocalApplyPreview/PendingCommitResult 类型 + 按记录逐条提交(部分应用保留尾部重试)+
//! simplify 动作执行计数(store 经闭包注入,错误计入 errors)。

use crate::experience_query::RecordView;
use crate::experience_types::PendingChange;
use crate::protocols::{EXPERIENCE_ENTRY, LOCAL_APPLY_COMPLETED, SKILL_EXPERIENCE_ENTRY};
use serde_json::Value;
use std::collections::BTreeMap;

/// 本地应用预览契约(对齐 LocalApplyPreview)。
#[derive(Debug, Clone, PartialEq)]
pub struct LocalApplyPreview {
    pub skill_name: String,
    pub records: Vec<RecordView>,
    /// ApplyResult 列表(以 JSON 承载,对齐 list[ApplyResult])。
    pub apply_results: Vec<Value>,
    pub change_type: String,
    pub lifecycle_stage: String,
}

impl LocalApplyPreview {
    pub fn new(
        skill_name: impl Into<String>,
        records: Vec<RecordView>,
        apply_results: Vec<Value>,
        change_type: Option<String>,
    ) -> Self {
        Self {
            skill_name: skill_name.into(),
            records,
            apply_results,
            change_type: change_type.unwrap_or_else(|| SKILL_EXPERIENCE_ENTRY.to_string()),
            lifecycle_stage: LOCAL_APPLY_COMPLETED.to_string(),
        }
    }
}

/// 提交待审批快照的结果(对齐 PendingCommitResult)。
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PendingCommitResult {
    pub applied_count: usize,
    pub pending_count: usize,
    #[serde(default)]
    pub rejected_count: usize,
    #[serde(default)]
    pub errors: Vec<String>,
}

/// 逐条提交一个待审批快照(对齐 commit_pending_change)。
///
/// append 每次写一条记录:Ok(()) 计入 applied;Err(msg) 记录错误并将失败记录及其后
/// 全部记录保留为 pending(供重试),已批准记录从 payload 中移除。
pub fn commit_pending_change(
    pending_by_id: &mut BTreeMap<String, PendingChange>,
    change_id: &str,
    approved_record_ids: Option<&[String]>,
    mut append: impl FnMut(&str, &RecordView) -> Result<(), String>,
) -> Result<PendingCommitResult, String> {
    let Some(pending) = pending_by_id.get_mut(change_id) else {
        return Err(change_id.to_string());
    };
    if pending.change_type != SKILL_EXPERIENCE_ENTRY && pending.change_type != EXPERIENCE_ENTRY {
        return Err(pending.change_type.clone());
    }

    let all_records = pending.payload.clone();
    let (records, rejected_records): (Vec<RecordView>, Vec<RecordView>) = match approved_record_ids
    {
        None => (all_records.clone(), Vec::new()),
        Some(approved) => {
            let approved_set: std::collections::BTreeSet<&String> = approved.iter().collect();
            let mut records = Vec::new();
            let mut rejected = Vec::new();
            for record in all_records {
                if approved_set.contains(&record.id) {
                    records.push(record);
                } else {
                    rejected.push(record);
                }
            }
            (records, rejected)
        }
    };
    let mut errors: Vec<String> = Vec::new();

    if records.is_empty() {
        pending.payload.clear();
        pending_by_id.remove(change_id);
        return Ok(PendingCommitResult {
            applied_count: 0,
            pending_count: 0,
            rejected_count: rejected_records.len(),
            errors,
        });
    }

    let mut applied_count = 0usize;
    let mut remaining: Vec<RecordView> = Vec::new();
    for (index, record) in records.iter().enumerate() {
        match append(&pending.skill_name, record) {
            Ok(()) => applied_count += 1,
            Err(msg) => {
                errors.push(msg);
                remaining = records[index..].to_vec();
                break;
            }
        }
    }
    if remaining.is_empty() && applied_count == records.len() {
        remaining = Vec::new();
    }

    pending.payload = remaining.clone();
    if remaining.is_empty() {
        pending_by_id.remove(change_id);
    }

    Ok(PendingCommitResult {
        applied_count,
        pending_count: remaining.len(),
        rejected_count: rejected_records.len(),
        errors,
    })
}

/// simplify 动作执行计数(对齐 execute_simplify_actions;store 操作经闭包注入)。
///
/// 返回 {deleted, merged, refined, kept, errors};DELETE/MERGE/REFINE 的 store 失败与
/// 未知动作均计入 errors。
pub fn execute_simplify_actions(
    actions: &[Value],
    mut delete: impl FnMut(&str) -> Result<usize, String>,
    mut merge: impl FnMut(&str, &[String], &str) -> Result<bool, String>,
    mut refine: impl FnMut(&str, &str) -> Result<bool, String>,
) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::from([
        ("deleted".to_string(), 0usize),
        ("merged".to_string(), 0usize),
        ("refined".to_string(), 0usize),
        ("kept".to_string(), 0usize),
        ("errors".to_string(), 0usize),
    ]);

    for action in actions {
        let Some(obj) = action.as_object() else {
            *counts.get_mut("errors").unwrap() += 1;
            continue;
        };
        let action_type = obj
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("KEEP")
            .to_string();
        let record_id = obj
            .get("record_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let err = |counts: &mut BTreeMap<String, usize>| {
            *counts.get_mut("errors").unwrap() += 1;
        };

        match action_type.as_str() {
            "DELETE" => match delete(&record_id) {
                Ok(deleted) if deleted > 0 => *counts.get_mut("deleted").unwrap() += 1,
                Ok(_) => err(&mut counts),
                Err(_) => err(&mut counts),
            },
            "MERGE" => {
                let remove_ids: Vec<String> = obj
                    .get("merge_remove_ids")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                let new_content = obj
                    .get("new_content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                match merge(&record_id, &remove_ids, &new_content) {
                    Ok(true) => *counts.get_mut("merged").unwrap() += 1,
                    _ => err(&mut counts),
                }
            }
            "REFINE" => {
                let new_content = obj
                    .get("new_content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                match refine(&record_id, &new_content) {
                    Ok(true) => *counts.get_mut("refined").unwrap() += 1,
                    _ => err(&mut counts),
                }
            }
            "KEEP" => *counts.get_mut("kept").unwrap() += 1,
            _ => err(&mut counts),
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rec(id: &str) -> RecordView {
        RecordView {
            id: id.to_string(),
            summary: None,
            score: 0.5,
            timestamp: "t".to_string(),
            target: "body".to_string(),
            section: "Troubleshooting".to_string(),
            content: "c".to_string(),
        }
    }

    fn pending(ids: &[&str]) -> (String, PendingChange) {
        let p = PendingChange::make(
            "sk",
            &ids.iter().map(|i| rec(i)).collect::<Vec<_>>(),
            None,
            None,
        );
        (p.change_id.clone(), p)
    }

    #[test]
    fn local_apply_preview_defaults() {
        let preview = LocalApplyPreview::new("sk", vec![rec("r1")], Vec::new(), None);
        assert_eq!(preview.change_type, "skill_experience_entry");
        assert_eq!(preview.lifecycle_stage, "local_apply_completed");
        assert_eq!(preview.records.len(), 1);
    }

    #[test]
    fn commit_all_records() {
        let (cid, p) = pending(&["r1", "r2"]);
        let mut by_id = BTreeMap::from([(cid.clone(), p)]);
        let result = commit_pending_change(&mut by_id, &cid, None, |_, _| Ok(())).unwrap();
        assert_eq!(result.applied_count, 2);
        assert_eq!(result.pending_count, 0);
        assert_eq!(result.rejected_count, 0);
        assert!(by_id.is_empty());
    }

    #[test]
    fn commit_partial_apply_keeps_tail() {
        let (cid, p) = pending(&["r1", "r2", "r3"]);
        let mut by_id = BTreeMap::from([(cid.clone(), p)]);
        let result = commit_pending_change(&mut by_id, &cid, None, |_, rec| {
            if rec.id == "r2" {
                Err("store down".to_string())
            } else {
                Ok(())
            }
        })
        .unwrap();
        assert_eq!(result.applied_count, 1);
        assert_eq!(result.pending_count, 2); // r2 失败,r2/r3 保留
        assert_eq!(result.errors, vec!["store down"]);
        let remaining = by_id.get(&cid).unwrap();
        assert_eq!(
            remaining
                .payload
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            vec!["r2", "r3"],
        );
    }

    #[test]
    fn commit_approved_subset_rejects_rest() {
        let (cid, p) = pending(&["r1", "r2", "r3"]);
        let mut by_id = BTreeMap::from([(cid.clone(), p)]);
        let approved = vec!["r1".to_string(), "r3".to_string()];
        let result =
            commit_pending_change(&mut by_id, &cid, Some(&approved), |_, _| Ok(())).unwrap();
        assert_eq!(result.applied_count, 2);
        assert_eq!(result.rejected_count, 1);
        assert_eq!(result.pending_count, 0);
        assert!(by_id.is_empty());
    }

    #[test]
    fn commit_no_approved_records() {
        let (cid, p) = pending(&["r1", "r2"]);
        let mut by_id = BTreeMap::from([(cid.clone(), p)]);
        let approved: Vec<String> = Vec::new();
        let result =
            commit_pending_change(&mut by_id, &cid, Some(&approved), |_, _| Ok(())).unwrap();
        assert_eq!(result.applied_count, 0);
        assert_eq!(result.rejected_count, 2);
        assert!(by_id.is_empty());
    }

    #[test]
    fn commit_unknown_change_id_or_type() {
        let mut by_id = BTreeMap::new();
        assert!(commit_pending_change(&mut by_id, "nope", None, |_, _| Ok(())).is_err());
        let (cid, mut p) = pending(&["r1"]);
        p.change_type = "other".to_string();
        let mut by_id2 = BTreeMap::from([(cid.clone(), p)]);
        let err = commit_pending_change(&mut by_id2, &cid, None, |_, _| Ok(())).unwrap_err();
        assert_eq!(err, "other");
    }

    #[test]
    fn simplify_action_counts() {
        let actions = vec![
            json!({"action": "DELETE", "record_id": "r1"}),
            json!({"action": "MERGE", "record_id": "r2", "merge_remove_ids": ["r3"], "new_content": "n"}),
            json!({"action": "REFINE", "record_id": "r4", "new_content": "c"}),
            json!({"action": "KEEP", "record_id": "r5"}),
            json!({"action": "BOGUS", "record_id": "r6"}),
            json!({"action": "DELETE", "record_id": "missing"}),
        ];
        let counts = execute_simplify_actions(
            &actions,
            |id| {
                if id == "missing" { Ok(0) } else { Ok(1) }
            },
            |_, _, _| Ok(true),
            |_, _| Ok(true),
        );
        assert_eq!(counts["deleted"], 1);
        assert_eq!(counts["merged"], 1);
        assert_eq!(counts["refined"], 1);
        assert_eq!(counts["kept"], 1);
        assert_eq!(counts["errors"], 2); // BOGUS + DELETE missing
    }

    #[test]
    fn simplify_store_failure_counts_error() {
        let actions = vec![json!({"action": "REFINE", "record_id": "r1", "new_content": "c"})];
        let counts = execute_simplify_actions(
            &actions,
            |_| Ok(0),
            |_, _, _| Ok(false),
            |_, _| Err("boom".to_string()),
        );
        assert_eq!(counts["refined"], 0);
        assert_eq!(counts["errors"], 1);
    }
}
