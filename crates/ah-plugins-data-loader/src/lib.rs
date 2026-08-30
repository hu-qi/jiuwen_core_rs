//! # ah-plugins-data-loader
//!
//! Real curriculum-balanced batch planning (aligned with
//! rsi/data_loader/batch_planner.py + profiler.py):
//! - case_value: read balance value from top-level or nested metadata;
//! - BatchPlanner::plan: difficulty progression + dimension round-robin;
//! - batch_plan_item: serializable per-batch plan entry.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use ah_contracts::dataset_curator::{
    BatchPlanCase, BatchPlanEntry, BatchPlanMetadata, UNKNOWN_VALUE,
};
use ah_contracts::keys::DATA_LOADER;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use serde_json::Value;

/// 难度排序(对齐 _DIFFICULTY_ORDER)。
fn difficulty_rank(difficulty: &str) -> usize {
    match difficulty {
        "easy" => 0,
        "medium" => 1,
        "hard" => 2,
        _ => 3,
    }
}

/// 从 case 读取平衡值(对齐 case_value:顶层字段或 metadata 嵌套,空 → unknown)。
pub fn case_value(case: &BTreeMap<String, Value>, key: &str) -> String {
    let top = case.get(key).and_then(non_empty_str);
    if let Some(v) = top {
        return v;
    }
    if let Some(v) = case
        .get("metadata")
        .and_then(Value::as_object)
        .and_then(|meta| meta.get(key).and_then(non_empty_str))
    {
        return v;
    }
    UNKNOWN_VALUE.to_string()
}

