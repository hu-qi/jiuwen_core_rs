//! # ah-plugins-team-skill-generator
//!
//! 真实团队技能生成归一化(1:1 对齐 `rsi/team_skill_generator/generator.py`
//! 的确定性部分):slugify/single_line/string_list/normalize_roles/
//! normalize_workflow_steps/normalize_team_skill_plan/write_skill_md。
//!
//! 纯函数、无 IO、无 LLM;LLM plan/create/repair 由插件侧注入(本 crate 只做
//! 归一化门面)。

use std::sync::Arc;

use ah_contracts::keys::TEAM_SKILL_GENERATOR;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::team_skill_generator::{
    SkillGenError, TeamSkillPlanNormalizer, normalize_team_skill_plan, write_skill_md,
};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 真实实现:委托契约层纯函数。
pub struct TeamSkillGeneratorImpl;

impl Seam for TeamSkillGeneratorImpl {}

impl TeamSkillPlanNormalizer for TeamSkillGeneratorImpl {
    fn normalize_plan(
        &self,
        raw_plan: &serde_json::Value,
        task: &str,
    ) -> Result<serde_json::Value, SkillGenError> {
        normalize_team_skill_plan(raw_plan, task)
    }

    fn build_skill_md(&self, plan: &serde_json::Value) -> String {
        write_skill_md(plan)
    }
}

/// team-skill-generator 插件:注册 `team-skill-generator` seam。
pub struct TeamSkillGeneratorPlugin;

impl Plugin for TeamSkillGeneratorPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-team-skill-generator"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![TEAM_SKILL_GENERATOR]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let svc: Arc<dyn TeamSkillPlanNormalizer> = Arc::new(TeamSkillGeneratorImpl);
        Ok(vec![ctx.register(TEAM_SKILL_GENERATOR, svc)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::TEAM_SKILL_GENERATOR;
    use ah_hub::plugin::DynPlugin;

    fn build_ctx() -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![Arc::new(TeamSkillGeneratorPlugin)];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[test]
    fn seam_registered() {
        let (ctx, effects) = build_ctx();
        assert!(
            ctx.service::<dyn TeamSkillPlanNormalizer>(&TEAM_SKILL_GENERATOR)
                .is_some()
        );
        drop(effects);
    }

    #[test]
    fn normalize_and_build_via_seam() {
        let (ctx, effects) = build_ctx();
        let svc = ctx
            .service::<dyn TeamSkillPlanNormalizer>(&TEAM_SKILL_GENERATOR)
            .expect("svc");
        let raw = serde_json::json!({
            "roles": [
                {"id": "collector", "purpose": "Collect"},
                {"id": "analyzer", "purpose": "Analyze"},
            ]
        });
        let plan = svc.normalize_plan(&raw, "analyze data").expect("plan");
        assert_eq!(plan["team_name"], "analyze-data-team");
        assert_eq!(plan["roles"].as_array().unwrap().len(), 2);
        let md = svc.build_skill_md(&plan);
        assert!(md.contains("## Roles"));
        drop(effects);
    }
}
