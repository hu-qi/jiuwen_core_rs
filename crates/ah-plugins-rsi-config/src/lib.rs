//! # ah-plugins-rsi-config
//!
//! Real rsi configuration models (aligned with openjiuwen/rsi/config/config.py):
//! 12 dataclass configs with from_dict parsing. Pure data + validation.

use ah_contracts::keys::RSI_CONFIG;
use ah_contracts::prelude::Effect;
use ah_contracts::rsi_config::{parse_bool, parse_float, parse_int, parse_string_list};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use serde_json::Value;
use std::sync::Arc;

fn str_val(data: &serde_json::Map<String, Value>, key: &str, default: &str) -> String {
    data.get(key)
        .and_then(Value::as_str)
        .unwrap_or(default)
        .to_string()
}

/// 数据集加载配置(对齐 DataLoaderConfig)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DataLoaderConfig {
    pub file_pattern: String,
    pub batch_size: usize,
    pub batch_balance_keys: Vec<String>,
}

impl Default for DataLoaderConfig {
    fn default() -> Self {
        Self {
            file_pattern: "*.json".to_string(),
            batch_size: 1,
            batch_balance_keys: vec![
                "dimension".to_string(),
                "difficulty".to_string(),
                "source".to_string(),
                "task_type".to_string(),
            ],
        }
    }
}

impl DataLoaderConfig {
    pub fn from_dict(data: &Value) -> Result<Self, String> {
        let data = data.as_object().ok_or("expected object")?;
        Ok(Self {
            file_pattern: str_val(data, "file_pattern", "*.json"),
            batch_size: parse_int(data.get("batch_size"), 1)? as usize,
            batch_balance_keys: {
                let keys = parse_string_list(data.get("batch_balance_keys"))?;
                if keys.is_empty() {
                    DataLoaderConfig::default().batch_balance_keys
                } else {
                    keys
                }
            },
        })
    }
}

/// 数据集策展配置(对齐 DatasetCurationConfig)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DatasetCurationConfig {
    pub enabled: bool,
    pub score_threshold: f64,
    pub require_judgeable_reference: bool,
    pub output_filename: String,
    pub report_filename: String,
    pub targeted_seed_filename: String,
    pub source_label: String,
}

impl Default for DatasetCurationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            score_threshold: 1.0,
            require_judgeable_reference: true,
            output_filename: "replay_cases.json".to_string(),
            report_filename: "curation_report.yaml".to_string(),
            targeted_seed_filename: "targeted_dataset_seed.json".to_string(),
            source_label: "trace_replay".to_string(),
        }
    }
}

impl DatasetCurationConfig {
    pub fn from_dict(data: &Value) -> Result<Self, String> {
        let data = data.as_object().ok_or("expected object")?;
        Ok(Self {
            enabled: parse_bool(data.get("enabled"), true)?,
            score_threshold: parse_float(data.get("score_threshold"), 1.0)?,
            require_judgeable_reference: parse_bool(data.get("require_judgeable_reference"), true)?,
            output_filename: str_val(data, "output_filename", "replay_cases.json"),
            report_filename: str_val(data, "report_filename", "curation_report.yaml"),
            targeted_seed_filename: str_val(
                data,
                "targeted_seed_filename",
                "targeted_dataset_seed.json",
            ),
            source_label: str_val(data, "source_label", "trace_replay"),
        })
    }
}

/// 模型配置引用(对齐 ModelConfigs)。
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct ModelConfigs {
    pub evaluation: String,
    pub judge: String,
    pub analysis: String,
    pub team_skill_optimization: String,
    pub member_optimization: String,
    pub experience_learning: String,
    pub dataset_generation: String,
}

impl ModelConfigs {
    pub fn from_dict(data: &Value) -> Self {
        let data = data.as_object().cloned().unwrap_or_default();
        Self {
            evaluation: str_val(&data, "evaluation", ""),
            judge: str_val(&data, "judge", ""),
            analysis: str_val(&data, "analysis", ""),
            team_skill_optimization: str_val(&data, "team_skill_optimization", ""),
            member_optimization: str_val(&data, "member_optimization", ""),
            experience_learning: str_val(&data, "experience_learning", ""),
            dataset_generation: str_val(&data, "dataset_generation", ""),
        }
    }
}

/// 种子评估配置(对齐 SeedEvaluationConfig)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SeedEvaluationConfig {
    pub enabled: bool,
    pub pass_threshold: f64,
    pub excellent_threshold: f64,
    pub max_cases: usize,
}

impl Default for SeedEvaluationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            pass_threshold: 0.8,
            excellent_threshold: 0.99,
            max_cases: 20,
        }
    }
}

impl SeedEvaluationConfig {
    pub fn from_dict(data: &Value) -> Result<Self, String> {
        let data = data.as_object().ok_or("expected object")?;
        Ok(Self {
            enabled: parse_bool(data.get("enabled"), false)?,
            pass_threshold: parse_float(data.get("pass_threshold"), 0.8)?,
            excellent_threshold: parse_float(data.get("excellent_threshold"), 0.99)?,
            max_cases: parse_int(data.get("max_cases"), 20)? as usize,
        })
    }
}

