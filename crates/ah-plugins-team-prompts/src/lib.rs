//! # ah-plugins-team-prompts
//!
//! 真实 agent_teams 提示组装(1:1 对齐
//! `agent_teams/prompts/team_plan_mode.py` + `bridge_remote_brief.py`):
//! - `build_team_plan_mode_section`:team.plan MODE_INSTRUCTIONS section
//!   (双语模板 + enter_plan_mode 状态 + plan 文件信息,priority=85);
//! - `build_bridge_brief`:bridge 远程执行者简报(双语);
//! - `build_team_overview`:团队名册概览(双语)。
//!
//! 另提供 `team-prompt-loader` seam(对齐 `prompts/loader.py` 的
//! `load_template`):嵌入 scheduler_* 消息模板(双语,`{{ns.field}}` 占位
//! 由 team-message seam 在投递时展开),缺失模板显式报错。
//!
//! 纯函数、无 IO、无 LLM;委托契约层实现。

use std::sync::Arc;

use ah_contracts::keys::TEAM_PROMPTS;
use ah_contracts::prelude::Effect;
use ah_contracts::prompt_builder::PromptSection;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::team_prompts::{
    MemberSummary, PromptLoadError, TeamPromptLoader, TeamPrompts, build_bridge_brief,
    build_team_overview, build_team_plan_mode_section,
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

/// 嵌入的团队消息模板:(名称, 语言, 正文)。
/// 对齐 `prompts/<lang>/<name>.md`(当前覆盖 scheduler_* 消息模板,被
/// team-message seam 在投递时按 `{{ns.field}}` 展开)。
const EMBEDDED_TEMPLATES: &[(&str, &str, &str)] = &[
    // ---- scheduler_task_start ----
    (
        "scheduler_task_start",
        "cn",
        "[任务开工] 任务 [{{task.task_id}}]「{{task.title}}」已由调度框架启动并指派给你，现在开始执行。\n\n## 目标与验收标准\n\n{{task.content}}\n\n## 交付方式\n\n- 完成后调用 `member_complete_task(task_id='{{task.task_id}}')` 提交。\n- 该任务的验证者：{{task.reviewer}}。提交后由调度框架转交验收，你不需要联系验证者。\n- 执行中需要澄清或协调时，用 `send_message` 联系 leader。",
    ),
    (
        "scheduler_task_start",
        "en",
        "[Task Started] Task [{{task.task_id}}] \"{{task.title}}\" was started by the scheduling framework and is assigned to you — begin now.\n\n## Goal and acceptance criteria\n\n{{task.content}}\n\n## How to deliver\n\n- Call `member_complete_task(task_id='{{task.task_id}}')` when done.\n- Reviewers for this task: {{task.reviewer}}. The framework hands your submission over for review — do not contact the reviewers yourself.\n- Use `send_message` to reach the leader if you need clarification or coordination.",
    ),
    // ---- scheduler_task_start_plan ----
    (
        "scheduler_task_start_plan",
        "cn",
        "[任务开工·计划阶段] 任务 [{{task.task_id}}]「{{task.title}}」已由调度框架启动并指派给你。你处于计划模式：先提交执行计划，待 leader 批准后再开始执行。\n\n## 目标与验收标准\n\n{{task.content}}\n\n## 交付方式\n\n- 先用 `submit_plan` 提交执行计划，不要直接动手执行。\n- leader 批准后调度框架会通知你进入执行阶段。",
    ),
    (
        "scheduler_task_start_plan",
        "en",
        "[Task Started · Planning] Task [{{task.task_id}}] \"{{task.title}}\" was started by the scheduling framework and is assigned to you. You run in plan mode: submit an execution plan first and wait for the leader's approval before executing.\n\n## Goal and acceptance criteria\n\n{{task.content}}\n\n## How to deliver\n\n- Submit your execution plan via `submit_plan`; do not start executing yet.\n- Once the leader approves, the framework notifies you to begin execution.",
    ),
    // ---- scheduler_review_request ----
    (
        "scheduler_review_request",
        "cn",
        "[验收指派] 任务 [{{task.task_id}}]「{{task.title}}」的承担者 {{task.assignee}} 已提交交付物（第 {{task.review_round}} 轮验收）。你是该任务的验证者之一，请现在验收。\n\n## 任务目标与验收标准\n\n{{task.content}}\n\n## 验收步骤\n\n1. 对照上面的验收标准检查交付产物。\n2. 调用 `verify_task(task_id='{{task.task_id}}', decision='pass'|'fail')` 投票；`fail` 时在 `feedback` 中写明具体返工要求。\n3. 投票后无需跟进——调度框架按票数判定并推进（本任务验证者：{{task.reviewer}}）。",
    ),
    (
        "scheduler_review_request",
        "en",
        "[Review Assigned] {{task.assignee}} submitted the deliverable of task [{{task.task_id}}] \"{{task.title}}\" (review round {{task.review_round}}). You are one of its reviewers — verify it now.\n\n## Goal and acceptance criteria\n\n{{task.content}}\n\n## How to review\n\n1. Inspect the deliverable against the acceptance criteria above.\n2. Call `verify_task(task_id='{{task.task_id}}', decision='pass'|'fail')` to vote; on `fail`, state the concrete rework requirements in `feedback`.\n3. No follow-up needed after voting — the framework settles by tally (reviewers for this task: {{task.reviewer}}).",
    ),
    // ---- scheduler_review_renudge ----
    (
        "scheduler_review_renudge",
        "cn",
        "[验收催办] 任务 [{{task.task_id}}]「{{task.title}}」第 {{task.review_round}} 轮验收仍在等待你的投票。\n\n## 任务目标与验收标准\n\n{{task.content}}\n\n请尽快调用 `verify_task(task_id='{{task.task_id}}', decision='pass'|'fail')` 完成投票。",
    ),
    (
        "scheduler_review_renudge",
        "en",
        "[Review Reminder] Task [{{task.task_id}}] \"{{task.title}}\" round {{task.review_round}} is still waiting for your vote.\n\n## Goal and acceptance criteria\n\n{{task.content}}\n\nPlease call `verify_task(task_id='{{task.task_id}}', decision='pass'|'fail')` soon.",
    ),
    // ---- scheduler_rework ----
    (
        "scheduler_rework",
        "cn",
        "[验收未过·返工] 任务 [{{task.task_id}}]「{{task.title}}」第 {{task.review_round}} 轮验收未通过（轮数上限 {{param.max_rounds}}），已打回给你返工。\n\n## 验证者反馈\n\n{{param.feedback}}\n\n## 任务目标与验收标准\n\n{{task.content}}\n\n按反馈修复后，再次调用 `member_complete_task(task_id='{{task.task_id}}')` 提交验收。",
    ),
    (
        "scheduler_rework",
        "en",
        "[Review Failed · Rework] Task [{{task.task_id}}] \"{{task.title}}\" failed review round {{task.review_round}} (ceiling {{param.max_rounds}}) and was sent back to you.\n\n## Reviewer feedback\n\n{{param.feedback}}\n\n## Goal and acceptance criteria\n\n{{task.content}}\n\nFix per the feedback, then resubmit via `member_complete_task(task_id='{{task.task_id}}')`.",
    ),
    // ---- scheduler_verified_report ----
    (
        "scheduler_verified_report",
        "cn",
        "[验收通过] 任务 [{{task.task_id}}]「{{task.title}}」已通过验收并标记完成。\n\n请用 `send_message` 向 leader 汇报执行结果：关键产物路径、重要决策、遗留问题。",
    ),
    (
        "scheduler_verified_report",
        "en",
        "[Review Passed] Task [{{task.task_id}}] \"{{task.title}}\" passed review and is completed.\n\nReport the outcome to the leader via `send_message`: key artifact paths, notable decisions, open issues.",
    ),
];

/// 真实模板加载器:嵌入表查表(对齐 loader.py 的 `@cache` 语义——同一
/// (name, language) 恒返回同一正文;缺失显式 Err)。
pub struct EmbeddedTeamPromptLoader;

impl Seam for EmbeddedTeamPromptLoader {}

impl TeamPromptLoader for EmbeddedTeamPromptLoader {
    fn load(&self, name: &str, language: &str) -> Result<String, PromptLoadError> {
        EMBEDDED_TEMPLATES
            .iter()
            .find(|(n, lang, _)| *n == name && *lang == language)
            .map(|(_, _, content)| (*content).to_string())
            .ok_or_else(|| {
                PromptLoadError(format!(
                    "template '{name}' not found for language '{language}'"
                ))
            })
    }
}

/// team-prompts 插件:注册 `team-prompts` + `team-prompt-loader` seam。
pub struct TeamPromptsPlugin;

impl Plugin for TeamPromptsPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-team-prompts"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![TEAM_PROMPTS, ah_contracts::keys::TEAM_PROMPT_LOADER]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let svc: Arc<dyn TeamPrompts> = Arc::new(TeamPromptsImpl);
        let loader: Arc<dyn TeamPromptLoader> = Arc::new(EmbeddedTeamPromptLoader);
        Ok(vec![
            ctx.register(TEAM_PROMPTS, svc),
            ctx.register(ah_contracts::keys::TEAM_PROMPT_LOADER, loader),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::bridge_compose::TeamRole;
    use ah_contracts::keys::{TEAM_PROMPT_LOADER, TEAM_PROMPTS};
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
        assert!(
            ctx.service::<dyn TeamPromptLoader>(&TEAM_PROMPT_LOADER)
                .is_some()
        );
        drop(effects);
    }

    #[test]
    fn loader_loads_scheduler_templates_bilingual() {
        let (ctx, effects) = build_ctx();
        let loader = ctx
            .service::<dyn TeamPromptLoader>(&TEAM_PROMPT_LOADER)
            .expect("loader");
        let cn = loader.load("scheduler_task_start", "cn").expect("cn");
        assert!(cn.contains("[任务开工]"));
        assert!(cn.contains("{{task.task_id}}"));
        assert!(cn.contains("{{task.content}}"));
        let en = loader.load("scheduler_task_start", "en").expect("en");
        assert!(en.contains("[Task Started]"));
        assert!(en.contains("{{task.title}}"));
        // 缺失模板显式报错。
        let missing = loader.load("no_such_template", "cn");
        assert!(missing.is_err());
        let err = missing.unwrap_err();
        assert!(err.0.contains("no_such_template"));
        drop(effects);
    }

    #[test]
    fn loader_returns_stable_content() {
        let (ctx, effects) = build_ctx();
        let loader = ctx
            .service::<dyn TeamPromptLoader>(&TEAM_PROMPT_LOADER)
            .expect("loader");
        let a = loader.load("scheduler_rework", "en").expect("a");
        let b = loader.load("scheduler_rework", "en").expect("b");
        assert_eq!(a, b, "同 (name, language) 恒等");
        assert!(a.contains("[Review Failed · Rework]"));
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
