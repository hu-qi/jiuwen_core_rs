//! # ah-plugins-team-prompts
//!
//! 真实 agent_teams 提示组装(1:1 对齐
//! `agent_teams/prompts/team_plan_mode.py` + `bridge_remote_brief.py`):
//! - `build_team_plan_mode_section`:team.plan MODE_INSTRUCTIONS section
//!   (双语模板 + enter_plan_mode 状态 + plan 文件信息,priority=85);
//! - `build_bridge_brief`:bridge 远程执行者简报(双语);
//! - `build_team_overview`:团队名册概览(双语)。
//!
//! 纯函数、无 IO、无 LLM;委托契约层实现。

use std::sync::Arc;

use ah_contracts::keys::TEAM_PROMPTS;
use ah_contracts::prelude::Effect;
use ah_contracts::prompt_builder::PromptSection;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::team_prompts::{
    MemberSummary, TeamPrompts, build_bridge_brief, build_team_overview,
    build_team_plan_mode_section,
};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 真实实现:委托契约层纯函数。
pub struct TeamPromptsImpl;

impl Seam for TeamPromptsImpl {}

impl TeamPrompts for TeamPromptsImpl {
    fn build_team_plan_mode_section(
        &self,
        language: &str,
        plan_path: &str,
        plan_path_exists: bool,
    ) -> PromptSection {
        build_team_plan_mode_section(language, plan_path, plan_path_exists)
    }

    fn build_bridge_brief(&self, member_name: &str, prompt: &str, language: &str) -> String {
        build_bridge_brief(member_name, prompt, language)
    }

    fn build_team_overview(
        &self,
        team_name: &str,
        members: &[MemberSummary],
        language: &str,
    ) -> String {
        build_team_overview(team_name, members, language)
    }
}

/// team-prompts 插件:注册 `team-prompts` seam。
pub struct TeamPromptsPlugin;

impl Plugin for TeamPromptsPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-team-prompts"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![TEAM_PROMPTS]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let svc: Arc<dyn TeamPrompts> = Arc::new(TeamPromptsImpl);
        Ok(vec![ctx.register(TEAM_PROMPTS, svc)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::bridge_compose::TeamRole;
    use ah_contracts::keys::TEAM_PROMPTS;
    use ah_hub::plugin::DynPlugin;

    fn build_ctx() -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![Arc::new(TeamPromptsPlugin)];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[test]
    fn seam_registered() {
        let (ctx, effects) = build_ctx();
        assert!(ctx.service::<dyn TeamPrompts>(&TEAM_PROMPTS).is_some());
        drop(effects);
    }

    #[test]
    fn plan_mode_section_via_seam() {
        let (ctx, effects) = build_ctx();
        let svc = ctx.service::<dyn TeamPrompts>(&TEAM_PROMPTS).expect("svc");
        let section = svc.build_team_plan_mode_section("en", "/p/plan.md", true);
        assert_eq!(section.priority, 85);
        let body = section.render("en");
        assert!(body.contains("enter_plan_mode has been called"));
        assert!(body.contains("already exists at /p/plan.md"));
        drop(effects);
    }

    #[test]
    fn brief_and_overview_via_seam() {
        let (ctx, effects) = build_ctx();
        let svc = ctx.service::<dyn TeamPrompts>(&TEAM_PROMPTS).expect("svc");
        let brief = svc.build_bridge_brief("alice", "be helpful", "en");
        assert!(brief.starts_with("You are alice."));

        let members = vec![MemberSummary {
            member_name: "bob".to_string(),
            role: TeamRole::Teammate,
            desc: "dev".to_string(),
        }];
        let overview = svc.build_team_overview("TeamA", &members, "cn");
        assert!(overview.contains("- bob (teammate): dev"));
        drop(effects);
    }
}