fn non_empty_str(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// 稳定 case 标识(对齐 profiler.case_id:case_id 或 id,空 → path 文件名 + case_index)。
pub fn case_id(case: &BTreeMap<String, Value>) -> String {
    for key in ["case_id", "id"] {
        if let Some(v) = case.get(key).and_then(non_empty_str) {
            return v;
        }
    }
    let path = case
        .get("case_path")
        .and_then(Value::as_str)
        .unwrap_or("case");
    let basename = path.rsplit('/').next().unwrap_or(path);
    let stem = match basename.rfind('.') {
        Some(dot) => &basename[..dot],
        None => basename,
    }
    .to_string();
    let index = case.get("case_index").and_then(Value::as_u64).unwrap_or(0);
    format!("{stem}#{index}")
}

/// 课程平衡分批规划器(对齐 BatchPlanner)。
pub struct BatchPlanner;

impl BatchPlanner {
    /// 规划一轮的批次(难度渐进 + 维度轮转;对齐 plan)。
    pub fn plan(
        cases: Vec<BTreeMap<String, Value>>,
        batch_size: usize,
    ) -> Vec<Vec<BTreeMap<String, Value>>> {
        let ordered = curriculum_balanced_cases(cases);
        let mut batches = Vec::new();
        let mut batch = Vec::new();
        for case in ordered {
            batch.push(case);
            if batch.len() >= batch_size {
                batches.push(std::mem::take(&mut batch));
            }
        }
        if !batch.is_empty() {
            batches.push(batch);
        }
        batches
    }
}

/// 生成批次计划条目(对齐 batch_plan_item)。
pub fn batch_plan_item(batch: &[BTreeMap<String, Value>], batch_index: usize) -> BatchPlanEntry {
    let difficulties: Vec<String> = batch.iter().map(|c| case_value(c, "difficulty")).collect();
    let mut dimensions: Vec<String> = batch.iter().map(|c| case_value(c, "dimension")).collect();
    dimensions.sort();
    dimensions.dedup();
    BatchPlanEntry {
        batch_id: format!("batch_{batch_index:03}"),
        cases: batch
            .iter()
            .map(|case| BatchPlanCase {
                case_id: case_id(case),
                difficulty: case_value(case, "difficulty"),
                dimension: case_value(case, "dimension"),
                source: case_value(case, "source"),
                task_type: case_value(case, "task_type"),
                case_path: case
                    .get("case_path")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                case_index: case.get("case_index").and_then(Value::as_u64),
            })
            .collect(),
        metadata: BatchPlanMetadata {
            difficulty_stage: dominant_difficulty(&difficulties),
            dimensions,
        },
    }
}

/// 课程平衡排序(难度升序 + 组内维度轮转;对齐 _curriculum_balanced_cases)。
fn curriculum_balanced_cases(cases: Vec<BTreeMap<String, Value>>) -> Vec<BTreeMap<String, Value>> {
    let mut by_difficulty: BTreeMap<String, Vec<BTreeMap<String, Value>>> = BTreeMap::new();
    for case in cases {
        by_difficulty
            .entry(case_value(&case, "difficulty"))
            .or_default()
            .push(case);
    }
    let mut ordered = Vec::new();
    for group in by_difficulty.into_values() {
        ordered.extend(round_robin_by_dimension(group));
    }
    ordered
}

/// 组内维度轮转(对齐 _round_robin_by_dimension)。
fn round_robin_by_dimension(cases: Vec<BTreeMap<String, Value>>) -> Vec<BTreeMap<String, Value>> {
    let mut grouped: BTreeMap<String, VecDeque<BTreeMap<String, Value>>> = BTreeMap::new();
    let mut sorted = cases;
    sorted.sort_by_key(stable_case_sort_key);
    for case in sorted {
        grouped
            .entry(case_value(&case, "dimension"))
            .or_default()
            .push_back(case);
    }
    let dimensions: Vec<String> = grouped.keys().cloned().collect();
    let mut ordered = Vec::new();
    loop {
        let mut advanced = false;
        for dim in &dimensions {
            if let Some(c) = grouped.get_mut(dim).and_then(VecDeque::pop_front) {
                ordered.push(c);
                advanced = true;
            }
        }
        if !advanced {
            break;
        }
    }
    ordered
}

/// 稳定排序键(对齐 _stable_case_sort_key)。
fn stable_case_sort_key(case: &BTreeMap<String, Value>) -> (String, String, String, u64) {
    let source = case_value(case, "source");
    let task_type = case_value(case, "task_type");
    let cid = case_id(case);
    let index = case.get("case_index").and_then(Value::as_u64).unwrap_or(0);
    (source, task_type, cid, index)
}

/// 主导难度(对齐 _dominant_difficulty:已知难度中最低等级)。
fn dominant_difficulty(difficulties: &[String]) -> String {
    let known: Vec<&String> = difficulties
        .iter()
        .filter(|d| d.as_str() != UNKNOWN_VALUE)
        .collect();
    if known.is_empty() {
        return UNKNOWN_VALUE.to_string();
    }
    known
        .iter()
        .min_by_key(|d| difficulty_rank(d))
        .map(|d| (*d).clone())
        .unwrap_or_else(|| UNKNOWN_VALUE.to_string())
}

/// data-loader 插件:注册分批规划服务。
pub struct DataLoaderPlugin;

impl Plugin for DataLoaderPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-data-loader"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![DATA_LOADER]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let planner = Arc::new(BatchPlanner);
        Ok(vec![ctx.register(DATA_LOADER, planner)])
    }
}

impl Seam for BatchPlanner {}

#[cfg(test)]
mod tests {
    use super::*;

    fn case(
        cid: &str,
        difficulty: &str,
        dimension: &str,
        source: &str,
        index: u64,
    ) -> BTreeMap<String, Value> {
        let mut m = BTreeMap::new();
        m.insert("case_id".to_string(), Value::String(cid.to_string()));
        m.insert(
            "difficulty".to_string(),
            Value::String(difficulty.to_string()),
        );
        m.insert(
            "dimension".to_string(),
            Value::String(dimension.to_string()),
        );
        m.insert("source".to_string(), Value::String(source.to_string()));
        m.insert("task_type".to_string(), Value::String("t".to_string()));
        m.insert("case_index".to_string(), Value::from(index));
        m
    }

    #[test]
    fn case_value_reads_top_or_metadata() {
        let mut m = BTreeMap::new();
        m.insert("difficulty".to_string(), Value::String("easy".to_string()));
        m.insert(
            "metadata".to_string(),
            serde_json::json!({"dimension": "code"}),
        );
        assert_eq!(case_value(&m, "difficulty"), "easy");
        assert_eq!(case_value(&m, "dimension"), "code");
        assert_eq!(case_value(&m, "missing"), UNKNOWN_VALUE);
    }

