//! 优化经验检索器(对齐 rsi/optimization_experience_learner/learner.py
//! `ExperienceRetriever` 确定性部分)。
//!
//! 索引后端支持本地 `index.yaml` 与可选的 `BaseKVStore` 外部 ledger;检索按
//! 状态/优化类型/阶段/角色/候选模块/失败签名/机制类型过滤,按
//! (confidence, created_at) 降序排序,limit 截断,summary 预算截断。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ah_contracts::store::{BaseKVStore, StoreError};

use ah_contracts::rsi_learner::{
    allowed_statuses, bounded_list, confidence_score, entry_matches_query, first_text, truncate,
};
use serde_json::Value;

/// 检索查询(对齐 OptimizationExperienceRetrievalQuery 确定性字段)。
#[derive(Debug, Clone, Default)]
pub struct ExperienceRetrievalQuery {
    pub optimization_type: String,
    pub stage: String,
    pub target_members: Vec<String>,
    pub candidate_modules: Vec<String>,
    pub limit: usize,
    pub learning_statuses: Vec<String>,
    pub allow_provisional: bool,
    pub failure_signature: String,
    pub mechanism_type: String,
    pub summary_char_budget: usize,
}

impl ExperienceRetrievalQuery {
    /// 默认 summary 预算(对齐 metadata 默认 1200)。
    pub fn new(optimization_type: impl Into<String>, stage: impl Into<String>) -> Self {
        Self {
            optimization_type: optimization_type.into(),
            stage: stage.into(),
            limit: 5,
            summary_char_budget: 1200,
            ..Default::default()
        }
    }
}

/// 索引后端检索器(对齐 `ExperienceRetriever`)。
pub struct ExperienceRetriever {
    roots: Vec<PathBuf>,
    external_store: Option<Arc<dyn BaseKVStore>>,
}

impl ExperienceRetriever {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self {
            roots,
            external_store: None,
        }
    }

    /// 注入外部经验 ledger;本地 index.yaml 仍作为补充来源。
    pub fn with_external_store(mut self, store: Arc<dyn BaseKVStore>) -> Self {
        self.external_store = Some(store);
        self
    }

    /// 检索(对齐 `retrieve`):返回 (matches, metadata)。
    pub fn retrieve(&self, query: &ExperienceRetrievalQuery) -> (Vec<Value>, Value) {
        if self.roots.is_empty() && self.external_store.is_none() {
            return (
                vec![],
                serde_json::json!({
                    "retrieval_status": "empty",
                    "searched_roots": [],
                }),
            );
        }
        let allowed = allowed_statuses(
            &Value::Array(
                query
                    .learning_statuses
                    .iter()
                    .map(|s| Value::String(s.clone()))
                    .collect(),
            ),
            query.allow_provisional,
        );
        let entries = match self.load_entries() {
            Ok(entries) => entries,
            Err(error) => {
                return (
                    vec![],
                    serde_json::json!({
                        "retrieval_status": "error",
                        "error": error.to_string(),
                        "searched_roots": self.roots.iter().map(|r| r.to_string_lossy().to_string()).collect::<Vec<_>>(),
                        "external_store": self.external_store.is_some(),
                    }),
                );
            }
        };
        let filtered: Vec<Value> = entries
            .into_iter()
            .filter(|entry| {
                let Some(obj) = entry.as_object() else {
                    return false;
                };
                entry_matches_query(
                    obj,
                    &query.optimization_type,
                    &query.stage,
                    &query.target_members,
                    &query.candidate_modules,
                    &query.failure_signature,
                    &query.mechanism_type,
                    &allowed,
                )
            })
            .collect();
        let mut ranked = filtered.clone();
        ranked.sort_by(|a, b| {
            let score_a =
                confidence_score(a.get("confidence").and_then(Value::as_str).unwrap_or(""));
            let score_b =
                confidence_score(b.get("confidence").and_then(Value::as_str).unwrap_or(""));
            let created_a = a
                .get("created_at")
                .map(Value::to_string)
                .unwrap_or_default();
            let created_b = b
                .get("created_at")
                .map(Value::to_string)
                .unwrap_or_default();
            (score_b, created_b.clone()).cmp(&(score_a, created_a.clone()))
        });
        let limit = query.limit;
        let budget = if query.summary_char_budget == 0 {
            1200
        } else {
            query.summary_char_budget
        };
        let matches: Vec<Value> = ranked
            .iter()
            .take(limit)
            .map(|entry| bounded_match(entry, budget))
            .collect();
        let returned_count = matches.len();
        (
            matches,
            serde_json::json!({
                "retrieval_status": "ok",
                "searched_roots": self
                    .roots
                    .iter()
                    .map(|r| r.to_string_lossy().to_string())
                    .collect::<Vec<_>>(),
                "external_store": self.external_store.is_some(),
                "matched_count": filtered.len(),
                "returned_count": returned_count,
                "allowed_statuses": allowed,
            }),
        )
    }

    /// 加载全部索引条目(本地 index.yaml + 外部 ledger;外部 id 优先去重)。
    fn load_entries(&self) -> Result<Vec<Value>, StoreError> {
        let mut entries: Vec<Value> = Vec::new();
        let mut external_ids = std::collections::HashSet::new();
        if let Some(store) = &self.external_store {
            for entry in store.scan("rsi:experience:")? {
                if entry.value.is_object() {
                    if let Some(id) = entry.value.get("experience_id").and_then(Value::as_str) {
                        external_ids.insert(id.to_string());
                    }
                    entries.push(entry.value);
                }
            }
        }
        for root in &self.roots {
            let index = read_index(&root.join("index.yaml"));
            if let Some(Value::Array(items)) = index.get("experiences") {
                for item in items {
                    if item.is_object()
                        && item
                            .get("experience_id")
                            .and_then(Value::as_str)
                            .is_none_or(|id| !external_ids.contains(id))
                    {
                        entries.push(item.clone());
                    }
                }
            }
        }
        Ok(entries)
    }
}

