//! 顶层编排配置。
//!
//! 聚合全部子配置(AutoCoordinatingHarnessConfig)。
//!
//! Real rsi configuration models (aligned with openjiuwen/rsi/config/config.py):
//! 12 dataclass configs with from_dict parsing. Pure data + validation.

use ah_contracts::rsi_config::{parse_bool, parse_int};
use serde_json::Value;

use crate::core::{DataLoaderConfig, DatasetCurationConfig, ModelConfigs, SeedEvaluationConfig};
use crate::extended::{
    DatasetGeneratorConfig, EvaluationResultAnalyzerConfig, EvaluatorConfig, MemberOptimizerConfig,
    OptimizationExperienceLearnerConfig, TeamSkillOptimizerConfig,
};
use crate::scheduling::OrchestratorSchedulingConfig;

fn str_val(data: &serde_json::Map<String, Value>, key: &str, default: &str) -> String {
    data.get(key)
        .and_then(Value::as_str)
        .unwrap_or(default)
        .to_string()
}

pub struct AutoCoordinatingHarnessConfig {
    pub workspace_dir: String,
    pub max_epochs: i64,
    pub freeze_team_skill: bool,
    pub freeze_team_members: bool,
    pub model_configs: ModelConfigs,
    pub data_loader: DataLoaderConfig,
    pub dataset_curation: DatasetCurationConfig,
    pub dataset_generator: DatasetGeneratorConfig,
    pub evaluator: EvaluatorConfig,
    pub evaluation_result_analyzer: EvaluationResultAnalyzerConfig,
    pub team_skill_optimizer: TeamSkillOptimizerConfig,
    pub member_optimizer: MemberOptimizerConfig,
    pub optimization_experience_learner: OptimizationExperienceLearnerConfig,
    pub scheduling: OrchestratorSchedulingConfig,
    pub seed_evaluation: SeedEvaluationConfig,
}

impl Default for AutoCoordinatingHarnessConfig {
    fn default() -> Self {
        Self {
            workspace_dir: String::new(),
            max_epochs: 1,
            freeze_team_skill: false,
            freeze_team_members: false,
            model_configs: ModelConfigs::default(),
            data_loader: DataLoaderConfig::default(),
            dataset_curation: DatasetCurationConfig::default(),
            dataset_generator: DatasetGeneratorConfig::default(),
            evaluator: EvaluatorConfig::default(),
            evaluation_result_analyzer: EvaluationResultAnalyzerConfig::default(),
            team_skill_optimizer: TeamSkillOptimizerConfig::default(),
            member_optimizer: MemberOptimizerConfig::default(),
            optimization_experience_learner: OptimizationExperienceLearnerConfig::default(),
            scheduling: OrchestratorSchedulingConfig::default(),
            seed_evaluation: SeedEvaluationConfig::default(),
        }
    }
}

impl AutoCoordinatingHarnessConfig {
    /// 从顶层 YAML 解析(对齐 from_dict;子配置缺省用默认)。
    pub fn from_dict(data: &Value) -> Result<Self, String> {
        let data = data.as_object().ok_or("expected object")?;
        let sub = |key: &str| {
            data.get(key)
                .cloned()
                .unwrap_or_else(|| Value::Object(Default::default()))
        };
        Ok(Self {
            workspace_dir: str_val(data, "workspace_dir", ""),
            max_epochs: parse_int(data.get("max_epochs"), 1)?,
            freeze_team_skill: parse_bool(data.get("freeze_team_skill"), false)?,
            freeze_team_members: parse_bool(data.get("freeze_team_members"), false)?,
            model_configs: ModelConfigs::from_dict(&sub("model_configs")),
            data_loader: DataLoaderConfig::from_dict(&sub("data_loader"))?,
            dataset_curation: DatasetCurationConfig::from_dict(&sub("dataset_curation"))?,
            dataset_generator: DatasetGeneratorConfig::from_dict(&sub("dataset_generator"))?,
            evaluator: EvaluatorConfig::from_dict(&sub("evaluator"))?,
            evaluation_result_analyzer: EvaluationResultAnalyzerConfig::from_dict(&sub(
                "evaluation_result_analyzer",
            ))?,
            team_skill_optimizer: TeamSkillOptimizerConfig::from_dict(&sub(
                "team_skill_optimizer",
            ))?,
            member_optimizer: MemberOptimizerConfig::from_dict(&sub("member_optimizer"))?,
            optimization_experience_learner: OptimizationExperienceLearnerConfig::from_dict(&sub(
                "optimization_experience_learner",
            ))?,
            scheduling: OrchestratorSchedulingConfig::from_dict(&sub("scheduling"))?,
            seed_evaluation: SeedEvaluationConfig::from_dict(&sub("seed_evaluation"))?,
        })
    }
}