    #[test]
    fn plan_groups_by_batch_size() {
        let cases = vec![
            case("a", "easy", "code", "s1", 0),
            case("b", "medium", "code", "s1", 1),
            case("c", "hard", "test", "s2", 2),
            case("d", "easy", "test", "s2", 3),
            case("e", "medium", "code", "s1", 4),
        ];
        let batches = BatchPlanner::plan(cases, 2);
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].len(), 2);
        assert_eq!(batches[2].len(), 1);
    }

    #[test]
    fn plan_orders_by_difficulty_then_round_robin_dimension() {
        let cases = vec![
            case("a", "easy", "code", "s", 0),
            case("b", "easy", "test", "s", 1),
            case("c", "easy", "code", "s", 2),
            case("d", "hard", "code", "s", 3),
        ];
        let ordered = curriculum_balanced_cases(cases);
        assert_eq!(case_value(&ordered[0], "difficulty"), "easy");
        assert_eq!(case_value(&ordered[3], "difficulty"), "hard");
        assert_ne!(
            case_value(&ordered[0], "dimension"),
            case_value(&ordered[1], "dimension")
        );
    }

    #[test]
    fn batch_plan_item_produces_serializable_entry() {
        let batch = vec![
            case("x1", "easy", "code", "src", 0),
            case("x2", "medium", "test", "src", 1),
        ];
        let entry = batch_plan_item(&batch, 0);
        assert_eq!(entry.batch_id, "batch_000");
        assert_eq!(entry.cases.len(), 2);
        assert_eq!(entry.cases[0].case_id, "x1");
        assert_eq!(entry.metadata.difficulty_stage, "easy");
        let dims = &entry.metadata.dimensions;
        assert_eq!(dims.len(), 2);
        assert!(dims.contains(&"code".to_string()));
        let js = serde_json::to_string(&entry).unwrap();
        assert!(js.contains("batch_000"));
    }

    #[test]
    fn unknown_difficulty_dominates_to_unknown() {
        let difficulties = vec![UNKNOWN_VALUE.to_string(), UNKNOWN_VALUE.to_string()];
        assert_eq!(dominant_difficulty(&difficulties), UNKNOWN_VALUE);
        let mixed = vec![
            "hard".to_string(),
            "easy".to_string(),
            UNKNOWN_VALUE.to_string(),
        ];
        assert_eq!(dominant_difficulty(&mixed), "easy");
    }
}

/// 数据集画像(对齐 DatasetProfiler.profile 的返回结构)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DatasetProfile {
    pub total_cases: usize,
    /// balance_key -> 值计数(排除 unknown,按键排序)。
    pub summary: BTreeMap<String, BTreeMap<String, usize>>,
    pub warnings: Vec<MissingFieldWarning>,
    pub quality: String,
}

/// 缺失字段警告(对齐 profiler warnings 条目)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MissingFieldWarning {
    pub case_id: String,
    pub missing_fields: Vec<String>,
}

/// 数据集画像器(对齐 DatasetProfiler)。
pub struct DatasetProfiler;

impl DatasetProfiler {
    /// 生成确定性摘要(对齐 profile:balance_keys 计数 + 缺失警告 + quality)。
    pub fn profile(cases: &[BTreeMap<String, Value>], balance_keys: &[String]) -> DatasetProfile {
        let mut summary: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
        for key in balance_keys {
            let mut counter: BTreeMap<String, usize> = BTreeMap::new();
            for case in cases {
                let v = case_value(case, key);
                if v != UNKNOWN_VALUE {
                    *counter.entry(v).or_insert(0) += 1;
                }
            }
            summary.insert(key.clone(), counter);
        }
        let mut warnings = Vec::new();
        for case in cases {
            let missing: Vec<String> = balance_keys
                .iter()
                .filter(|k| case_value(case, k) == UNKNOWN_VALUE)
                .cloned()
                .collect();
            if !missing.is_empty() {
                warnings.push(MissingFieldWarning {
                    case_id: case_id(case),
                    missing_fields: missing,
                });
            }
        }
        let total = cases.len();
        let quality = profile_quality(total, warnings.len());
        DatasetProfile {
            total_cases: total,
            summary,
            warnings,
            quality,
        }
    }
}

