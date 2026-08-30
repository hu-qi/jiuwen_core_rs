//! # ah-plugins-dataset-curator
//!
//! Real replay-dataset curation (aligned with openjiuwen/rsi/dataset_curator/curator.py):
//! mines failed, judgeable evaluation cases into a replay dataset:
//! - reads eval_ref (YAML/JSON, cases list); loads original cases by case_id/index;
//! - per-case decision: missing original / inconclusive / passed threshold / not judgeable
//!   -> rejected; else accepted with replay_ prefix + provenance metadata;
//! - outputs replay_cases.json, targeted_dataset_seed.json, curation_report.yaml.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ah_contracts::dataset_curator::{
    CurationError, DatasetCurationArtifact, DatasetCurationConfig, DatasetCurator,
};
use ah_contracts::keys::DATASET_CURATOR;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use serde_json::{Value, json};

fn str_of(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// Read eval_ref (YAML parses JSON too; YAML 1.2 is a superset).
pub fn read_eval_ref(path: &str) -> Result<Value, CurationError> {
    let text = fs::read_to_string(path)
        .map_err(|e| CurationError(format!("read eval_ref failed: {e}")))?;
    serde_yaml::from_str(&text).map_err(|e| CurationError(format!("parse eval_ref failed: {e}")))
}

/// Read a JSON mapping (missing/unparsable -> Null).
pub fn read_json_mapping(path: &str) -> Value {
    let Ok(text) = fs::read_to_string(path) else {
        return Value::Null;
    };
    serde_json::from_str::<Value>(&text).unwrap_or(Value::Null)
}

/// Case refs: dict entries under eval_ref.cases.
pub fn case_refs(eval_ref: &Value) -> Vec<Value> {
    eval_ref
        .get("cases")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter(|c| c.is_object()).cloned().collect())
        .unwrap_or_default()
}

/// Failure check: score field -> result_path.score -> evaluation.passed -> status != passed.
pub fn case_failed(case_ref: &Value, score_threshold: f64) -> bool {
    if let Some(n) = case_ref.get("score").and_then(Value::as_f64) {
        return n < score_threshold;
    }
    let result = read_json_mapping(&str_of(case_ref, "result_path"));
    if let Some(n) = result.get("score").and_then(Value::as_f64) {
        return n < score_threshold;
    }
    if let Some(passed) = result
        .get("evaluation")
        .filter(|e| e.is_object())
        .and_then(|e| e.get("passed"))
    {
        return !passed.as_bool().unwrap_or(false);
    }
    str_of(case_ref, "status") != "passed"
}

