//! 进化更新执行(对齐 agent_evolving/update_execution.py)。
//!
//! 纯逻辑:批量更新归一化执行 + 结果聚合。

use ah_contracts::evolving::{ApplyResult, UpdateEffect, UpdateKey, UpdateMode, UpdateValue};
use std::collections::BTreeMap;

pub type OperatorApply = Box<dyn Fn(&str, &UpdateValue) -> ApplyResult + Send + Sync>;

/// 归一化批量更新(对齐 normalize_updates;过滤 None 值)。
pub fn normalize_updates(
    updates: &BTreeMap<UpdateKey, Option<serde_json::Value>>,
) -> BTreeMap<UpdateKey, UpdateValue> {
    updates
        .iter()
        .filter(|(_, v)| v.is_some())
        .map(|(key, v)| {
            let target = key.1.clone();
            let value = v.clone().unwrap();
            (key.clone(), UpdateValue::normalize(value, Some(&target)))
        })
        .collect()
}

/// 执行批量更新并聚合结果(对齐 execute_updates 的纯逻辑部分)。
/// operators 为 operator_id → apply 函数;None 表示 operator 缺失。
pub fn execute_updates(
    operators: &BTreeMap<String, OperatorApply>,
    updates: &BTreeMap<UpdateKey, Option<serde_json::Value>>,
) -> Vec<ApplyResult> {
    let mut results = Vec::new();
    let normalized = normalize_updates(updates);
    for (key, update) in &normalized {
        let (operator_id, target) = key;
        match operators.get(operator_id) {
            Some(apply) => results.push(apply(target, update)),
            None => results.push(ApplyResult {
                operator_id: operator_id.clone(),
                target: target.clone(),
                applied: false,
                mode: update.mode,
                effect: update.effect,
                value: Some(update.payload.clone()),
                records: vec![],
                change_type: update.change_type.clone(),
                lifecycle_stage: None,
                pending_change_id: None,
                errors: vec![format!("operator not found: {operator_id}")],
                metadata: update.metadata.clone(),
            }),
        }
    }
    // None 值更新 → 显式失败结果
    for (key, value) in updates {
        if value.is_none() {
            results.push(ApplyResult {
                operator_id: key.0.clone(),
                target: key.1.clone(),
                applied: false,
                mode: UpdateMode::Replace,
                effect: UpdateEffect::State,
                value: None,
                records: vec![],
                change_type: None,
                lifecycle_stage: None,
                pending_change_id: None,
                errors: vec!["update value is None".to_string()],
                metadata: serde_json::Map::new(),
            });
        }
    }
    results
}

/// 聚合应用结果计数(对齐 summarize_apply_results)。
pub fn summarize_apply_results(results: &[ApplyResult]) -> (usize, usize, usize) {
    let total = results.len();
    let applied = results.iter().filter(|r| r.ok()).count();
    (total, applied, total - applied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::evolving::{UpdateEffect, UpdateMode};
    use serde_json::json;
    use std::collections::BTreeMap;

    fn updates() -> BTreeMap<UpdateKey, Option<serde_json::Value>> {
        let mut m = BTreeMap::new();
        m.insert(
            ("op1".to_string(), "body".to_string()),
            Some(json!("new body")),
        );
        m.insert(
            ("op2".to_string(), "experiences".to_string()),
            Some(json!({"e": 1})),
        );
        m.insert(("op3".to_string(), "script".to_string()), None);
        m
    }

    #[test]
    fn normalize_updates_filters_none_and_targets_experiences() {
        let u = updates();
        let normalized = normalize_updates(&u);
        assert_eq!(normalized.len(), 2);
        let exp = normalized
            .get(&("op2".to_string(), "experiences".to_string()))
            .unwrap();
        assert_eq!(exp.mode, UpdateMode::Append);
        assert_eq!(exp.effect, UpdateEffect::PendingChange);
        let body = normalized
            .get(&("op1".to_string(), "body".to_string()))
            .unwrap();
        assert_eq!(body.mode, UpdateMode::Replace);
    }

    #[test]
    fn execute_updates_missing_operator_errors() {
        let operators = BTreeMap::new();
        let results = execute_updates(&operators, &updates());
        // op1, op2 normalized + op3 none = 3 results
        assert_eq!(results.len(), 3);
        let missing = results.iter().find(|r| r.operator_id == "op1").unwrap();
        assert!(!missing.applied);
        assert!(missing.errors[0].contains("operator not found"));
        let none_result = results.iter().find(|r| r.operator_id == "op3").unwrap();
        assert!(none_result.errors[0].contains("None"));
    }

    #[test]
    fn execute_updates_applies_with_operator() {
        let mut operators = BTreeMap::new();
        operators.insert(
            "op1".to_string(),
            Box::new(|target: &str, update: &UpdateValue| ApplyResult {
                operator_id: "op1".to_string(),
                target: target.to_string(),
                applied: true,
                mode: update.mode,
                effect: update.effect,
                value: Some(update.payload.clone()),
                records: vec![],
                change_type: update.change_type.clone(),
                lifecycle_stage: None,
                pending_change_id: None,
                errors: vec![],
                metadata: update.metadata.clone(),
            }) as OperatorApply,
        );
        let results = execute_updates(&operators, &updates());
        let applied = results.iter().find(|r| r.operator_id == "op1").unwrap();
        assert!(applied.applied);
        assert!(applied.ok());
    }

    #[test]
    fn summarize_counts_applied_and_failed() {
        let results = vec![
            ApplyResult {
                operator_id: "a".to_string(),
                target: "t".to_string(),
                applied: true,
                mode: UpdateMode::Replace,
                effect: UpdateEffect::State,
                value: None,
                records: vec![],
                change_type: None,
                lifecycle_stage: None,
                pending_change_id: None,
                errors: vec![],
                metadata: serde_json::Map::new(),
            },
            ApplyResult {
                operator_id: "b".to_string(),
                target: "t".to_string(),
                applied: false,
                mode: UpdateMode::Replace,
                effect: UpdateEffect::State,
                value: None,
                records: vec![],
                change_type: None,
                lifecycle_stage: None,
                pending_change_id: None,
                errors: vec!["x".to_string()],
                metadata: serde_json::Map::new(),
            },
        ];
        let (total, applied, failed) = summarize_apply_results(&results);
        assert_eq!(total, 2);
        assert_eq!(applied, 1);
        assert_eq!(failed, 1);
    }
}