/// 画像质量分级(对齐 _profile_quality)。
fn profile_quality(total_cases: usize, warning_count: usize) -> String {
    if total_cases == 0 {
        return "empty".to_string();
    }
    if warning_count == 0 {
        return "normal".to_string();
    }
    if warning_count as f64 / total_cases as f64 >= 0.5 {
        return "low_quality_fallback".to_string();
    }
    "partial_metadata".to_string()
}
#[cfg(test)]
mod tests2 {
    use super::*;

    fn case(cid: &str, difficulty: &str, dimension: &str) -> BTreeMap<String, Value> {
        let mut m = BTreeMap::new();
        m.insert("case_id".to_string(), Value::String(cid.to_string()));
        m.insert(
            "difficulty".to_string(),
            Value::String(difficulty.to_string()),
        );
        m.insert(
            "dimension".to_string(),
            Value::String(dimension.to_string()),
        );
        m
    }

    #[test]
    fn profile_counts_balance_keys_and_omits_unknown() {
        let cases = vec![
            case("a", "easy", "code"),
            case("b", "easy", "test"),
            case("c", "hard", "code"),
        ];
        let keys = vec!["difficulty".to_string(), "dimension".to_string()];
        let profile = DatasetProfiler::profile(&cases, &keys);
        assert_eq!(profile.total_cases, 3);
        let diff = profile.summary.get("difficulty").unwrap();
        assert_eq!(diff.get("easy"), Some(&2));
        assert_eq!(diff.get("hard"), Some(&1));
        assert_eq!(profile.quality, "normal");
    }

    #[test]
    fn profile_warns_on_missing_fields_and_grades_quality() {
        let mut c1 = BTreeMap::new();
        c1.insert("case_id".to_string(), Value::String("x1".to_string()));
        c1.insert("difficulty".to_string(), Value::String("easy".to_string()));
        let cases = vec![c1];
        let keys = vec!["difficulty".to_string(), "dimension".to_string()];
        let profile = DatasetProfiler::profile(&cases, &keys);
        assert_eq!(profile.warnings.len(), 1);
        assert_eq!(profile.warnings[0].case_id, "x1");
        assert_eq!(
            profile.warnings[0].missing_fields,
            vec!["dimension".to_string()]
        );
        // 1/1 = 100% missing -> low_quality_fallback (matches Python threshold >= 0.5)
        assert_eq!(profile.quality, "low_quality_fallback");
    }

    #[test]
    fn profile_quality_low_fallback_when_most_missing() {
        let mut c1 = BTreeMap::new();
        c1.insert("case_id".to_string(), Value::String("x1".to_string()));
        let mut c2 = BTreeMap::new();
        c2.insert("case_id".to_string(), Value::String("x2".to_string()));
        let cases = vec![c1, c2];
        let keys = vec!["dimension".to_string()];
        let profile = DatasetProfiler::profile(&cases, &keys);
        assert_eq!(profile.warnings.len(), 2);
        assert_eq!(profile.quality, "low_quality_fallback");
    }

    #[test]
    fn profile_empty_cases_is_empty_quality() {
        let profile = DatasetProfiler::profile(&[], &["dimension".to_string()]);
        assert_eq!(profile.total_cases, 0);
        assert_eq!(profile.quality, "empty");
    }

    #[test]
    fn case_id_falls_back_to_path_stem_and_index() {
        let mut m = BTreeMap::new();
        m.insert(
            "case_path".to_string(),
            Value::String("/data/bench.json".to_string()),
        );
        m.insert("case_index".to_string(), Value::from(3));
        assert_eq!(case_id(&m), "bench#3");
        // explicit case_id wins
        m.insert("case_id".to_string(), Value::String("custom".to_string()));
        assert_eq!(case_id(&m), "custom");
    }
}