/// 读取 index.yaml(对齐 `_read_index`)。
pub fn read_index(path: &Path) -> serde_json::Map<String, Value> {
    let index = read_yaml(path);
    let experiences = match index.get("experiences") {
        Some(Value::Array(items)) => Value::Array(items.clone()),
        _ => Value::Array(vec![]),
    };
    let version = index.get("version").and_then(Value::as_i64).unwrap_or(1);
    let mut out = serde_json::Map::new();
    out.insert("version".to_string(), Value::from(version));
    out.insert("experiences".to_string(), experiences);
    out
}

/// 读取 YAML 为对象(对齐 `_read_yaml`);文件缺失/非对象 → 空对象。
pub fn read_yaml(path: &Path) -> serde_json::Map<String, Value> {
    if !path.is_file() {
        return serde_json::Map::new();
    }
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return serde_json::Map::new(),
    };
    match serde_yaml::from_str::<Value>(&text) {
        Ok(Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    }
}

/// 读取结构化文件(对齐 `_read_structured`):.json 用 JSON,其余按 YAML。
pub fn read_structured(path: &Path) -> Value {
    let lower = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if lower == "json" {
        return std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or(Value::Null);
    }
    let map = read_yaml(path);
    if map.is_empty() {
        Value::Null
    } else {
        Value::Object(map)
    }
}

/// 有界匹配视图(对齐 `_bounded_match`):读取 stage YAML,组装扁平视图。
pub fn bounded_match(entry: &Value, summary_budget: usize) -> Value {
    let entry_obj = entry.as_object().cloned().unwrap_or_default();
    let stage_path = entry_obj
        .get("stage_experience_path")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .map(|p| expand_user(&p))
        .unwrap_or_default();
    let payload = read_yaml(&stage_path);
    let experience = first_mapping(payload.get("experience"));
    let summary = first_text(&[payload.get("summary"), entry_obj.get("summary")]);
    let truncated = summary.chars().count() > summary_budget;
    let summary = truncate(&summary, summary_budget);

    serde_json::json!({
        "experience_id": str_value(entry_obj.get("experience_id")),
        "optimization_type": str_value(entry_obj.get("optimization_type")),
        "role": str_value(entry_obj.get("role")),
        "stage": str_value(entry_obj.get("stage")),
        "learning_status": str_value(entry_obj.get("learning_status")),
        "component_layer": str_value(entry_obj.get("component_layer")),
        "failure_signature": str_value(entry_obj.get("failure_signature")),
        "mechanism_type": str_value(entry_obj.get("mechanism_type")),
        "summary": summary,
        "experience": {
            "problem_signature": experience.get("problem_signature").cloned().unwrap_or(Value::Object(serde_json::Map::new())),
            "general_principles": bounded_list(experience.get("general_principles").unwrap_or(&Value::Null), summary_budget),
            "anti_patterns": bounded_list(experience.get("anti_patterns").unwrap_or(&Value::Null), summary_budget),
            "applicable_conditions": bounded_list(experience.get("applicable_conditions").unwrap_or(&Value::Null), summary_budget),
            "negative_conditions": bounded_list(experience.get("negative_conditions").unwrap_or(&Value::Null), summary_budget),
            "confidence": experience.get("confidence").cloned().unwrap_or(Value::String("medium".to_string())),
        },
        "source_artifact_paths": payload.get("source_artifact_paths").cloned().unwrap_or(Value::Array(vec![])),
        "experience_ref_path": entry_obj.get("experience_ref_path").map(Value::to_string).unwrap_or_default(),
        "stage_experience_path": entry_obj.get("stage_experience_path").map(Value::to_string).unwrap_or_default(),
        "truncated": truncated,
    })
}

