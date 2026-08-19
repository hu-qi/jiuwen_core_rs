//! 扩展配置模型。

//! # ah-plugins-rsi-config
//!
//! Real rsi configuration models (aligned with openjiuwen/rsi/config/config.py):
//! 12 dataclass configs with from_dict parsing. Pure data + validation.

use ah_contracts::rsi_config::{parse_bool, parse_float, parse_int, parse_string_list};
use serde_json::Value;

fn str_val(data: &serde_json::Map<String, Value>, key: &str, default: &str) -> String {
    data.get(key)
        .and_then(Value::as_str)
        .unwrap_or(default)
        .to_string()
}

pub struct EvaluatorConfig {
    pub model_config_ref: String,
    pub judge_model_config_ref: String,
    pub team_spec_config_ref: String,
    pub default_script: String,
    pub model_name: String,
    pub model_url: String,
    pub model_api_key: String,
    pub model_provider: String,
    pub backend: String,
    pub evaluation_method: String,
    pub script_configs: Vec<String>,
    pub success_score: f64,
    pub case_lifecycle_timeout_sec: f64,
    pub transient_case_retry_limit: i64,
}

impl Default for EvaluatorConfig {
    fn default() -> Self {
        Self {
            model_config_ref: String::new(),
            judge_model_config_ref: String::new(),
            team_spec_config_ref: String::new(),
            default_script: "default".to_string(),
            model_name: String::new(),
            model_url: String::new(),
            model_api_key: String::new(),
            model_provider: "OpenAI".to_string(),
            backend: "local".to_string(),
            evaluation_method: "llm-as-judge".to_string(),
            script_configs: Vec::new(),
            success_score: 1.0,
            case_lifecycle_timeout_sec: 3600.0,
            transient_case_retry_limit: 2,
        }
    }
}

impl EvaluatorConfig {
    pub fn from_dict(data: &Value) -> Result<Self, String> {
        let data = data.as_object().ok_or("expected object")?;
        Ok(Self {
            model_config_ref: str_val(data, "model_config_ref", ""),
            judge_model_config_ref: str_val(data, "judge_model_config_ref", ""),
            team_spec_config_ref: str_val(data, "team_spec_config_ref", ""),
            default_script: str_val(data, "default_script", "default"),
            model_name: str_val(data, "model_name", ""),
            model_url: str_val(data, "model_url", ""),
            model_api_key: str_val(data, "model_api_key", ""),
            model_provider: str_val(data, "model_provider", "OpenAI"),
            backend: str_val(data, "backend", "local"),
            evaluation_method: str_val(data, "evaluation_method", "llm-as-judge"),
            script_configs: parse_string_list(data.get("script_configs"))?,
            success_score: parse_float(data.get("success_score"), 1.0)?,
            case_lifecycle_timeout_sec: parse_float(
                data.get("case_lifecycle_timeout_sec"),
                3600.0,
            )?,
            transient_case_retry_limit: parse_int(data.get("transient_case_retry_limit"), 2)?,
        })
    }
}

/// 数据集生成配置(对齐 DatasetGeneratorConfig)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DatasetGeneratorConfig {
    pub model_config_ref: String,
    pub min_cases: i64,
    pub coverage_dimensions: Vec<String>,
    pub known_failures_ref: String,
    pub quality_review_enabled: bool,
    pub quality_score_threshold: i64,
    pub capability_alignment_score_threshold: i64,
    pub verifiability_score_threshold: i64,
    pub difficulty_score_threshold: i64,
}

impl Default for DatasetGeneratorConfig {
    fn default() -> Self {
        Self {
            model_config_ref: String::new(),
            min_cases: 0,
            coverage_dimensions: Vec::new(),
            known_failures_ref: String::new(),
            quality_review_enabled: true,
            quality_score_threshold: 8,
            capability_alignment_score_threshold: 8,
            verifiability_score_threshold: 8,
            difficulty_score_threshold: 3,
        }
    }
}

impl DatasetGeneratorConfig {
    pub fn from_dict(data: &Value) -> Result<Self, String> {
        let data = data.as_object().ok_or("expected object")?;
        Ok(Self {
            model_config_ref: str_val(data, "model_config_ref", ""),
            min_cases: parse_int(data.get("min_cases"), 0)?,
            coverage_dimensions: parse_string_list(data.get("coverage_dimensions"))?,
            known_failures_ref: str_val(data, "known_failures_ref", ""),
            quality_review_enabled: parse_bool(data.get("quality_review_enabled"), true)?,
            quality_score_threshold: parse_int(data.get("quality_score_threshold"), 8)?,
            capability_alignment_score_threshold: parse_int(
                data.get("capability_alignment_score_threshold"),
                8,
            )?,
            verifiability_score_threshold: parse_int(data.get("verifiability_score_threshold"), 8)?,
            difficulty_score_threshold: parse_int(data.get("difficulty_score_threshold"), 3)?,
        })
    }
}