/// 解析 JSON 数据集文件内容为 cases 列表(对齐 _load_json_cases 的解析逻辑)。
/// 支持:单个 case 对象 / case 列表 / {"cases": [...]}。
pub fn parse_json_cases(data: &Value) -> Result<Vec<BTreeMap<String, Value>>, String> {
    match data {
        Value::Object(map) => {
            if let Some(cases) = map.get("cases") {
                match cases.as_array() {
                    Some(arr) => arr
                        .iter()
                        .enumerate()
                        .map(|(i, c)| {
                            c.as_object()
                                .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                                .ok_or_else(|| format!("dataset case must be a mapping: #{i}"))
                        })
                        .collect(),
                    None => Err("dataset cases must be a list".to_string()),
                }
            } else {
                Ok(vec![
                    map.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                ])
            }
        }
        Value::Array(arr) => arr
            .iter()
            .enumerate()
            .map(|(i, c)| {
                c.as_object()
                    .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                    .ok_or_else(|| format!("dataset case must be a mapping: #{i}"))
            })
            .collect(),
        _ => Err("dataset json must be a case object, case list, or object with cases".to_string()),
    }
}

/// 组装批次计划 payload(对齐 BatchPlanStore.write_batch_plan 的 payload dict)。
pub fn batch_plan_payload(
    dataset_dir: &str,
    epoch: usize,
    batch_size: usize,
    balance_keys: &[String],
    profile: &DatasetProfile,
    batches: &[Vec<BTreeMap<String, Value>>],
) -> serde_json::Value {
    let plan_entries: Vec<serde_json::Value> = batches
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let entry = batch_plan_item(b, i + 1);
            serde_json::to_value(entry).unwrap_or(serde_json::Value::Null)
        })
        .collect();
    let dir_name = dataset_dir.rsplit('/').next().unwrap_or(dataset_dir);
    serde_json::json!({
        "plan_id": format!("batch_plan_epoch_{epoch:03}"),
        "dataset_dir": dataset_dir,
        "strategy": "curriculum_balanced",
        "epoch": epoch,
        "seed": format!("{dir_name}:epoch_{epoch:03}"),
        "batch_size": batch_size,
        "balance_keys": balance_keys,
        "profile_summary": profile.summary,
        "batches": plan_entries,
        "warnings": profile.warnings,
        "metadata": { "quality": profile.quality },
    })
}

#[cfg(test)]
mod tests3 {
    use super::*;

    fn case(cid: &str, difficulty: &str, dimension: &str) -> BTreeMap<String, Value> {
        let mut m = BTreeMap::new();
        m.insert("case_id".to_string(), Value::String(cid.to_string()));
        m.insert(
            "difficulty".to_string(),
            Value::String(difficulty.to_string()),
        );
        m.insert(
            "dimension".to_string(),
            Value::String(dimension.to_string()),
        );
        m
    }

    #[test]
    fn parse_json_cases_handles_all_shapes() {
        // single object
        let single = serde_json::json!({"case_id": "a", "difficulty": "easy"});
        let cases = parse_json_cases(&single).unwrap();
        assert_eq!(cases.len(), 1);
        // list
        let list = serde_json::json!([{"case_id": "a"}, {"case_id": "b"}]);
        assert_eq!(parse_json_cases(&list).unwrap().len(), 2);
        // cases key
        let wrapped = serde_json::json!({"cases": [{"case_id": "a"}]});
        assert_eq!(parse_json_cases(&wrapped).unwrap().len(), 1);
        // invalid
        assert!(parse_json_cases(&serde_json::json!(42)).is_err());
        assert!(parse_json_cases(&serde_json::json!({"cases": 42})).is_err());
    }

    #[test]
    fn batch_plan_payload_assembles_full_plan() {
        let batch = vec![case("x1", "easy", "code"), case("x2", "medium", "test")];
        let batches = vec![batch];
        let profile = DatasetProfiler::profile(&batches[0], &["difficulty".to_string()]);
        let payload = batch_plan_payload(
            "/data/ds",
            1,
            2,
            &["difficulty".to_string()],
            &profile,
            &batches,
        );
        assert_eq!(payload["plan_id"], "batch_plan_epoch_001");
        assert_eq!(payload["strategy"], "curriculum_balanced");
        assert_eq!(payload["epoch"], 1);
        assert_eq!(payload["batch_size"], 2);
        assert!(payload["seed"].as_str().unwrap().contains("ds:epoch_001"));
        assert_eq!(payload["batches"].as_array().unwrap().len(), 1);
        assert_eq!(payload["batches"][0]["batch_id"], "batch_001");
        assert_eq!(payload["metadata"]["quality"], "normal");
    }
}