/// Inconclusive check: status / result.status / evaluation.method == error.
pub fn case_inconclusive(case_ref: &Value) -> bool {
    let status = str_of(case_ref, "status").to_lowercase();
    let result = read_json_mapping(&str_of(case_ref, "result_path"));
    let result_status = str_of(&result, "status").to_lowercase();
    let method = result
        .get("evaluation")
        .and_then(|e| e.get("method"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase();
    status == "error" || result_status == "error" || method == "error"
}

/// Judgeable check: verification_contract/evaluation_adapter/reference.answer/required_behaviors/expected/assertions/verifier.
pub fn is_judgeable(case: &Value) -> bool {
    if case.get("verification_contract").is_some() || case.get("evaluation_adapter").is_some() {
        return true;
    }
    if let Some(reference) = case.get("reference").filter(|r| r.is_object()) {
        if let Some(answer) = reference
            .get("answer")
            .filter(|a| !(a.is_null() || a.as_str().map(str::is_empty).unwrap_or(false)))
        {
            let _ = answer;
            return true;
        }
        if let Some(behaviors) = reference
            .get("required_behaviors")
            .and_then(Value::as_array)
            .filter(|b| !b.is_empty())
        {
            let _ = behaviors;
            return true;
        }
    }
    if let Some(expected) = case
        .get("expected")
        .filter(|e| !(e.is_null() || e.as_str().map(str::is_empty).unwrap_or(false)))
    {
        let _ = expected;
        return true;
    }
    if case.get("assertions").is_some() || case.get("verifier").is_some() {
        return true;
    }
    false
}

/// Load original case from case_path JSON by case_id or 1-based case_index.
pub fn load_original_case(case_ref: &Value) -> Value {
    let case_path = str_of(case_ref, "case_path");
    if case_path.is_empty() {
        return Value::Null;
    }
    let Ok(text) = fs::read_to_string(Path::new(&case_path)) else {
        return Value::Null;
    };
    let Ok(data) = serde_json::from_str::<Value>(&text) else {
        return Value::Null;
    };
    let cases: Vec<Value> = if let Some(list) = data.as_array() {
        list.clone()
    } else if let Some(list) = data.get("cases").and_then(Value::as_array) {
        list.clone()
    } else if data.is_object() {
        vec![data.clone()]
    } else {
        Vec::new()
    };
    let case_id = str_of(case_ref, "case_id");
    if !case_id.is_empty()
        && let Some(case) = cases.iter().find(|c| str_of(c, "case_id") == case_id)
    {
        return case.clone();
    }
    if let Some(index) = case_ref.get("case_index").and_then(Value::as_u64)
        && index >= 1
        && index as usize <= cases.len()
        && let Some(case) = cases.get(index as usize - 1)
    {
        return case.clone();
    }
    Value::Null
}
fn string_list(raw: Option<&Value>) -> Vec<String> {
    raw.and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .map(|v| v.as_str().map(|s| s.trim().to_string()).unwrap_or_default())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn bounded_text(text: &str, limit: usize) -> String {
    let value = text.trim();
    if value.chars().count() <= limit {
        value.to_string()
    } else {
        format!("{}...", value.chars().take(limit - 3).collect::<String>())
    }
}

/// Trace evidence: {trace_path, excerpt} (JSON dump truncated to 3000 chars).
pub fn trace_evidence(trace_path: &str) -> Value {
    let path_text = trace_path.trim();
    if path_text.is_empty() {
        return json!({ "trace_path": "", "excerpt": "" });
    }
    let path = Path::new(path_text);
    if !path.is_file() {
        return json!({ "trace_path": path_text, "excerpt": "" });
    }
    let excerpt = match fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(payload) => serde_json::to_string(&payload).unwrap_or_default(),
            Err(_) => text,
        },
        Err(_) => String::new(),
    };
    json!({
        "trace_path": path_text,
        "excerpt": bounded_text(&excerpt, 3000),
    })
}

/// Root-cause capabilities (behavior score < 0.8; fallback case_failed).
pub fn root_cause_capabilities(
    behavior_results: &[Value],
    target_capabilities: &[String],
    capability_gap: &str,
) -> Vec<Value> {
    let mut root_causes: Vec<Value> = Vec::new();
    for item in behavior_results {
        if !item.is_object() {
            continue;
        }
        if let Some(score) = item.get("score").and_then(Value::as_f64)
            && score >= 0.8
        {
            continue;
        }
        let missing_capability = str_of(item, "missing_capability");
        let failure_reason = str_of(item, "failure_reason");
        let evidence = str_of(item, "evidence");
        if missing_capability.is_empty() && failure_reason.is_empty() && evidence.is_empty() {
            continue;
        }
        let capability_name = if missing_capability.is_empty() {
            target_capabilities
                .first()
                .cloned()
                .unwrap_or_else(|| "unknown".to_string())
        } else {
            missing_capability
        };
        let failure_type = if failure_reason.is_empty() {
            "low_scored_behavior".to_string()
        } else {
            failure_reason.clone()
        };
        root_causes.push(json!({
            "capability_name": capability_name,
            "failure_type": failure_type,
            "evidence_from_trace": evidence,
            "why_it_caused_failure": if failure_reason.is_empty() { capability_gap } else { &failure_reason },
            "data_needed_to_fix": capability_gap,
        }));
    }
    if !root_causes.is_empty() {
        return root_causes;
    }
    vec![json!({
        "capability_name": target_capabilities.first().cloned().unwrap_or_else(|| "unknown".to_string()),
        "failure_type": "case_failed",
        "evidence_from_trace": "",
        "why_it_caused_failure": capability_gap,
        "data_needed_to_fix": capability_gap,
    })]
}

fn difficulty_level(metadata: &Value) -> i64 {
    let difficulty = metadata
        .get("difficulty")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase();
    match difficulty.as_str() {
        "easy" => 2,
        "hard" => 4,
        _ => 3, // medium and unknown both 3
    }
}

/// Targeted seed task (built when a training signal is present; else Null).
pub fn build_targeted_seed_task(
    eval_ref_path: &str,
    case_ref: &Value,
    original_case: &Value,
) -> Value {
    let training_signal = original_case
        .get("training_signal")
        .filter(|t| t.is_object());
    let Some(training_signal) =
        training_signal.filter(|t| !t.as_object().map(|m| m.is_empty()).unwrap_or(true))
    else {
        return Value::Null;
    };
    let result = read_json_mapping(&str_of(case_ref, "result_path"));
    let evaluation = result
        .get("evaluation")
        .filter(|e| e.is_object())
        .cloned()
        .unwrap_or(Value::Null);
    let behavior_results = evaluation
        .get("behavior_results")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let source_case_id = {
        let id = str_of(case_ref, "case_id");
        if id.is_empty() {
            str_of(original_case, "case_id")
        } else {
            id
        }
    };
    let reference = original_case
        .get("reference")
        .filter(|r| r.is_object())
        .cloned()
        .unwrap_or(Value::Null);
    let mut success_criteria = string_list(reference.get("success_criteria"));
    if success_criteria.is_empty() {
        success_criteria = reference
            .get("required_behaviors")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter(|b| b.is_object())
                    .map(|b| str_of(b, "description"))
                    .filter(|d| !d.is_empty())
                    .collect()
            })
            .unwrap_or_default();
    }
    let expected_failure_modes = string_list(training_signal.get("expected_failure_modes"));
    let capability_gap = str_of(training_signal, "capability_gap");
    let target_capabilities = string_list(training_signal.get("target_capabilities"));
    let root_causes =
        root_cause_capabilities(&behavior_results, &target_capabilities, &capability_gap);
    let task_pattern = if let Some(input) = original_case.get("input").filter(|i| i.is_object()) {
        let user_message = str_of(input, "user_message");
        if user_message.is_empty() {
            str_of(input, "query")
        } else {
            user_message
        }
    } else {
        let input_text = str_of(original_case, "input");
        if input_text.is_empty() {
            str_of(original_case, "query")
        } else {
            input_text
        }
    };
    let metadata = original_case
        .get("metadata")
        .cloned()
        .unwrap_or(Value::Null);
    let failure_summary = {
        let reason = str_of(&evaluation, "reason");
        if reason.is_empty() {
            let error = str_of(&result, "error");
            if error.is_empty() {
                let status = str_of(&result, "status");
                if status.is_empty() {
                    "case failed".to_string()
                } else {
                    status
                }
            } else {
                error
            }
        } else {
            reason
        }
    };
    json!({
        "source_case_id": source_case_id,
        "source_eval_ref_path": eval_ref_path,
        "result_path": str_of(case_ref, "result_path"),
        "trace_path": str_of(case_ref, "trace_path"),
        "task_pattern": task_pattern,
        "difficulty_level": difficulty_level(&metadata),
        "target_capabilities": target_capabilities,
        "capability_combination": str_of(training_signal, "capability_combination"),
        "target_surfaces": string_list(training_signal.get("target_surfaces")),
        "specific_trap_to_include": if expected_failure_modes.is_empty() { capability_gap.clone() } else { expected_failure_modes[0].clone() },
        "success_criteria": success_criteria,
        "failure_summary": failure_summary,
        "trace_evidence": trace_evidence(&str_of(case_ref, "trace_path")),
        "root_cause_capabilities": root_causes,
        "generation_reason": capability_gap,
    })
}
/// Report payload (aligned with _report_payload).
pub fn report_payload(
    status: &str,
    eval_ref_path: &str,
    accepted_cases: &[Value],
    rejected_cases: &[Value],
    dataset_file: &str,
    targeted_dataset_seed_file: &str,
) -> Value {
    let accepted_meta: Vec<Value> = accepted_cases
        .iter()
        .map(|case| {
            let provenance = case
                .get("metadata")
                .and_then(|m| m.get("provenance"))
                .cloned()
                .unwrap_or(Value::Null);
            json!({
                "case_id": case.get("case_id").cloned().unwrap_or(Value::Null),
                "source_case_id": str_of(&provenance, "source_case_id"),
            })
        })
        .collect();
    json!({
        "status": status,
        "source_eval_ref_path": eval_ref_path,
        "dataset_file": dataset_file,
        "targeted_dataset_seed_file": targeted_dataset_seed_file,
        "summary": {
            "candidate_cases": accepted_cases.len() + rejected_cases.len(),
            "accepted_cases": accepted_cases.len(),
            "rejected_cases": rejected_cases.len(),
        },
        "accepted_cases": accepted_meta,
        "rejected_cases": rejected_cases,
    })
}

