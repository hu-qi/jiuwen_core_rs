//! 调度配置 + rsi-config 插件。

//! # ah-plugins-rsi-config
//!
//! Real rsi configuration models (aligned with openjiuwen/rsi/config/config.py):
//! 12 dataclass configs with from_dict parsing. Pure data + validation.

use ah_contracts::keys::RSI_CONFIG;
use ah_contracts::prelude::Effect;
use ah_contracts::rsi_config::parse_bool;
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