/// 评估结果分析配置(对齐 EvaluationResultAnalyzerConfig)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EvaluationResultAnalyzerConfig {
    pub model_config_ref: String,
    pub diagnosis_agent_model_config_ref: String,
    pub diagnosis_agent_max_retries: i64,
    pub diagnosis_agent_max_concurrency: i64,
    pub diagnosis_agent_max_iterations: i64,
    pub max_issues: i64,
    pub evidence_limit_per_issue: i64,
    pub output_filename: String,
}

impl Default for EvaluationResultAnalyzerConfig {
    fn default() -> Self {
        Self {
            model_config_ref: String::new(),
            diagnosis_agent_model_config_ref: String::new(),
            diagnosis_agent_max_retries: 3,
            diagnosis_agent_max_concurrency: 5,
            diagnosis_agent_max_iterations: 20,
            max_issues: 20,
            evidence_limit_per_issue: 5,
            output_filename: "issues.yaml".to_string(),
        }
    }
}

impl EvaluationResultAnalyzerConfig {
    pub fn from_dict(data: &Value) -> Result<Self, String> {
        let data = data.as_object().ok_or("expected object")?;
        Ok(Self {
            model_config_ref: str_val(data, "model_config_ref", ""),
            diagnosis_agent_model_config_ref: str_val(data, "diagnosis_agent_model_config_ref", ""),
            diagnosis_agent_max_retries: parse_int(data.get("diagnosis_agent_max_retries"), 3)?,
            diagnosis_agent_max_concurrency: parse_int(
                data.get("diagnosis_agent_max_concurrency"),
                5,
            )?,
            diagnosis_agent_max_iterations: parse_int(
                data.get("diagnosis_agent_max_iterations"),
                20,
            )?,
            max_issues: parse_int(data.get("max_issues"), 20)?,
            evidence_limit_per_issue: parse_int(data.get("evidence_limit_per_issue"), 5)?,
            output_filename: str_val(data, "output_filename", "issues.yaml"),
        })
    }
}

/// 团队技能优化配置(对齐 TeamSkillOptimizerConfig)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TeamSkillOptimizerConfig {
    pub model_config_ref: String,
    pub max_candidates: i64,
    pub freeze: bool,
    pub language: String,
    pub auto_approve: bool,
}

impl Default for TeamSkillOptimizerConfig {
    fn default() -> Self {
        Self {
            model_config_ref: String::new(),
            max_candidates: 1,
            freeze: false,
            language: "cn".to_string(),
            auto_approve: true,
        }
    }
}

impl TeamSkillOptimizerConfig {
    pub fn from_dict(data: &Value) -> Result<Self, String> {
        let data = data.as_object().ok_or("expected object")?;
        Ok(Self {
            model_config_ref: str_val(data, "model_config_ref", ""),
            max_candidates: parse_int(data.get("max_candidates"), 1)?,
            freeze: parse_bool(data.get("freeze"), false)?,
            language: str_val(data, "language", "cn"),
            auto_approve: parse_bool(data.get("auto_approve"), true)?,
        })
    }
}

/// 优化经验学习配置(对齐 OptimizationExperienceLearnerConfig)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OptimizationExperienceLearnerConfig {
    pub model_config_ref: String,
    pub output_filename: String,
    pub enabled: bool,
}

impl Default for OptimizationExperienceLearnerConfig {
    fn default() -> Self {
        Self {
            model_config_ref: String::new(),
            output_filename: "experience_ref.yaml".to_string(),
            enabled: true,
        }
    }
}

impl OptimizationExperienceLearnerConfig {
    pub fn from_dict(data: &Value) -> Result<Self, String> {
        let data = data.as_object().ok_or("expected object")?;
        Ok(Self {
            model_config_ref: str_val(data, "model_config_ref", ""),
            output_filename: str_val(data, "output_filename", "experience_ref.yaml"),
            enabled: parse_bool(data.get("enabled"), true)?,
        })
    }
}

