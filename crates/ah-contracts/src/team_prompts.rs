//! team_prompts seam:agent_teams 提示模板的确定性组装。
//!
//! 对齐 `openjiuwen/agent_teams/prompts/team_plan_mode.py` +
//! `bridge_remote_brief.py`:
//! - team_plan_mode:`build_team_plan_mode_prompt`(双语模板渲染,
//!   `{enter_plan_mode_status}` / `{plan_file_info}` 占位替换,对齐 Python
//!   `.format()`)、`build_enter_plan_mode_status` / `build_plan_file_info`
//!   状态文本(plan_path 是否存在由调用方经 FS seam 判定后传入)、
//!   `build_team_plan_mode_section`(MODE_INSTRUCTIONS, priority=85);
//! - bridge brief:`build_bridge_brief`(远程执行者简报,双语)/
//!   `build_team_overview`(名册概览,双语)。
//!
//! 纯函数、无 IO、无 LLM;契约零实现。模板正文内嵌常量
//! (对齐 Python 模块级 `TEAM_PLAN_MODE_PROMPT_CN/EN`,即
//! `load_template(...).content.strip()` 的产物)。

use crate::bridge_compose::TeamRole;
use crate::prompt_builder::PromptSection;
use crate::prompt_builder::section_name::MODE_INSTRUCTIONS;
use crate::seam::Seam;
use std::collections::BTreeMap;

/// 语言解析(对齐 `resolve_language` 的确定性投影:仅 "en"/"cn" 受支持,
/// 其余回退默认 "cn";环境变量与配置解析属调用方职责)。
pub fn resolve_language(language: &str) -> &'static str {
    if language == "en" { "en" } else { "cn" }
}

/// team.plan 提示模板(EN,对齐 `prompts/en/team_plan_mode.md` stripped)。
pub const TEAM_PLAN_MODE_PROMPT_EN: &str = r##"Team.plan mode is active. You are the real Team Leader. First produce a team-level plan for the user to approve. Do not build the team, assign tasks, or execute implementation before approval.

## Mandatory Team Execution Semantics

Team.plan means this request must be delivered through the team workflow. No matter how small the task appears, the plan must assume that after user approval the Leader will call `build_team`, create tasks, start members, and delegate delivery to teammates.

Never recommend "no team needed", "do not build a team", "Leader can implement directly", or "exit plan mode and provide code directly". For simple tasks, design the smallest valid team, such as one implementation teammate, one task, and clear acceptance criteria.

You must not modify anything except the plan file. Do not call tools that change repository, configuration, workspace, or team state. You may use read-only tools to understand the context, and ask_user to clarify goals, scope, acceptance criteria, or constraints.

## First Step (Critical)

**Before doing anything else, call the `enter_plan_mode` tool.**

It creates the team plan file and returns its path. All plan content must be written to that file.

{enter_plan_mode_status}

## Plan File Info

{plan_file_info}

This is the only file you may edit. During planning, do not create tasks, call `build_team`, `spawn_teammate`, `update_task`, or use any teammate execution tools.

## Team.plan Workflow

### Phase 1: Understand the Goal

Clarify the user's desired outcome, domain/product/engineering constraints, delivery boundary, acceptance criteria, and risk tolerance. Use ask_user when important information is missing.

### Phase 2: Research the Context

Read relevant code, documents, configuration, prior conventions, or other read-only material. Use task_tool with explore_agent when context gathering would otherwise flood your own context. Do not over-delegate simple checks.

### Phase 3: Design the Team Execution Plan

Plan as a Team Leader, not only as a coding implementer. Cover:

- Team objective and deliverables
- Required members, roles, capabilities, and why they are needed
- Task decomposition, dependencies, and parallel/sequential order
- After approval, the first execution step must be `build_team`, followed by task creation, member spawning, and `send_message` to start execution
- Collaboration handoffs and Leader review/approval checkpoints
- If teammates run in plan_mode, state that each member must submit_plan for Leader approval before executing
- Acceptance criteria, verification, risks, and rollback or fallback approach