/// 调度配置(对齐 OrchestratorSchedulingConfig;含固定策略校验)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OrchestratorSchedulingConfig {
    pub evaluation_strategy: String,
    pub coordination_strategy: String,
    pub promotion_policy: String,
    pub full_evaluation_enabled: bool,
}

impl Default for OrchestratorSchedulingConfig {
    fn default() -> Self {
        Self {
            evaluation_strategy: "hybrid".to_string(),
            coordination_strategy: "team_first_single_pass".to_string(),
            promotion_policy: "epoch_full_evaluation".to_string(),
            full_evaluation_enabled: true,
        }
    }
}

impl OrchestratorSchedulingConfig {
    pub fn from_dict(data: &Value) -> Result<Self, String> {
        let data = data.as_object().ok_or("expected object")?;
        let config = Self {
            evaluation_strategy: str_val(data, "evaluation_strategy", "hybrid"),
            coordination_strategy: str_val(data, "coordination_strategy", "team_first_single_pass"),
            promotion_policy: str_val(data, "promotion_policy", "epoch_full_evaluation"),
            full_evaluation_enabled: parse_bool(data.get("full_evaluation_enabled"), true)?,
        };
        config.validate()?;
        Ok(config)
    }

    /// 强制 011 调度策略契约(对齐 validate)。
    pub fn validate(&self) -> Result<(), String> {
        if self.evaluation_strategy != "hybrid" {
            return Err("scheduling.evaluation_strategy must be hybrid".to_string());
        }
        if self.coordination_strategy != "team_first_single_pass" {
            return Err(
                "scheduling.coordination_strategy must be team_first_single_pass".to_string(),
            );
        }
        if self.promotion_policy != "epoch_full_evaluation" {
            return Err("scheduling.promotion_policy must be epoch_full_evaluation".to_string());
        }
        Ok(())
    }
}

/// rsi-config 插件:注册配置聚合服务。
pub struct RsiConfigService;

impl Seam for RsiConfigService {}

/// rsi-config 插件。
pub struct RsiConfigPlugin;

impl Plugin for RsiConfigPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-rsi-config"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![RSI_CONFIG]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let svc = Arc::new(RsiConfigService);
        Ok(vec![ctx.register(RSI_CONFIG, svc)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn data_loader_config_defaults_and_from_dict() {
        let d = DataLoaderConfig::default();
        assert_eq!(d.file_pattern, "*.json");
        assert_eq!(d.batch_size, 1);
        assert_eq!(d.batch_balance_keys.len(), 4);
        let parsed = DataLoaderConfig::from_dict(&json!({
            "batch_size": 4,
            "batch_balance_keys": ["dimension", "difficulty"]
        }))
        .unwrap();
        assert_eq!(parsed.batch_size, 4);
        assert_eq!(parsed.batch_balance_keys.len(), 2);
        assert!(DataLoaderConfig::from_dict(&json!({"batch_size": true})).is_err());
    }

    #[test]
    fn dataset_curation_config_parses() {
        let d = DatasetCurationConfig::default();
        assert!(d.enabled);
        assert_eq!(d.score_threshold, 1.0);
        let parsed = DatasetCurationConfig::from_dict(&json!({
            "score_threshold": 0.5,
            "source_label": "custom"
        }))
        .unwrap();
        assert_eq!(parsed.score_threshold, 0.5);
        assert_eq!(parsed.source_label, "custom");
    }

    #[test]
    fn model_configs_all_fields() {
        let m = ModelConfigs::from_dict(&json!({
            "evaluation": "eval-model",
            "judge": "judge-model"
        }));
        assert_eq!(m.evaluation, "eval-model");
        assert_eq!(m.judge, "judge-model");
        assert_eq!(m.analysis, "");
    }

    #[test]
    fn seed_evaluation_config_defaults() {
        let s = SeedEvaluationConfig::default();
        assert!(!s.enabled);
        assert_eq!(s.pass_threshold, 0.8);
        assert_eq!(s.excellent_threshold, 0.99);
        assert_eq!(s.max_cases, 20);
        let parsed =
            SeedEvaluationConfig::from_dict(&json!({"enabled": "yes", "max_cases": 5})).unwrap();
        assert!(parsed.enabled);
        assert_eq!(parsed.max_cases, 5);
    }

    #[test]
    fn scheduling_config_validates_fixed_strategy() {
        let s = OrchestratorSchedulingConfig::default();
        assert!(s.validate().is_ok());
        let bad = OrchestratorSchedulingConfig {
            evaluation_strategy: "wrong".to_string(),
            ..Default::default()
        };
        assert!(bad.validate().is_err());
        assert!(
            OrchestratorSchedulingConfig::from_dict(&json!({"evaluation_strategy": "x"})).is_err()
        );
    }

    #[test]
    fn parse_helpers_reject_bad_types() {
        assert!(parse_int(Some(&json!(true)), 1).is_err());
        assert!(parse_float(Some(&json!(false)), 1.0).is_err());
        assert!(parse_bool(Some(&json!("maybe")), false).is_err());
        assert!(parse_bool(Some(&json!("ON")), false).unwrap());
        assert_eq!(parse_int(Some(&json!("42")), 0).unwrap(), 42);
        assert_eq!(parse_float(Some(&json!("1.5")), 0.0).unwrap(), 1.5);
    }
}