/// 成员优化配置(对齐 MemberOptimizerConfig;feat_009 扩展字段)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MemberOptimizerConfig {
    pub model_config_ref: String,
    pub action_group_configs: Vec<String>,
    pub agent_skills_dirs: Vec<String>,
    pub freeze: bool,
    pub max_roles_per_run: i64,
    pub min_attribution_confidence: f64,
    pub attribution_retry_limit: i64,
    pub stage_retry_limit: i64,
    pub max_cases_per_issue: i64,
    pub max_trace_excerpt_chars: i64,
    pub max_result_excerpt_chars: i64,
    pub allow_empty_harness_refs_noop: bool,
    pub execution_concurrency: i64,
    pub role_execution_concurrency: i64,
    pub action_execution_concurrency_per_role: i64,
    pub allowed_action_groups: Vec<String>,
    pub allowed_prompt_surfaces: Vec<String>,
    pub max_actions_per_plan: i64,
    pub candidate_min_score_delta: f64,
    pub candidate_min_target_behavior_delta: f64,
    pub candidate_non_target_max_regression: f64,
    pub candidate_holdout_cases: i64,
    pub candidate_holdout_max_regression: f64,
    pub adapt_frozen_team_issues: bool,
}

impl Default for MemberOptimizerConfig {
    fn default() -> Self {
        Self {
            model_config_ref: String::new(),
            action_group_configs: Vec::new(),
            agent_skills_dirs: Vec::new(),
            freeze: false,
            max_roles_per_run: 2,
            min_attribution_confidence: 0.5,
            attribution_retry_limit: 3,
            stage_retry_limit: 3,
            max_cases_per_issue: 3,
            max_trace_excerpt_chars: 4000,
            max_result_excerpt_chars: 2000,
            allow_empty_harness_refs_noop: true,
            execution_concurrency: 2,
            role_execution_concurrency: 2,
            action_execution_concurrency_per_role: 2,
            allowed_action_groups: Vec::new(),
            allowed_prompt_surfaces: Vec::new(),
            max_actions_per_plan: 0,
            candidate_min_score_delta: 0.0,
            candidate_min_target_behavior_delta: 0.0,
            candidate_non_target_max_regression: 0.0,
            candidate_holdout_cases: 0,
            candidate_holdout_max_regression: 0.0,
            adapt_frozen_team_issues: false,
        }
    }
}

impl MemberOptimizerConfig {
    pub fn from_dict(data: &Value) -> Result<Self, String> {
        let data = data.as_object().ok_or("expected object")?;
        Ok(Self {
            model_config_ref: str_val(data, "model_config_ref", ""),
            action_group_configs: parse_string_list(data.get("action_group_configs"))?,
            agent_skills_dirs: parse_string_list(data.get("agent_skills_dirs"))?,
            freeze: parse_bool(data.get("freeze"), false)?,
            max_roles_per_run: parse_int(data.get("max_roles_per_run"), 2)?,
            min_attribution_confidence: parse_float(data.get("min_attribution_confidence"), 0.5)?,
            attribution_retry_limit: parse_int(data.get("attribution_retry_limit"), 3)?,
            stage_retry_limit: parse_int(data.get("stage_retry_limit"), 3)?,
            max_cases_per_issue: parse_int(data.get("max_cases_per_issue"), 3)?,
            max_trace_excerpt_chars: parse_int(data.get("max_trace_excerpt_chars"), 4000)?,
            max_result_excerpt_chars: parse_int(data.get("max_result_excerpt_chars"), 2000)?,
            allow_empty_harness_refs_noop: parse_bool(
                data.get("allow_empty_harness_refs_noop"),
                true,
            )?,
            execution_concurrency: parse_int(data.get("execution_concurrency"), 2)?,
            role_execution_concurrency: parse_int(data.get("role_execution_concurrency"), 2)?,
            action_execution_concurrency_per_role: parse_int(
                data.get("action_execution_concurrency_per_role"),
                2,
            )?,
            allowed_action_groups: parse_string_list(data.get("allowed_action_groups"))?,
            allowed_prompt_surfaces: parse_string_list(data.get("allowed_prompt_surfaces"))?,
            max_actions_per_plan: parse_int(data.get("max_actions_per_plan"), 0)?,
            candidate_min_score_delta: parse_float(data.get("candidate_min_score_delta"), 0.0)?,
            candidate_min_target_behavior_delta: parse_float(
                data.get("candidate_min_target_behavior_delta"),
                0.0,
            )?,
            candidate_non_target_max_regression: parse_float(
                data.get("candidate_non_target_max_regression"),
                0.0,
            )?,
            candidate_holdout_cases: parse_int(data.get("candidate_holdout_cases"), 0)?,
            candidate_holdout_max_regression: parse_float(
                data.get("candidate_holdout_max_regression"),
                0.0,
            )?,
            adapt_frozen_team_issues: parse_bool(data.get("adapt_frozen_team_issues"), false)?,
        })
    }
}