When deeper synthesis is useful, use task_tool with plan_agent. That subagent should reason about team execution strategy; it must not create tasks or execute.

### Phase 4: Write the Final Team Plan

Write the final plan to the plan file. Provide the recommended approach, not a catalog of alternatives. The plan must let the user judge how the team will be organized, how work will be split, and how success will be verified. After approval, the Leader should be able to build the team and assign tasks from it.

The plan must not include conclusions like "no team needed", "do not build a team", or "implement directly"; for small tasks, describe the minimal team workflow.

### Phase 5: End Planning

When the plan file is complete, call `exit_plan_mode`. It reads the full plan and returns it for user approval. Do not use ask_user for approval wording such as "is this plan OK?" or "should I start?"; approval must happen via exit_plan_mode.

## Turn Ending Rules (Critical)

Your turn can only end in one of these two ways:

1. Call ask_user to clarify requirements or ask the user to choose between key options
2. Call exit_plan_mode to end planning and request user approval

Do not end the turn without exit_plan_mode once the team plan is complete."##;

/// team.plan 提示模板(CN,对齐 `prompts/cn/team_plan_mode.md` stripped)。
pub const TEAM_PLAN_MODE_PROMPT_CN: &str = r##"Team.plan 模式已激活。你现在是真实的 Team Leader，必须先为用户制定团队级计划并提交审批。在用户审批前，不得创建团队、分配任务或执行实现。

## 强制团队执行语义

Team.plan 表示本轮任务必须通过团队机制交付。无论任务看起来多简单，计划都必须假设用户审批后由 Leader 调用 `build_team` 建立团队，再创建任务、启动成员并由成员完成交付。

禁止建议"不启动团队""无需团队协作""Leader 直接实现""退出 plan 后直接给代码"。简单任务也要设计最小团队，例如 1 个实现成员、1 条任务和清晰验收标准。

除计划文件外，你不得进行任何修改；不得调用会改变仓库、配置、工作区或团队状态的工具。你可以使用只读工具理解背景，也可以使用 ask_user 澄清目标、范围、验收标准或约束。

## 首要步骤（关键）

**在做任何其他事情之前，你必须先调用 `enter_plan_mode` 工具。**

这个工具会创建本轮团队计划文件并返回文件路径。后续所有计划正文都必须写入该文件。

{enter_plan_mode_status}

## Plan 文件信息

{plan_file_info}

这是你唯一允许编辑的文件。不要在 plan 阶段创建任务、调用 `build_team`、`spawn_teammate`、`update_task` 或任何成员执行工具。

## Team.plan 工作流

### Phase 1: 理解目标

明确用户想达成的结果、业务/产品/工程约束、交付边界、验收标准和风险偏好。如果关键信息不足，使用 ask_user 澄清。

### Phase 2: 调研背景

根据任务需要读取代码、文档、配置、历史约定或其他只读资料。需要大量上下文收集时，可以通过 task_tool 委派 explore_agent；不要为了简单检查滥用子代理。

### Phase 3: 设计团队执行方案

以 Team Leader 视角设计团队计划，而不是只写代码实现方案。计划应覆盖：

- 团队目标和交付物
- 需要哪些成员、角色、能力，以及为什么需要
- 任务拆分、依赖关系、并行/串行顺序
- 审批后第一步必须调用 `build_team`，随后 `create_task`、`spawn_teammate`、`send_message` 启动执行
- 成员协作方式、交接点和 Leader 需要审批/检查的节点
- 如果成员执行模式是 plan_mode，说明成员认领任务后需先提交 `submit_plan` 供 Leader 审批
- 验收标准、验证方式、风险和回滚/降级思路

如需进一步推演，可通过 task_tool 调用 plan_agent；它应负责团队方案推演，不是替你直接创建任务或执行实现。

### Phase 4: 写入最终团队计划