/// Per-case decision (aligned with _curate_case).
pub fn curate_case(
    config: &DatasetCurationConfig,
    eval_ref_path: &str,
    case_ref: &Value,
    original_case: &Value,
) -> Value {
    if original_case.is_null() {
        return json!({ "status": "rejected", "reason": "missing_original_case" });
    }
    if case_inconclusive(case_ref) {
        return json!({ "status": "rejected", "reason": "case_result_inconclusive" });
    }
    if !case_failed(case_ref, config.score_threshold) {
        return json!({ "status": "rejected", "reason": "case_passed_threshold" });
    }
    if config.require_judgeable_reference && !is_judgeable(original_case) {
        return json!({ "status": "rejected", "reason": "missing_judgeable_reference" });
    }
    let source_case_id = {
        let id = str_of(case_ref, "case_id");
        if id.is_empty() {
            str_of(original_case, "case_id")
        } else {
            id
        }
    };
    let source_case_id = if source_case_id.is_empty() {
        "case".to_string()
    } else {
        source_case_id
    };
    let mut replay_case = original_case.clone();
    replay_case["case_id"] = json!(format!("replay_{source_case_id}"));
    let mut metadata = replay_case.get("metadata").cloned().unwrap_or(json!({}));
    if let Some(map) = metadata.as_object_mut() {
        map.insert("source".to_string(), json!(config.source_label));
        map.insert("synthetic".to_string(), json!(false));
        map.insert("judgeable".to_string(), json!(is_judgeable(original_case)));
        map.insert(
            "provenance".to_string(),
            json!({
                "source_case_id": source_case_id,
                "source_eval_ref_path": eval_ref_path,
                "source_case_path": str_of(case_ref, "case_path"),
                "source_case_index": case_ref.get("case_index").cloned().unwrap_or(Value::Null),
                "result_path": str_of(case_ref, "result_path"),
                "trace_path": str_of(case_ref, "trace_path"),
                "score": case_ref.get("score").cloned().unwrap_or(Value::Null),
                "status": str_of(case_ref, "status"),
            }),
        );
    }
    replay_case["metadata"] = metadata;
    replay_case["source"] = json!(config.source_label);
    json!({ "status": "accepted", "case": replay_case })
}