#[cfg(test)]
mod tests2 {
    use super::*;
    use serde_json::json;

    #[test]
    fn evaluator_config_parses_fields() {
        let e = EvaluatorConfig::default();
        assert_eq!(e.backend, "local");
        assert_eq!(e.evaluation_method, "llm-as-judge");
        assert_eq!(e.success_score, 1.0);
        let parsed = EvaluatorConfig::from_dict(&json!({
            "model_provider": "DeepSeek",
            "success_score": 0.9,
            "transient_case_retry_limit": 5
        }))
        .unwrap();
        assert_eq!(parsed.model_provider, "DeepSeek");
        assert_eq!(parsed.success_score, 0.9);
        assert_eq!(parsed.transient_case_retry_limit, 5);
    }

    #[test]
    fn dataset_generator_config_parses() {
        let g = DatasetGeneratorConfig::default();
        assert_eq!(g.quality_score_threshold, 8);
        assert!(g.quality_review_enabled);
        let parsed = DatasetGeneratorConfig::from_dict(&json!({
            "min_cases": 10,
            "quality_review_enabled": "no"
        }))
        .unwrap();
        assert_eq!(parsed.min_cases, 10);
        assert!(!parsed.quality_review_enabled);
    }

    #[test]
    fn member_optimizer_config_defaults_and_overrides() {
        let m = MemberOptimizerConfig::default();
        assert_eq!(m.max_roles_per_run, 2);
        assert_eq!(m.max_trace_excerpt_chars, 4000);
        assert!(m.allow_empty_harness_refs_noop);
        let parsed = MemberOptimizerConfig::from_dict(&json!({
            "max_roles_per_run": 3,
            "max_actions_per_plan": 5
        }))
        .unwrap();
        assert_eq!(parsed.max_roles_per_run, 3);
        assert_eq!(parsed.max_actions_per_plan, 5);
        assert_eq!(parsed.max_trace_excerpt_chars, 4000); // untouched default
    }

    #[test]
    fn auto_config_nests_all_subconfigs() {
        let a = AutoCoordinatingHarnessConfig::default();
        assert_eq!(a.max_epochs, 1);
        assert!(!a.freeze_team_skill);
        assert_eq!(a.evaluator.backend, "local");
        let parsed = AutoCoordinatingHarnessConfig::from_dict(&json!({
            "max_epochs": 3,
            "evaluator": {"backend": "remote"},
            "seed_evaluation": {"enabled": true}
        }))
        .unwrap();
        assert_eq!(parsed.max_epochs, 3);
        assert_eq!(parsed.evaluator.backend, "remote");
        assert!(parsed.seed_evaluation.enabled);
        assert_eq!(parsed.data_loader.batch_size, 1); // default
    }

    #[test]
    fn analyzer_and_skill_configs_parse() {
        let a = EvaluationResultAnalyzerConfig::from_dict(&json!({
            "max_issues": 10,
            "output_filename": "custom.yaml"
        }))
        .unwrap();
        assert_eq!(a.max_issues, 10);
        assert_eq!(a.output_filename, "custom.yaml");
        let t = TeamSkillOptimizerConfig::from_dict(&json!({
            "max_candidates": 3,
            "language": "en"
        }))
        .unwrap();
        assert_eq!(t.max_candidates, 3);
        assert_eq!(t.language, "en");
        let o = OptimizationExperienceLearnerConfig::from_dict(&json!({
            "enabled": "false"
        }))
        .unwrap();
        assert!(!o.enabled);
    }
}
