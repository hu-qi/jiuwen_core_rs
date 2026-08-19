//! 进化草稿 schema 归一化(对齐 experience/draft_schema.py)。
//!
//! 纯逻辑:主题归一化(team-skill→swarm-skill)+ 演进草稿/精简草稿校验与视图。

use serde_json::{Map, Value};
use std::collections::BTreeSet;

use crate::protocols::{
    EVOLUTION_SUBJECT_KIND_VALUES, EVOLUTION_TARGET_VALUES, SIMPLIFY_ACTION_VALUES, VALID_SECTIONS,
};
use serde_json::json;

pub const SUMMARY_LIMIT: usize = 160;
pub const MAX_EVOLUTION_REVIEW_PROPOSALS: usize = 3;

/// 支持的演进经验主题类型(对齐 SUPPORTED_EXPERIENCE_SUBJECT_KINDS)。
pub const SUPPORTED_EXPERIENCE_SUBJECT_KINDS: [&str; 2] = ["skill", "swarm-skill"];

fn validation_error(message: &str) -> String {
    format!("evolution draft: {message}")
}

/// 归一化主题信封(对齐 EvolutionSubject)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EvolutionSubject {
    pub kind: String,
    pub name: String,
    pub scope: Option<Value>,
}

impl EvolutionSubject {
    /// 稳定 agent 面 payload(对齐 to_payload;scope 非空才带出)。
    pub fn to_payload(&self) -> Value {
        let mut payload = Map::new();
        payload.insert("kind".to_string(), Value::String(self.kind.clone()));
        payload.insert("name".to_string(), Value::String(self.name.clone()));
        if let Some(scope) = &self.scope
            && !scope.is_null()
        {
            payload.insert("scope".to_string(), scope.clone());
        }
        Value::Object(payload)
    }
}

/// 归一化遗留主题类型别名(对齐 normalize_evolution_subject_kind:team-skill→swarm-skill)。
pub fn normalize_evolution_subject_kind(kind: &str) -> String {
    let normalized = kind.trim();
    if normalized == "team-skill" {
        "swarm-skill".to_string()
    } else {
        normalized.to_string()
    }
}

/// 支持的演进经验主题类型(对齐 supported_experience_subject_kinds)。
pub fn supported_experience_subject_kinds() -> BTreeSet<String> {
    SUPPORTED_EXPERIENCE_SUBJECT_KINDS
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// 归一化 agent 面主题信封(对齐 normalize_subject)。
pub fn normalize_subject(
    raw: &Value,
    allowed_kinds: Option<&[&str]>,
) -> Result<EvolutionSubject, String> {
    let Some(obj) = raw.as_object() else {
        return Err(validation_error("subject must be an object"));
    };
    let kind_raw = obj
        .get("kind")
        .map(|v| v.as_str().unwrap_or("").to_string())
        .unwrap_or_default();
    let kind = kind_raw.trim().to_string();
    if kind.is_empty() {
        return Err(validation_error("subject.kind is required"));
    }
    let allowed: Vec<String> = match allowed_kinds {
        Some(kinds) => kinds
            .iter()
            .map(|k| normalize_evolution_subject_kind(k))
            .collect(),
        None => EVOLUTION_SUBJECT_KIND_VALUES
            .iter()
            .map(|k| normalize_evolution_subject_kind(k))
            .collect(),
    };
    let normalized_kind = normalize_evolution_subject_kind(&kind);
    if !allowed.contains(&normalized_kind) {
        // 消息列出原始(未归一化)值集合
        let mut sorted_allowed: Vec<String> = match allowed_kinds {
            Some(kinds) => kinds.iter().map(|k| k.to_string()).collect(),
            None => EVOLUTION_SUBJECT_KIND_VALUES
                .iter()
                .map(|k| k.to_string())
                .collect(),
        };
        sorted_allowed.sort();
        sorted_allowed.dedup();
        return Err(validation_error(&format!(
            "subject.kind must be one of: {}",
            sorted_allowed.join(", ")
        )));
    }
    let name = obj
        .get("name")
        .map(|v| v.as_str().unwrap_or("").to_string())
        .unwrap_or_default()
        .trim()
        .to_string();
    if name.is_empty() {
        return Err(validation_error("subject.name is required"));
    }
    let scope = obj.get("scope").cloned();
    if let Some(s) = &scope
        && !s.is_null()
        && !s.is_object()
    {
        return Err(validation_error("subject.scope must be an object"));
    }
    Ok(EvolutionSubject {
        kind: normalized_kind,
        name,
        scope,
    })
}

/// 演进草稿(对齐 EvolveDraft)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvolveDraft {
    pub subject: EvolutionSubject,
    pub experiences: Vec<Value>,
}