/// 值 → 字符串(字符串原样,其余 JSON 序列化;None → 空串)。
fn str_value(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

/// 展开 ~(对齐 Path.expanduser)。
fn expand_user(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    path.to_path_buf()
}

/// 取首个 mapping(对齐 `_first_mapping`)。
fn first_mapping(value: Option<&Value>) -> serde_json::Map<String, Value> {
    match value {
        Some(Value::Object(map)) => map.clone(),
        Some(Value::Array(items)) => {
            for item in items {
                if let Value::Object(map) = item {
                    return map.clone();
                }
            }
            serde_json::Map::new()
        }
        _ => serde_json::Map::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::seam::Seam;
    use ah_contracts::store::{BaseKVStore, StoreError};
    use serde_json::json;
    use std::sync::Arc;

    #[derive(Default)]
    struct MemoryStore(std::sync::Mutex<std::collections::BTreeMap<String, Value>>);

    impl Seam for MemoryStore {}

    impl BaseKVStore for MemoryStore {
        fn get(&self, key: &str) -> Result<Option<Value>, StoreError> {
            Ok(self.0.lock().unwrap().get(key).cloned())
        }

        fn set(&self, key: &str, value: Value) -> Result<(), StoreError> {
            self.0.lock().unwrap().insert(key.to_string(), value);
            Ok(())
        }

        fn delete(&self, key: &str) -> Result<(), StoreError> {
            self.0.lock().unwrap().remove(key);
            Ok(())
        }

        fn scan(&self, prefix: &str) -> Result<Vec<ah_contracts::store::KvEntry>, StoreError> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .iter()
                .filter(|(key, _)| key.starts_with(prefix))
                .map(|(key, value)| ah_contracts::store::KvEntry {
                    key: key.clone(),
                    value: value.clone(),
                    updated_ms: 0,
                })
                .collect())
        }
    }

    fn write_yaml(path: &Path, value: &Value) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let text = serde_yaml::to_string(value).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn retriever_filters_and_ranks() {
        let root = std::env::temp_dir().join(format!("ah-rsi-learn-{}", std::process::id()));
        // 阶段经验文件。
        let stage_dir = root.join("prompt_experience_001/stages");
        write_yaml(
            &stage_dir.join("001_evaluate.yaml"),
            &json!({
                "summary": "prompt 改进经验",
                "experience": {
                    "problem_signature": {"sig": 1},
                    "general_principles": ["原则一", "原则二"],
                    "confidence": "high"
                }
            }),
        );
        // 索引。
        write_yaml(
            &root.join("prompt_experience_001/index.yaml"),
            &json!({
                "version": 1,
                "experiences": [
                    {
                        "experience_id": "exp-1",
                        "optimization_type": "prompt",
                        "stage": "evaluate",
                        "role": "leader",
                        "learning_status": "accepted",
                        "component_layer": "scheduler",
                        "confidence": "high",
                        "created_at": "2026-01-02",
                        "stage_experience_path": "./stages/001_evaluate.yaml",
                        "summary": "prompt 改进经验"
                    },
                    {
                        "experience_id": "exp-2",
                        "optimization_type": "prompt",
                        "stage": "train",
                        "learning_status": "accepted",
                        "confidence": "low",
                        "created_at": "2026-01-01",
                        "stage_experience_path": ""
                    },
                    {
                        "experience_id": "exp-3",
                        "optimization_type": "prompt",
                        "stage": "evaluate",
                        "role": "teammate",
                        "learning_status": "provisional",
                        "confidence": "medium",
                        "created_at": "2026-01-03",
                        "stage_experience_path": ""
                    }
                ]
            }),
        );

        let retriever = ExperienceRetriever::new(vec![root.join("prompt_experience_001")]);
        let query = ExperienceRetrievalQuery::new("prompt", "evaluate");
        let (matches, metadata) = retriever.retrieve(&query);
        // 仅 exp-1(stage=evaluate,status=accepted,默认不 allow_provisional)。
        assert_eq!(matches.len(), 1, "metadata: {metadata}");
        assert_eq!(matches[0]["experience_id"], "exp-1");
        assert_eq!(metadata["retrieval_status"], "ok");
        assert_eq!(metadata["matched_count"], 1);
        // summary 预算截断标记。
        assert_eq!(matches[0]["truncated"], false);

        // 允许 provisional → 命中 exp-1 + exp-3。
        let mut query = ExperienceRetrievalQuery::new("prompt", "evaluate");
        query.allow_provisional = true;
        let (matches, metadata) = retriever.retrieve(&query);
        assert_eq!(matches.len(), 2, "metadata: {metadata}");

        // 候选模块过滤。
        let mut query = ExperienceRetrievalQuery::new("prompt", "evaluate");
        query.candidate_modules = vec!["scheduler".to_string()];
        let (matches, _) = retriever.retrieve(&query);
        assert_eq!(matches.len(), 1);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn external_store_entries_are_retrieved_without_local_roots() {
        let store = Arc::new(MemoryStore::default());
        store
            .set(
                "rsi:experience:exp-external",
                json!({
                    "experience_id": "exp-external",
                    "optimization_type": "prompt",
                    "stage": "evaluate",
                    "learning_status": "accepted",
                    "confidence": "high",
                    "created_at": "2026-09-06",
                    "summary": "external ledger entry",
                    "stage_experience_path": ""
                }),
            )
            .unwrap();
        let retriever = ExperienceRetriever::new(vec![]).with_external_store(store);
        let (matches, metadata) =
            retriever.retrieve(&ExperienceRetrievalQuery::new("prompt", "evaluate"));
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["experience_id"], "exp-external");
        assert_eq!(matches[0]["summary"], "external ledger entry");
        assert_eq!(metadata["retrieval_status"], "ok");
        assert_eq!(metadata["external_store"], true);
    }
    #[test]
    fn empty_roots_returns_empty_status() {
        let retriever = ExperienceRetriever::new(vec![]);
        let (matches, metadata) = retriever.retrieve(&ExperienceRetrievalQuery::new("p", "s"));
        assert!(matches.is_empty());
        assert_eq!(metadata["retrieval_status"], "empty");
    }

    #[test]
    fn bounded_match_uses_stage_payload() {
        let root = std::env::temp_dir().join(format!("ah-rsi-bm-{}", std::process::id()));
        let stage_dir = root.join("x/stages");
        write_yaml(
            &stage_dir.join("001_s.yaml"),
            &json!({
                "summary": "s",
                "experience": {"general_principles": ["a"], "confidence": "low"},
                "source_artifact_paths": ["/tmp/a"]
            }),
        );
        let stage_abs = stage_dir.join("001_s.yaml");
        let entry = json!({
            "experience_id": "e1",
            "optimization_type": "prompt",
            "role": "leader",
            "stage": "s",
            "learning_status": "accepted",
            "component_layer": "c",
            "stage_experience_path": stage_abs.to_string_lossy().to_string(),
            "summary": "s"
        });
        let view = bounded_match(&entry, 1200);
        assert_eq!(view["experience_id"], "e1");
        assert_eq!(view["experience"]["confidence"], "low");
        assert_eq!(view["source_artifact_paths"][0], "/tmp/a");
        assert_eq!(view["truncated"], false);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn read_structured_handles_json_and_yaml() {
        let root = std::env::temp_dir().join(format!("ah-rsi-rs-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.json"), r#"{"k": [1, 2]}"#).unwrap();
        let v = read_structured(&root.join("a.json"));
        assert_eq!(v["k"][0], 1);
        // 缺失文件 → Null。
        assert_eq!(read_structured(&root.join("missing.yaml")), Value::Null);
        let _ = std::fs::remove_dir_all(&root);
    }
}