/// Real dataset curator implementation.
pub struct DatasetCuratorImpl;

impl Seam for DatasetCuratorImpl {}

impl DatasetCurator for DatasetCuratorImpl {
    fn curate(
        &self,
        config: &DatasetCurationConfig,
        eval_ref_path: &str,
        output_dir: &str,
    ) -> Result<DatasetCurationArtifact, CurationError> {
        let output_root = PathBuf::from(output_dir);
        fs::create_dir_all(&output_root)
            .map_err(|e| CurationError(format!("create output dir failed: {e}")))?;
        let report_path = output_root.join(&config.report_filename);
        let eval_ref = read_eval_ref(eval_ref_path)?;
        if !config.enabled {
            let payload = report_payload("disabled", eval_ref_path, &[], &[], "", "");
            write_yaml(&report_path, &payload)?;
            return Ok(DatasetCurationArtifact {
                status: "disabled".to_string(),
                eval_ref_path: eval_ref_path.to_string(),
                output_dir: output_root.display().to_string(),
                dataset_file: String::new(),
                targeted_seed_file: String::new(),
                report_path: report_path.display().to_string(),
                accepted_cases: 0,
                rejected_cases: 0,
            });
        }
        let mut accepted: Vec<Value> = Vec::new();
        let mut rejected: Vec<Value> = Vec::new();
        let mut targeted: Vec<Value> = Vec::new();
        for case_ref in case_refs(&eval_ref) {
            let original = load_original_case(&case_ref);
            let decision = curate_case(config, eval_ref_path, &case_ref, &original);
            if str_of(&decision, "status") == "accepted" {
                let case = decision.get("case").cloned().unwrap_or(Value::Null);
                let seed = build_targeted_seed_task(eval_ref_path, &case_ref, &original);
                if !seed.is_null() {
                    targeted.push(seed);
                }
                accepted.push(case);
            } else {
                rejected.push(json!({
                    "case_id": str_of(&case_ref, "case_id"),
                    "reason": str_of(&decision, "reason"),
                }));
            }
        }
        let mut dataset_file = String::new();
        if !accepted.is_empty() {
            let dataset_path = output_root.join(&config.output_filename);
            let payload = json!({
                "dataset_id": output_root.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
                "created_at": "",
                "source": config.source_label,
                "cases": accepted,
            });
            write_json(&dataset_path, &payload)?;
            dataset_file = dataset_path.display().to_string();
        }
        let mut targeted_seed_file = String::new();
        if !targeted.is_empty() {
            let seed_path = output_root.join(&config.targeted_seed_filename);
            let payload = json!({
                "dataset_id": format!("{}_targeted_seed", output_root.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()),
                "created_at": "",
                "source": format!("{}_targeted_seed", config.source_label),
                "source_eval_ref_path": eval_ref_path,
                "recommended_synthetic_tasks": targeted,
            });
            write_json(&seed_path, &payload)?;
            targeted_seed_file = seed_path.display().to_string();
        }
        let report = report_payload(
            "completed",
            eval_ref_path,
            &accepted,
            &rejected,
            &dataset_file,
            &targeted_seed_file,
        );
        write_yaml(&report_path, &report)?;
        Ok(DatasetCurationArtifact {
            status: "completed".to_string(),
            eval_ref_path: eval_ref_path.to_string(),
            output_dir: output_root.display().to_string(),
            dataset_file,
            targeted_seed_file,
            report_path: report_path.display().to_string(),
            accepted_cases: accepted.len(),
            rejected_cases: rejected.len(),
        })
    }
}

