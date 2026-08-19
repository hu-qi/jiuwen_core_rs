//! # ah-plugins-rsi-evaluator
//!
//! 真实 RSI 轨迹工具(1:1 对齐 `rsi/evaluator/trajectory_paths.py` +
//! `trajectory_usage.py` 的确定性部分):
//! - bounded 轨迹:truncate_text / truncate_json_like / bounded_messages /
//!   tool_summary / safe_role_file_stem / bound_llm_detail / bound_tool_detail;
//! - usage 提取:collect_successful_tool_names / collect_successful_skill_names /
//!   collect_pre_edit_successful_usage / is_persistent_edit_step。
//!
//! 纯函数、无 IO;委托契约层实现。

use std::sync::Arc;

use ah_contracts::keys::RSI_EVALUATOR;
use ah_contracts::prelude::Effect;
use ah_contracts::rsi_evaluator::{
    RsiTrajectoryTools, bounded_trajectory_dict, collect_successful_skill_names,
    collect_successful_tool_names,
};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 真实实现:委托契约层纯函数。
pub struct RsiTrajectoryToolsImpl;

impl Seam for RsiTrajectoryToolsImpl {}

impl RsiTrajectoryTools for RsiTrajectoryToolsImpl {
    fn bounded_trajectory_dict(&self, data: serde_json::Value) -> serde_json::Value {
        bounded_trajectory_dict(data)
    }

    fn collect_successful_tool_names(&self, value: &serde_json::Value, names: &mut Vec<String>) {
        collect_successful_tool_names(value, names);
    }

    fn collect_successful_skill_names(&self, value: &serde_json::Value, names: &mut Vec<String>) {
        collect_successful_skill_names(value, names);
    }
}

/// rsi-evaluator 插件:注册 `rsi-evaluator` seam。
pub struct RsiEvaluatorPlugin;

impl Plugin for RsiEvaluatorPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-rsi-evaluator"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![RSI_EVALUATOR]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let svc: Arc<dyn RsiTrajectoryTools> = Arc::new(RsiTrajectoryToolsImpl);
        Ok(vec![ctx.register(RSI_EVALUATOR, svc)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::RSI_EVALUATOR;
    use ah_hub::plugin::DynPlugin;

    fn build_ctx() -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![Arc::new(RsiEvaluatorPlugin)];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[test]
    fn seam_registered() {
        let (ctx, effects) = build_ctx();
        assert!(
            ctx.service::<dyn RsiTrajectoryTools>(&RSI_EVALUATOR)
                .is_some()
        );
        drop(effects);
    }

    #[test]
    fn bounded_and_usage_via_seam() {
        let (ctx, effects) = build_ctx();
        let svc = ctx
            .service::<dyn RsiTrajectoryTools>(&RSI_EVALUATOR)
            .expect("svc");
        // bounded。
        let data = serde_json::json!({
            "steps": [{"detail": {"messages": [{"content": "x".repeat(1500)}]}}]
        });
        let bounded = svc.bounded_trajectory_dict(data);
        assert!(
            bounded["steps"][0]["detail"]["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("[truncated")
        );
        // usage。
        let traj = serde_json::json!({
            "steps": [
                {"detail": {"tool_name": "read_file"}},
                {"detail": {"tool_name": "skill_tool", "call_args": {"skill_name": "s1"}}},
            ]
        });
        let mut tools = Vec::new();
        svc.collect_successful_tool_names(&traj, &mut tools);
        assert!(tools.contains(&"read_file".to_string()));
        let mut skills = Vec::new();
        svc.collect_successful_skill_names(&traj, &mut skills);
        assert_eq!(skills, vec!["s1".to_string()]);
        drop(effects);
    }

    #[test]
    fn pre_edit_usage_via_seam() {
        let traj = serde_json::json!({
            "steps": [
                {"detail": {"tool_name": "grep"}},
                {"detail": {"tool_name": "write_file"}},
            ]
        });
        let mut tools = Vec::new();
        let edit = ah_contracts::rsi_evaluator::collect_pre_edit_successful_usage(
            &traj,
            Some(&mut tools),
            None,
        );
        assert_eq!(edit, Some(1));
        assert!(tools.contains(&"grep".to_string()));
        let _ = ah_contracts::rsi_evaluator::bound_llm_detail;
        let _ = ah_contracts::rsi_evaluator::bound_tool_detail;
    }
}