将最终计划写入 plan 文件。请写推荐方案，不要堆砌备选方案。内容应能让用户判断"团队要如何组织、怎么分工、如何验收"，也能让审批后的 Leader 据此 build team 并分配任务。

计划中不得包含"无需团队协作""不启动团队""直接实现"一类结论；如果任务很小，写出最小团队执行方案。

### Phase 5: 结束规划阶段

当计划文件完成后，必须调用 `exit_plan_mode`。该工具会读取计划全文并返回给用户审批。不要用 ask_user 问"计划是否 OK"或"是否开始执行"；审批必须通过 exit_plan_mode 完成。

## Turn 结束规则（关键）

你的 turn 只能以下面两种方式结束：

1. 调用 ask_user 澄清需求或让用户在关键选项中选择
2. 调用 exit_plan_mode 结束规划阶段，请求用户审批

不要在未调用 exit_plan_mode 的情况下结束已完成的团队计划。"##;

/// 返回 team.plan 提示模板(对齐 `get_team_plan_mode_prompt`)。
pub fn get_team_plan_mode_prompt(language: &str) -> &'static str {
    if resolve_language(language) == "en" {
        TEAM_PLAN_MODE_PROMPT_EN
    } else {
        TEAM_PLAN_MODE_PROMPT_CN
    }
}

/// enter_plan_mode 状态文本(对齐 `_build_enter_plan_mode_status`)。
///
/// `plan_path_present` 对应 Python 的 `bool(agent.get_plan_file_path(session))`。
pub fn build_enter_plan_mode_status(plan_path_present: bool, language: &str) -> String {
    let lang = resolve_language(language);
    match (lang, plan_path_present) {
        ("en", true) => "enter_plan_mode has been called. Proceed with the workflow.".to_string(),
        ("en", false) => {
            "You have NOT called enter_plan_mode yet. Call it NOW as your first action.".to_string()
        }
        ("cn", true) => "enter_plan_mode 已调用完成。请继续工作流。".to_string(),
        _ => "你尚未调用 enter_plan_mode。请立即调用它作为你的第一个操作。".to_string(),
    }
}

/// plan 文件信息文本(对齐 `_build_plan_file_info` 的确定性部分)。
///
/// `plan_path` 非空时,`plan_path_exists` 对应 Python 的 `plan_path.exists()`(由
/// 调用方经 FS seam 判定后传入,契约保持纯函数)。
pub fn build_plan_file_info(plan_path: &str, plan_path_exists: bool, language: &str) -> String {
    let lang = resolve_language(language);
    if plan_path.is_empty() {
        return match lang {
            "en" => "No plan file yet. Call enter_plan_mode first to create one.".to_string(),
            _ => "暂无 plan 文件。请先调用 enter_plan_mode 创建。".to_string(),
        };
    }
    match (lang, plan_path_exists) {
        ("en", true) => format!(
            "A plan file already exists at {plan_path}. You can read it and make incremental edits using the edit_file tool."
        ),
        ("en", false) => format!(
            "No plan file exists yet. You should create your plan at {plan_path} using the write_file tool."
        ),
        ("cn", true) => {
            format!("计划文件已存在于 {plan_path}。你可以使用 edit_file 工具读取并增量编辑它。")
        }
        _ => format!("计划文件尚不存在。你应该使用 write_file 工具在 {plan_path} 创建计划。"),
    }
}

/// 渲染 team.plan 提示(对齐 `build_team_plan_mode_prompt`)。
///
/// 模板为 `{enter_plan_mode_status}` / `{plan_file_info}` 单花括号占位
/// (Python `.format()` 语义;模板内无其它花括号,故逐占位替换等价)。
pub fn build_team_plan_mode_prompt(
    language: &str,
    enter_plan_mode_status: &str,
    plan_file_info: &str,
) -> String {
    get_team_plan_mode_prompt(language)
        .replace("{enter_plan_mode_status}", enter_plan_mode_status)
        .replace("{plan_file_info}", plan_file_info)
}