impl EvolveDraft {
    /// 审批面视图(对齐 approval_view)。
    pub fn approval_view(&self) -> Value {
        json!({ "experiences": self.experiences.clone() })
    }

    /// 持久化面视图(去掉 run-scoped source_refs;对齐 persistence_view)。
    pub fn persistence_view(&self) -> Value {
        let experiences: Vec<Value> = self
            .experiences
            .iter()
            .map(|item| {
                let mut persisted = item.clone();
                if let Some(obj) = persisted.as_object_mut() {
                    obj.remove("source_refs");
                }
                persisted
            })
            .collect();
        json!({ "experiences": experiences })
    }
}

/// 精简草稿(对齐 SimplifyDraft)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimplifyDraft {
    pub subject: EvolutionSubject,
    pub actions: Vec<Value>,
}

impl SimplifyDraft {
    pub fn approval_view(&self) -> Value {
        json!({ "actions": self.actions.clone() })
    }

    pub fn persistence_view(&self) -> Value {
        json!({ "actions": self.actions.clone() })
    }
}

fn normalize_summary(raw: &Map<String, Value>) -> Result<String, String> {
    let raw_summary = raw
        .get("summary")
        .map(|v| match v {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default();
    let summary = raw_summary.split_whitespace().collect::<Vec<_>>().join(" ");
    if summary.is_empty() {
        return Err(validation_error("experience summary is required"));
    }
    if summary.chars().count() > SUMMARY_LIMIT {
        return Err(validation_error(&format!(
            "experience summary must be at most {SUMMARY_LIMIT} characters"
        )));
    }
    Ok(summary)
}

fn normalize_section(raw: &Map<String, Value>, target: &str) -> Result<String, String> {
    if target == "script" {
        return Ok("Scripts".to_string());
    }
    let default = if target == "description" {
        "Instructions"
    } else {
        "Troubleshooting"
    };
    let section = raw
        .get("section")
        .map(|v| v.as_str().unwrap_or("").to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default.to_string())
        .trim()
        .to_string();
    if !VALID_SECTIONS.contains(&section.as_str()) {
        return Err(validation_error(&format!("invalid section: {section}")));
    }
    Ok(section)
}

fn normalize_script_filename(value: Option<&Value>) -> Result<Option<String>, String> {
    let filename = value
        .map(|v| v.as_str().unwrap_or("").to_string())
        .unwrap_or_default()
        .trim()
        .to_string();
    if filename.is_empty() {
        return Ok(None);
    }
    if filename == "."
        || filename == ".."
        || filename.contains('/')
        || filename.contains('\\')
        || filename.starts_with('/')
    {
        return Err(validation_error("script_filename must be a file name"));
    }
    Ok(Some(filename))
}

fn normalize_source_refs(value: Option<&Value>) -> Result<Vec<String>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    let Some(items) = value.as_array() else {
        return Err(validation_error("source_refs must be a list"));
    };
    let mut refs = Vec::new();
    for item in items {
        let normalized_ref = match item {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        }
        .trim()
        .to_string();
        if !normalized_ref.is_empty() {
            refs.push(normalized_ref);
        }
    }
    Ok(refs)
}

/// 归一化演进草稿条目(对齐 normalize_evolve_draft)。
pub fn normalize_evolve_draft(
    subject: EvolutionSubject,
    experiences: &Value,
) -> Result<EvolveDraft, String> {
    let Some(items) = experiences.as_array() else {
        return Err(validation_error("experiences must be a non-empty list"));
    };
    if items.is_empty() {
        return Err(validation_error("experiences must be a non-empty list"));
    }
    let mut normalized = Vec::new();
    for (index, raw) in items.iter().enumerate() {
        let Some(obj) = raw.as_object() else {
            return Err(validation_error(&format!(
                "experience at index {index} must be an object"
            )));
        };
        let content = obj
            .get("content")
            .map(|v| v.as_str().unwrap_or("").to_string())
            .unwrap_or_default()
            .trim()
            .to_string();
        if content.is_empty() {
            return Err(validation_error("experience content is required"));
        }
        let target = obj
            .get("target")
            .map(|v| v.as_str().unwrap_or("body").to_string())
            .unwrap_or_else(|| "body".to_string())
            .trim()
            .to_string();
        if !EVOLUTION_TARGET_VALUES.contains(&target.as_str()) {
            return Err(validation_error(&format!("invalid target: {target}")));
        }
        let mut item = Map::new();
        item.insert(
            "summary".to_string(),
            Value::String(normalize_summary(obj)?),
        );
        item.insert("content".to_string(), Value::String(content));
        item.insert("target".to_string(), Value::String(target.clone()));
        item.insert(
            "section".to_string(),
            Value::String(normalize_section(obj, &target)?),
        );
        item.insert(
            "reason".to_string(),
            Value::String(
                obj.get("reason")
                    .map(|v| v.as_str().unwrap_or("").to_string())
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
            ),
        );
        if let Some(script_filename) = normalize_script_filename(obj.get("script_filename"))? {
            item.insert(
                "script_filename".to_string(),
                Value::String(script_filename),
            );
        }
        for key in ["script_language", "script_purpose"] {
            let value = obj
                .get(key)
                .map(|v| v.as_str().unwrap_or("").to_string())
                .unwrap_or_default()
                .trim()
                .to_string();
            if !value.is_empty() {
                item.insert(key.to_string(), Value::String(value));
            }
        }
        let refs = normalize_source_refs(obj.get("source_refs"))?;
        if !refs.is_empty() {
            item.insert(
                "source_refs".to_string(),
                Value::Array(refs.into_iter().map(Value::String).collect()),
            );
        }
        normalized.push(Value::Object(item));
    }
    Ok(EvolveDraft {
        subject,
        experiences: normalized,
    })
}

fn normalize_merge_remove_ids(raw_ids: &Value, record_id: &str) -> Result<Vec<String>, String> {
    let mut seen = BTreeSet::new();
    let mut normalized = Vec::new();
    let items = raw_ids.as_array().cloned().unwrap_or_default();
    for raw_id in items {
        let remove_id = match raw_id {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        }
        .trim()
        .to_string();
        if remove_id.is_empty() {
            return Err(validation_error(
                "merge_remove_ids must not contain empty values",
            ));
        }
        if remove_id == record_id {
            return Err(validation_error(
                "merge_remove_ids must not contain record_id",
            ));
        }
        if seen.contains(&remove_id) {
            return Err(validation_error(&format!(
                "duplicate merge_remove_id: {remove_id}"
            )));
        }
        seen.insert(remove_id.clone());
        normalized.push(remove_id);
    }
    Ok(normalized)
}

/// 归一化精简动作草稿(对齐 normalize_simplify_draft)。
pub fn normalize_simplify_draft(
    subject: EvolutionSubject,
    actions: &Value,
) -> Result<SimplifyDraft, String> {
    let Some(items) = actions.as_array() else {
        return Err(validation_error("actions must be a non-empty list"));
    };
    if items.is_empty() {
        return Err(validation_error("actions must be a non-empty list"));
    }
    let mut normalized = Vec::new();
    for (index, raw) in items.iter().enumerate() {
        let Some(obj) = raw.as_object() else {
            return Err(validation_error(&format!(
                "action at index {index} must be an object"
            )));
        };
        let action = obj
            .get("action")
            .map(|v| v.as_str().unwrap_or("").to_string())
            .unwrap_or_default()
            .trim()
            .to_uppercase();
        if !SIMPLIFY_ACTION_VALUES.contains(&action.as_str()) {
            return Err(validation_error(&format!(
                "invalid simplify action: {action}"
            )));
        }
        let record_id = obj
            .get("record_id")
            .map(|v| v.as_str().unwrap_or("").to_string())
            .unwrap_or_default()
            .trim()
            .to_string();
        if record_id.is_empty() {
            return Err(validation_error("record_id is required"));
        }
        let mut item = Map::new();
        item.insert("action".to_string(), Value::String(action.clone()));
        item.insert("record_id".to_string(), Value::String(record_id.clone()));
        item.insert(
            "reason".to_string(),
            Value::String(
                obj.get("reason")
                    .map(|v| v.as_str().unwrap_or("").to_string())
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
            ),
        );
        let new_content = obj
            .get("new_content")
            .map(|v| v.as_str().unwrap_or("").to_string())
            .unwrap_or_default()
            .trim()
            .to_string();
        let merge_remove_ids = normalize_merge_remove_ids(
            obj.get("merge_remove_ids")
                .unwrap_or(&Value::Array(Vec::new())),
            &record_id,
        )?;
        if action == "REFINE" && new_content.is_empty() {
            return Err(validation_error("REFINE requires new_content"));
        }
        if action == "MERGE" {
            if new_content.is_empty() {
                return Err(validation_error("MERGE requires new_content"));
            }
            if merge_remove_ids.is_empty() {
                return Err(validation_error("MERGE requires merge_remove_ids"));
            }
        }
        if !new_content.is_empty() && (action == "REFINE" || action == "MERGE") {
            item.insert("new_content".to_string(), Value::String(new_content));
        }
        if !merge_remove_ids.is_empty() {
            item.insert(
                "merge_remove_ids".to_string(),
                Value::Array(merge_remove_ids.into_iter().map(Value::String).collect()),
            );
        }
        normalized.push(Value::Object(item));
    }
    Ok(SimplifyDraft {
        subject,
        actions: normalized,
    })
}

/// 校验精简动作记录引用(对齐 validate_simplify_record_refs)。
pub fn validate_simplify_record_refs(
    draft: &SimplifyDraft,
    existing_record_ids: &BTreeSet<String>,
) -> Result<(), String> {
    for action in &draft.actions {
        let obj = action
            .as_object()
            .ok_or_else(|| validation_error("action must be an object"))?;
        let record_id = obj["record_id"].as_str().unwrap_or("");
        if !existing_record_ids.contains(record_id) {
            return Err(validation_error(&format!("record not found: {record_id}")));
        }
        if let Some(remove_ids) = obj.get("merge_remove_ids").and_then(|v| v.as_array()) {
            for remove_id in remove_ids {
                let rid = remove_id.as_str().unwrap_or("");
                if !existing_record_ids.contains(rid) {
                    return Err(validation_error(&format!("record not found: {rid}")));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn subject_kind_normalization() {
        assert_eq!(
            normalize_evolution_subject_kind("team-skill"),
            "swarm-skill"
        );
        assert_eq!(normalize_evolution_subject_kind("skill"), "skill");
        assert_eq!(
            supported_experience_subject_kinds(),
            BTreeSet::from(["skill".to_string(), "swarm-skill".to_string()]),
        );
    }

    #[test]
    fn normalize_subject_valid_and_payload() {
        let s = normalize_subject(&json!({"kind": "team-skill", "name": "team"}), None).unwrap();
        assert_eq!(s.kind, "swarm-skill");
        assert_eq!(s.name, "team");
        assert_eq!(s.to_payload()["kind"], "swarm-skill");
        let with_scope = normalize_subject(
            &json!({"kind": "skill", "name": "sk", "scope": {"a": 1}}),
            None,
        )
        .unwrap();
        assert_eq!(with_scope.to_payload()["scope"]["a"], 1);
    }

    #[test]
    fn normalize_subject_errors() {
        let e1 = normalize_subject(&json!("x"), None).unwrap_err();
        assert!(e1.contains("subject must be an object"));
        let e2 = normalize_subject(&json!({"name": "x"}), None).unwrap_err();
        assert!(e2.contains("subject.kind is required"));
        let e3 = normalize_subject(&json!({"kind": "agent", "name": "x"}), None).unwrap_err();
        assert!(e3.contains("subject.kind must be one of"));
        let e4 = normalize_subject(&json!({"kind": "skill"}), None).unwrap_err();
        assert!(e4.contains("subject.name is required"));
        let e5 = normalize_subject(&json!({"kind": "skill", "name": "x", "scope": 1}), None)
            .unwrap_err();
        assert!(e5.contains("subject.scope must be an object"));
    }

    #[test]
    fn evolve_draft_normalization() {
        let subject = normalize_subject(&json!({"kind": "skill", "name": "sk"}), None).unwrap();
        let experiences = json!([
            {
                "summary": "总结",
                "content": "内容",
                "target": "body",
                "reason": "原因",
                "source_refs": ["r1", "r2"],
            }
        ]);
        let draft = normalize_evolve_draft(subject, &experiences).unwrap();
        assert_eq!(draft.experiences.len(), 1);
        let item = &draft.experiences[0];
        assert_eq!(item["target"], "body");
        assert_eq!(item["section"], "Troubleshooting");
        // persistence_view 去掉 source_refs
        let persisted = draft.persistence_view();
        assert!(persisted["experiences"][0].get("source_refs").is_none());
        let approval = draft.approval_view();
        assert!(approval["experiences"][0]["source_refs"].is_array());
    }

    #[test]
    fn evolve_draft_errors() {
        let subject = normalize_subject(&json!({"kind": "skill", "name": "sk"}), None).unwrap();
        assert!(normalize_evolve_draft(subject.clone(), &json!([])).is_err());
        assert!(normalize_evolve_draft(subject.clone(), &json!("x")).is_err());
        let bad_target = json!([{"summary": "s", "content": "c", "target": "other"}]);
        let err = normalize_evolve_draft(subject.clone(), &bad_target).unwrap_err();
        assert!(err.contains("invalid target: other"));
        let no_content = json!([{"summary": "s"}]);
        assert!(normalize_evolve_draft(subject, &no_content).is_err());
    }

    #[test]
    fn summary_limit_and_section_defaults() {
        let subject = normalize_subject(&json!({"kind": "skill", "name": "sk"}), None).unwrap();
        let long_summary = "x".repeat(200);
        let exp = json!([{"summary": long_summary, "content": "c"}]);
        let err = normalize_evolve_draft(subject.clone(), &exp).unwrap_err();
        assert!(err.contains("at most 160"));
        // description 默认 Instructions;script 固定 Scripts
        let exp2 = json!([{"summary": "s", "content": "c", "target": "description"}]);
        let d = normalize_evolve_draft(subject.clone(), &exp2).unwrap();
        assert_eq!(d.experiences[0]["section"], "Instructions");
        let exp3 = json!([{"summary": "s", "content": "c", "target": "script", "script_filename": "x.py"}]);
        let d3 = normalize_evolve_draft(subject, &exp3).unwrap();
        assert_eq!(d3.experiences[0]["section"], "Scripts");
        assert_eq!(d3.experiences[0]["script_filename"], "x.py");
    }

    #[test]
    fn script_filename_validation() {
        let subject = normalize_subject(&json!({"kind": "skill", "name": "sk"}), None).unwrap();
        for bad in ["a/b.py", "a\\b.py", "/abs.py", ".", ".."] {
            let exp = json!([{"summary": "s", "content": "c", "target": "script", "script_filename": bad}]);
            let err = normalize_evolve_draft(subject.clone(), &exp).unwrap_err();
            assert!(
                err.contains("script_filename must be a file name"),
                "{bad}: {err}"
            );
        }
    }

    #[test]
    fn simplify_draft_normalization_and_refs() {
        let subject = normalize_subject(&json!({"kind": "skill", "name": "sk"}), None).unwrap();
        let actions = json!([
            {
                "action": "refine",
                "record_id": "r1",
                "new_content": "新内容",
                "reason": "原因",
            },
            {
                "action": "MERGE",
                "record_id": "r2",
                "new_content": "合并",
                "merge_remove_ids": ["r3", "r4"],
            },
        ]);
        let draft = normalize_simplify_draft(subject, &actions).unwrap();
        assert_eq!(draft.actions[0]["action"], "REFINE");
        assert_eq!(draft.actions[0]["new_content"], "新内容");
        assert_eq!(draft.actions[1]["merge_remove_ids"], json!(["r3", "r4"]));
        let existing = BTreeSet::from([
            "r1".to_string(),
            "r2".to_string(),
            "r3".to_string(),
            "r4".to_string(),
        ]);
        assert!(validate_simplify_record_refs(&draft, &existing).is_ok());
        let missing = BTreeSet::from(["r1".to_string()]);
        let err = validate_simplify_record_refs(&draft, &missing).unwrap_err();
        assert!(err.contains("record not found: r2"));
    }

    #[test]
    fn simplify_draft_errors() {
        let subject = normalize_subject(&json!({"kind": "skill", "name": "sk"}), None).unwrap();
        assert!(normalize_simplify_draft(subject.clone(), &json!([])).is_err());
        let bad_action = json!([{"action": "X", "record_id": "r1"}]);
        assert!(normalize_simplify_draft(subject.clone(), &bad_action).is_err());
        let refine_no_content = json!([{"action": "REFINE", "record_id": "r1"}]);
        let err = normalize_simplify_draft(subject.clone(), &refine_no_content).unwrap_err();
        assert!(err.contains("REFINE requires new_content"));
        let merge_no_ids = json!([{"action": "MERGE", "record_id": "r1", "new_content": "x"}]);
        assert!(normalize_simplify_draft(subject.clone(), &merge_no_ids).is_err());
        let dup = json!([{"action": "MERGE", "record_id": "r1", "new_content": "x", "merge_remove_ids": ["r1"]}]);
        let err2 = normalize_simplify_draft(subject, &dup).unwrap_err();
        assert!(err2.contains("must not contain record_id"));
    }
}