fn write_json(path: &Path, payload: &Value) -> Result<(), CurationError> {
    let text = serde_json::to_string_pretty(payload)
        .map_err(|e| CurationError(format!("serialize json failed: {e}")))?;
    fs::write(path, text).map_err(|e| CurationError(format!("write json failed: {e}")))
}

fn write_yaml(path: &Path, payload: &Value) -> Result<(), CurationError> {
    let text = serde_yaml::to_string(payload)
        .map_err(|e| CurationError(format!("serialize yaml failed: {e}")))?;
    fs::write(path, text).map_err(|e| CurationError(format!("write yaml failed: {e}")))
}

/// dataset-curator plugin: registers the `dataset-curator` seam.
pub struct DatasetCuratorPlugin;

impl Plugin for DatasetCuratorPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-dataset-curator"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![DATASET_CURATOR]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let curator: Arc<dyn DatasetCurator> = Arc::new(DatasetCuratorImpl);
        Ok(vec![ctx.register(DATASET_CURATOR, curator)])
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::DATASET_CURATOR;
    use ah_hub::plugin::DynPlugin;
    use std::fs;
    use std::path::PathBuf;

    fn temp_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("ah-curator-{tag}-{}", std::process::id()))
    }

    /// 建立 fixture:eval_ref + case/result/trace 文件,返回 (dir, eval_ref_path)。
    fn make_fixtures(tag: &str) -> (PathBuf, String) {
        let dir = temp_dir(tag);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("case-1.json"),
            serde_json::to_string(&json!({
                "case_id": "case-1",
                "query": "solve the puzzle",
                "reference": { "answer": "42" },
                "training_signal": {
                    "capability_gap": "planning",
                    "target_capabilities": ["planning", "execution"],
                    "expected_failure_modes": ["premature_commit"],
                    "target_surfaces": ["prompt"],
                },
                "metadata": { "difficulty": "hard" },
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            dir.join("case-2.json"),
            serde_json::to_string(
                &json!({ "case_id": "case-2", "reference": { "answer": "yes" } }),
            )
            .unwrap(),
        )
        .unwrap();
        fs::write(
            dir.join("case-3.json"),
            serde_json::to_string(&json!({ "case_id": "case-3" })).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.join("result-1.json"),
            serde_json::to_string(&json!({
                "score": 0.4,
                "evaluation": { "method": "heuristic", "behavior_results": [
                    { "score": 0.3, "missing_capability": "planning", "failure_reason": "no plan", "evidence": "trace-1" },
                    { "score": 0.9 },
                ] },
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            dir.join("result-2.json"),
            serde_json::to_string(&json!({ "score": 1.0 })).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.join("result-3.json"),
            serde_json::to_string(&json!({ "status": "error" })).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.join("trace-1.json"),
            serde_json::to_string(&json!({ "steps": ["a", "b"] })).unwrap(),
        )
        .unwrap();
        let eval_ref = json!({
            "cases": [
                { "case_id": "case-1", "case_path": dir.join("case-1.json"), "result_path": dir.join("result-1.json"), "trace_path": dir.join("trace-1.json"), "score": 0.4, "status": "failed" },
                { "case_id": "case-2", "case_path": dir.join("case-2.json"), "result_path": dir.join("result-2.json"), "score": 1.0, "status": "passed" },
                { "case_id": "case-3", "case_path": dir.join("case-3.json"), "result_path": dir.join("result-3.json"), "status": "error" },
            ],
        });
        let eval_ref_path = dir.join("eval_ref.json");
        fs::write(
            &eval_ref_path,
            serde_json::to_string_pretty(&eval_ref).unwrap(),
        )
        .unwrap();
        (dir, eval_ref_path.display().to_string())
    }

    #[test]
    fn curate_accepts_failed_judgeable_and_writes_outputs() {
        let (dir, eval_ref) = make_fixtures("full");
        let out = dir.join("out");
        let curator = DatasetCuratorImpl;
        let artifact = curator
            .curate(
                &DatasetCurationConfig::default(),
                &eval_ref,
                out.to_str().unwrap(),
            )
            .expect("curate");
        assert_eq!(artifact.status, "completed");
        assert_eq!(artifact.accepted_cases, 1, "case-1 接受");
        assert_eq!(artifact.rejected_cases, 2, "case-2 过线 + case-3 error");
        let dataset: Value =
            serde_json::from_str(&fs::read_to_string(out.join("replay_cases.json")).unwrap())
                .unwrap();
        let cases = dataset["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 1);
        assert_eq!(cases[0]["case_id"], "replay_case-1");
        assert_eq!(cases[0]["metadata"]["synthetic"], false);
        assert_eq!(cases[0]["metadata"]["judgeable"], true);
        assert_eq!(
            cases[0]["metadata"]["provenance"]["source_case_id"],
            "case-1"
        );
        assert_eq!(cases[0]["metadata"]["provenance"]["score"], 0.4);
        let seed: Value = serde_json::from_str(
            &fs::read_to_string(out.join("targeted_dataset_seed.json")).unwrap(),
        )
        .unwrap();
        let tasks = seed["recommended_synthetic_tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["difficulty_level"], 4, "hard → 4");
        assert_eq!(tasks[0]["task_pattern"], "solve the puzzle");
        assert_eq!(tasks[0]["specific_trap_to_include"], "premature_commit");
        assert!(
            !tasks[0]["root_cause_capabilities"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let report_text = fs::read_to_string(out.join("curation_report.yaml")).unwrap();
        assert!(report_text.contains("status: completed"));
        assert!(report_text.contains("accepted_cases: 1"));
        assert!(report_text.contains("rejected_cases: 2"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn curate_disabled_writes_disabled_report() {
        let (dir, eval_ref) = make_fixtures("disabled");
        let out = dir.join("out");
        let config = DatasetCurationConfig {
            enabled: false,
            ..Default::default()
        };
        let curator = DatasetCuratorImpl;
        let artifact = curator
            .curate(&config, &eval_ref, out.to_str().unwrap())
            .expect("curate");
        assert_eq!(artifact.status, "disabled");
        assert!(artifact.dataset_file.is_empty());
        assert!(out.join("curation_report.yaml").exists(), "报告仍写入");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn curate_case_decision_paths() {
        let config = DatasetCurationConfig::default();
        let d1 = curate_case(&config, "ev", &json!({}), &Value::Null);
        assert_eq!(d1["reason"], "missing_original_case");
        let err_case = json!({ "status": "error" });
        let d2 = curate_case(
            &config,
            "ev",
            &err_case,
            &json!({ "reference": { "answer": "x" } }),
        );
        assert_eq!(d2["reason"], "case_result_inconclusive");
        let passed = json!({ "score": 1.0 });
        let d3 = curate_case(
            &config,
            "ev",
            &passed,
            &json!({ "reference": { "answer": "x" } }),
        );
        assert_eq!(d3["reason"], "case_passed_threshold");
        let failed = json!({ "score": 0.1 });
        let d4 = curate_case(&config, "ev", &failed, &json!({ "query": "q" }));
        assert_eq!(d4["reason"], "missing_judgeable_reference");
    }

    #[test]
    fn is_judgeable_variants() {
        assert!(is_judgeable(&json!({ "verification_contract": {} })));
        assert!(is_judgeable(&json!({ "reference": { "answer": "a" } })));
        assert!(is_judgeable(
            &json!({ "reference": { "required_behaviors": [{}] } })
        ));
        assert!(is_judgeable(&json!({ "expected": "e" })));
        assert!(is_judgeable(&json!({ "assertions": [] })));
        assert!(!is_judgeable(&json!({ "query": "q" })));
        assert!(!is_judgeable(&json!({ "reference": { "answer": "" } })));
    }

    #[test]
    fn load_original_case_by_id_and_index() {
        let dir = temp_dir("load");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("cases.json"),
            serde_json::to_string(&json!({ "cases": [
                { "case_id": "a", "q": 1 },
                { "case_id": "b", "q": 2 },
            ] }))
            .unwrap(),
        )
        .unwrap();
        let by_id = json!({ "case_id": "b", "case_path": dir.join("cases.json") });
        let loaded = load_original_case(&by_id);
        assert_eq!(loaded["q"], 2);
        let by_index = json!({ "case_index": 1, "case_path": dir.join("cases.json") });
        let loaded2 = load_original_case(&by_index);
        assert_eq!(loaded2["q"], 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn report_payload_summary_counts() {
        let payload = report_payload(
            "completed",
            "ev",
            &[
                json!({ "case_id": "replay_x", "metadata": { "provenance": { "source_case_id": "x" } } }),
            ],
            &[json!({ "case_id": "y", "reason": "r" })],
            "ds.json",
            "seed.json",
        );
        assert_eq!(payload["status"], "completed");
        assert_eq!(payload["summary"]["candidate_cases"], 2);
        assert_eq!(payload["summary"]["accepted_cases"], 1);
        assert_eq!(payload["accepted_cases"][0]["source_case_id"], "x");
        assert_eq!(payload["rejected_cases"][0]["reason"], "r");
    }

    #[test]
    fn plugin_registers_curator() {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(DatasetCuratorPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let curator = ctx
            .service::<dyn DatasetCurator>(&DATASET_CURATOR)
            .expect("curator seam");
        let _ = curator;
        drop(effects);
        assert!(!ctx.has_service(&DATASET_CURATOR));
    }
}