/// 构建 team.plan MODE_INSTRUCTIONS section(对齐 `build_team_plan_mode_section`)。
///
/// `plan_path` 非空即视为已调用 enter_plan_mode;`plan_path_exists` 由调用方
/// 判定(对齐 Python 的 `plan_path.exists()`)。
pub fn build_team_plan_mode_section(
    language: &str,
    plan_path: &str,
    plan_path_exists: bool,
) -> PromptSection {
    let lang = resolve_language(language);
    let content = build_team_plan_mode_prompt(
        lang,
        &build_enter_plan_mode_status(!plan_path.is_empty(), lang),
        &build_plan_file_info(plan_path, plan_path_exists, lang),
    );
    PromptSection::new(
        MODE_INSTRUCTIONS,
        BTreeMap::from([(lang.to_string(), content)]),
        85,
    )
}

/// 名册成员摘要(对齐 `MemberSummary`)。
#[derive(Debug, Clone, PartialEq)]
pub struct MemberSummary {
    pub member_name: String,
    pub role: TeamRole,
    pub desc: String,
}

/// 远程执行者简报(对齐 `build_bridge_brief`)。
pub fn build_bridge_brief(member_name: &str, prompt: &str, language: &str) -> String {
    if language == "en" {
        format!(
            "You are {member_name}.\n{prompt}\n\
             You are the EXECUTOR backing a bridge_agent member of the \
             same name on a jiuwen team. Each turn you receive a message \
             from the team, perform the requested work (code, analysis, \
             answer, ...) and return the result as plain text. Your \
             reply will be relayed VERBATIM back to the team by the \
             bridge agent, so respond with the final result directly — \
             no 'I suggest...' framing.\n\
             You do NOT have tools and cannot observe team state. The \
             bridge agent owns all team-facing actions (sending \
             messages, claiming/completing tasks)."
        )
    } else {
        format!(
            "你是 {member_name}。\n{prompt}\n\
             你是 jiuwen 团队中同名 bridge_agent 成员的**实际执行者**。\
             每次你将收到一段来自团队的消息文本，请直接**执行**对应工作\
             （如代码、分析、答案）并返回执行结果文本。你的回复会被 bridge \
             agent **原样**转交给团队，所以请直接给出最终结果，\
             不要使用[建议你这么做]之类的提示性语言。\n\
             你**没有工具**也无法感知团队内部状态——所有与团队的交互\
             （发送消息、认领/完成任务）由 bridge agent 完成。"
        )
    }
}

/// 团队概览文本(对齐 `build_team_overview`)。
pub fn build_team_overview(team_name: &str, members: &[MemberSummary], language: &str) -> String {
    let mut lines = vec![overview_header(team_name, language)];
    for m in members {
        lines.push(format_member_line(m, language));
    }
    lines.push(overview_footer(language));
    lines.join("\n")
}

fn overview_header(team_name: &str, language: &str) -> String {
    if language == "en" {
        format!("Team {team_name} roster:")
    } else {
        format!("团队 {team_name} 当前成员：")
    }
}

fn format_member_line(m: &MemberSummary, language: &str) -> String {
    let desc = if m.desc.is_empty() {
        if language == "en" {
            "(no desc)"
        } else {
            "（无描述）"
        }
    } else {
        &m.desc
    };
    format!("- {} ({}): {desc}", m.member_name, m.role.as_str())
}

fn overview_footer(language: &str) -> String {
    if language == "en" {
        "Use the above when crafting replies; do not assume any other team state.".to_string()
    } else {
        "以上信息供你回答时参考；除此之外的团队状态请勿假设。".to_string()
    }
}

/// team_prompts Seam(Service Definition):提示组装门面。
pub trait TeamPrompts: Seam {
    /// 构建 team.plan MODE_INSTRUCTIONS section。
    fn build_team_plan_mode_section(
        &self,
        language: &str,
        plan_path: &str,
        plan_path_exists: bool,
    ) -> PromptSection;