/// 批次计划落盘(对齐 `plan_store.BatchPlanStore`)。
pub struct BatchPlanStore;

impl BatchPlanStore {
    /// 写入 `dataset_profile.yaml` 并返回绝对路径(对齐 write_dataset_profile)。
    pub fn write_dataset_profile(
        root: &std::path::Path,
        profile: &DatasetProfile,
    ) -> Result<String, String> {
        let profile_path = root.join("dataset_profile.yaml");
        let value = serde_json::to_value(profile).map_err(|e| e.to_string())?;
        let yaml_text = json_to_yaml(&value)?;
        std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
        std::fs::write(&profile_path, yaml_text).map_err(|e| e.to_string())?;
        Ok(profile_path.to_string_lossy().to_string())
    }

    /// 写入 `batch_plan.yaml` 并返回绝对路径(对齐 write_batch_plan)。
    pub fn write_batch_plan(
        root: &std::path::Path,
        epoch: usize,
        batch_size: usize,
        balance_keys: &[String],
        profile: &DatasetProfile,
        batches: &[Vec<BTreeMap<String, Value>>],
    ) -> Result<String, String> {
        let plan_path = root.join("batch_plan.yaml");
        let payload = batch_plan_payload(
            &root.to_string_lossy(),
            epoch,
            batch_size,
            balance_keys,
            profile,
            batches,
        );
        let yaml_text = json_to_yaml(&payload)?;
        std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
        std::fs::write(&plan_path, yaml_text).map_err(|e| e.to_string())?;
        Ok(plan_path.to_string_lossy().to_string())
    }
}

/// JSON Value → YAML 文本(对齐 yaml.safe_dump,allow_unicode/sort_keys=False)。
pub fn json_to_yaml(value: &serde_json::Value) -> Result<String, String> {
    let yaml_value: serde_yaml::Value = serde_yaml::to_value(value).map_err(|e| e.to_string())?;
    serde_yaml::to_string(&yaml_value).map_err(|e| e.to_string())
}

#[cfg(test)]
mod store_tests {
    use super::*;

    #[test]
    fn batch_plan_store_writes_yaml_files() {
        let root = std::env::temp_dir().join(format!("ah-bps-{}", std::process::id()));
        let profile = DatasetProfile {
            total_cases: 2,
            summary: BTreeMap::from([(
                "difficulty".to_string(),
                BTreeMap::from([("easy".to_string(), 2)]),
            )]),
            warnings: Vec::new(),
            quality: "normal".to_string(),
        };
        let cases: Vec<BTreeMap<String, Value>> = vec![
            BTreeMap::from([("case_id".to_string(), Value::String("c1".into()))]),
            BTreeMap::from([("case_id".to_string(), Value::String("c2".into()))]),
        ];
        let batches = vec![cases];
        let profile_path = BatchPlanStore::write_dataset_profile(&root, &profile).expect("profile");
        let plan_path = BatchPlanStore::write_batch_plan(
            &root,
            1,
            2,
            &["difficulty".to_string()],
            &profile,
            &batches,
        )
        .expect("plan");

        // 文件真实落盘。
        assert!(std::path::Path::new(&profile_path).is_file());
        assert!(std::path::Path::new(&plan_path).is_file());
        let plan_text = std::fs::read_to_string(&plan_path).expect("read");
        assert!(plan_text.contains("plan_id: batch_plan_epoch_001"));
        assert!(plan_text.contains("strategy: curriculum_balanced"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn json_to_yaml_serializes_mapping() {
        let value = serde_json::json!({"a": 1, "b": ["x", "y"]});
        let yaml = json_to_yaml(&value).expect("yaml");
        assert!(yaml.contains("a: 1"));
        assert!(yaml.contains("b:"));
    }
}