    /// 构建 bridge 远程执行者简报。
    fn build_bridge_brief(&self, member_name: &str, prompt: &str, language: &str) -> String;

    /// 构建团队概览。
    fn build_team_overview(
        &self,
        team_name: &str,
        members: &[MemberSummary],
        language: &str,
    ) -> String;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_language_falls_back_to_cn() {
        assert_eq!(resolve_language("en"), "en");
        assert_eq!(resolve_language("cn"), "cn");
        assert_eq!(resolve_language("fr"), "cn");
    }

    #[test]
    fn enter_plan_mode_status_bilingual() {
        assert!(build_enter_plan_mode_status(true, "en").contains("has been called"));
        assert!(build_enter_plan_mode_status(false, "en").contains("have NOT called"));
        assert!(build_enter_plan_mode_status(true, "cn").contains("已调用完成"));
        assert!(build_enter_plan_mode_status(false, "cn").contains("尚未调用"));
    }

    #[test]
    fn plan_file_info_variants() {
        assert!(build_plan_file_info("", false, "cn").contains("暂无 plan 文件"));
        assert!(build_plan_file_info("/p/plan.md", true, "en").contains("edit_file"));
        assert!(build_plan_file_info("/p/plan.md", false, "en").contains("write_file"));
        assert!(build_plan_file_info("/p/plan.md", true, "cn").contains("edit_file"));
        assert!(build_plan_file_info("/p/plan.md", false, "cn").contains("write_file"));
    }

    #[test]
    fn plan_mode_prompt_renders_placeholders() {
        let en = build_team_plan_mode_prompt("en", "STATUS", "INFO");
        assert!(en.contains("Team.plan mode is active."));
        assert!(en.contains("STATUS"));
        assert!(en.contains("INFO"));
        assert!(!en.contains("{enter_plan_mode_status}"));
        assert!(!en.contains("{plan_file_info}"));
        let cn = build_team_plan_mode_prompt("cn", "状态", "信息");
        assert!(cn.contains("Team.plan 模式已激活。"));
        assert!(cn.contains("状态"));
        assert!(cn.contains("信息"));
    }

    #[test]
    fn plan_mode_section_shape() {
        let section = build_team_plan_mode_section("en", "/p/plan.md", true);
        assert_eq!(section.name, MODE_INSTRUCTIONS);
        assert_eq!(section.priority, 85);
        assert!(section.content.contains_key("en"));
        let body = section.render("en");
        assert!(body.contains("enter_plan_mode has been called"));
        assert!(body.contains("already exists at /p/plan.md"));
    }

    #[test]
    fn bridge_brief_bilingual() {
        let en = build_bridge_brief("alice", "be helpful", "en");
        assert!(en.starts_with("You are alice."));
        assert!(en.contains("EXECUTOR"));
        let cn = build_bridge_brief("alice", "be helpful", "cn");
        assert!(cn.starts_with("你是 alice。"));
        assert!(cn.contains("实际执行者"));
    }

    #[test]
    fn team_overview_lines() {
        let members = vec![
            MemberSummary {
                member_name: "bob".to_string(),
                role: TeamRole::Teammate,
                desc: "dev".to_string(),
            },
            MemberSummary {
                member_name: "carol".to_string(),
                role: TeamRole::Leader,
                desc: String::new(),
            },
        ];
        let en = build_team_overview("TeamA", &members, "en");
        assert!(en.contains("Team TeamA roster:"));
        assert!(en.contains("- bob (teammate): dev"));
        assert!(en.contains("- carol (leader): (no desc)"));
        assert!(en.contains("do not assume any other team state"));

        let cn = build_team_overview("TeamA", &members, "cn");
        assert!(cn.contains("团队 TeamA 当前成员："));
        assert!(cn.contains("- carol (leader): （无描述）"));
        assert!(cn.contains("请勿假设"));
    }
}
