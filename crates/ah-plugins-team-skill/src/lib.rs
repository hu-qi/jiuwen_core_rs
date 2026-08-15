//! # ah-plugins-team-skill
//!
//! 真实团队技能生成器(对齐 Python rsi/team_skill_generator):
//! - 确定性计划:从任务提取能力词 → 技能名/描述/步骤清单;
//! - 注册到 skill seam(文件持久化);
//! - 验证:在源任务上用 skill 评估(子代理 + evolving 轨迹判定);
//! - 修复重试:验证不合格按 max_repair_attempts 追加修正步骤后重试,
//!   仍失败显式报错。

use std::sync::Arc;

use ah_contracts::keys::{SKILL, TEAM_SKILL};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::skill::{Skill, SkillError, SkillRegistry};
use ah_contracts::team_skill::{
    GenerateTeamSkillRequest, GenerateTeamSkillResult, TeamSkillGenerator,
};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;

/// 已知能力词 → 步骤(真实规则,可扩展)。
const CAPABILITY_STEPS: &[(&str, &[&str])] = &[
    (
        "list",
        &[
            "Enumerate the target location or workspace.",
            "Record each entry with its name.",
            "Report the full list to the user.",
        ],
    ),
    (
        "read",
        &[
            "Open the target file or resource.",
            "Extract the requested content.",
            "Present the content clearly.",
        ],
    ),
    (
        "write",
        &[
            "Prepare the content to write.",
            "Write the content to the target path.",
            "Confirm the write succeeded.",
        ],
    ),
    (
        "search",
        &[
            "Formulate a search query from the task.",
            "Run the search over available knowledge.",
            "Return ranked results.",
        ],
    ),
    (
        "summarize",
        &[
            "Gather the source material.",
            "Condense the key points.",
            "Return a concise summary.",
        ],
    ),
];

fn slugify(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>()
        .join("-")
}

/// 从任务生成确定性技能计划(真实规则:关键词 → 能力 → 步骤)。
fn plan_for(task: &str, repair: u32) -> (String, String, Vec<String>) {
    let lower = task.to_lowercase();
    let matched = CAPABILITY_STEPS
        .iter()
        .filter(|(keyword, _)| lower.contains(keyword))
        .collect::<Vec<_>>();
    let name = matched
        .first()
        .map(|(_, steps)| steps.first().map(|s| slugify(s)).unwrap_or_default())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "general-task".to_string());
    let description = format!("A skill to handle tasks about: {task}");
    let mut steps: Vec<String> = matched
        .iter()
        .flat_map(|(_, steps)| steps.iter().map(|s| s.to_string()))
        .collect();
    if steps.is_empty() {
        steps = vec![
            "Understand the task requirements.".to_string(),
            "Take a first concrete action toward the goal.".to_string(),
            "Verify the result and report back.".to_string(),
        ];
    }
    // 修复重试:追加一条修正步骤(真实可解释)。
    if repair > 0 {
        steps.push(format!(
            "Correct the previous attempt based on feedback (repair #{repair})."
        ));
    }
    (name, description, steps)
}

/// 真实团队技能生成器。
pub struct LocalTeamSkillGenerator {
    skills: Arc<dyn SkillRegistry>,
}

impl LocalTeamSkillGenerator {
    pub fn new(skills: Arc<dyn SkillRegistry>) -> Self {
        Self { skills }
    }

    fn generate_once(
        &self,
        request: &GenerateTeamSkillRequest,
        repair: u32,
    ) -> Result<Skill, SkillError> {
        let (name, description, steps) = plan_for(&request.task, repair);
        let id = request
            .skill_id
            .clone()
            .unwrap_or_else(|| format!("team-{}", slugify(&request.task)));
        self.skills.create(&id, &name, &description, steps)
    }
}

impl Seam for LocalTeamSkillGenerator {}

#[async_trait]
impl TeamSkillGenerator for LocalTeamSkillGenerator {
    async fn generate(
        &self,
        request: &GenerateTeamSkillRequest,
    ) -> Result<GenerateTeamSkillResult, SkillError> {
        if request.task.trim().is_empty() {
            return Err(SkillError("task must not be empty".to_string()));
        }
        let max = request.max_repair_attempts;
        let mut passed_first_try = false;
        let mut last_skill: Option<Skill> = None;

        for repair in 0..=max {
            let skill = self.generate_once(request, repair)?;
            // 验证:在源任务上评估(子代理 + evolving 轨迹判定)。
            let evaluation = self.skills.evaluate(&skill.id, &request.task).await?;
            if evaluation.verdict == ah_contracts::evolving::Verdict::Pass {
                passed_first_try = repair == 0;
                last_skill = Some(skill);
                break;
            }
            last_skill = Some(skill);
            // 未通过:删除本次,下一轮带修复步骤重试。
            let _ = self.skills.remove(&last_skill.as_ref().unwrap().id);
        }
        let Some(skill) = last_skill else {
            return Err(SkillError("generation failed".to_string()));
        };
        // 最终仍不合格:显式报错(不静默接受)。
        let final_eval = self.skills.evaluate(&skill.id, &request.task).await?;
        if final_eval.verdict != ah_contracts::evolving::Verdict::Pass {
            return Err(SkillError(format!(
                "generated skill failed validation after {} repair attempts (score {:.2})",
                max, final_eval.score
            )));
        }
        Ok(GenerateTeamSkillResult {
            skill,
            validation_score: final_eval.score,
            passed_first_try,
        })
    }
}

/// team_skill 插件:注入 skill seam,提供 team-skill seam。
pub struct TeamSkillPlugin;

impl Plugin for TeamSkillPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-team-skill"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![TEAM_SKILL]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![SKILL]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let skills =
            ctx.service::<dyn SkillRegistry>(&SKILL)
                .ok_or_else(|| PluginError::Apply {
                    plugin: self.name(),
                    message: "skill seam not registered".to_string(),
                })?;
        let generator: Arc<dyn TeamSkillGenerator> = Arc::new(LocalTeamSkillGenerator::new(skills));
        Ok(vec![ctx.register(TEAM_SKILL, generator)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::TEAM_SKILL;
    use ah_contracts::team_skill::TeamSkillGenerator;
    use ah_hub::plugin::DynPlugin;
    use std::sync::Arc as StdArc;

    fn build_ctx(root: &std::path::Path) -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let session_dir = root.join("sessions");
        let default_path = session_dir.join("default.jsonl");
        let plugins: Vec<DynPlugin> = vec![
            StdArc::new(ah_plugins_mock::MockPlugin),
            StdArc::new(ah_plugins_tools::ToolsPlugin),
            StdArc::new(ah_plugins_sysop::SysopPlugin::new(root)),
            StdArc::new(ah_plugins_session_log::SessionLogPlugin::new(
                &default_path,
                &session_dir,
            )),
            StdArc::new(ah_plugins_subagent::SubagentPlugin),
            StdArc::new(ah_plugins_evolving::EvolvingPlugin::new(
                root.join("evolving"),
            )),
            StdArc::new(ah_plugins_skill::SkillPlugin::new(root.join("skills"))),
            StdArc::new(TeamSkillPlugin),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[tokio::test]
    async fn generates_skill_from_task_and_registers() {
        let root = std::env::temp_dir().join(format!("ah-tsk-run-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let generator = ctx
            .service::<dyn TeamSkillGenerator>(&TEAM_SKILL)
            .expect("team skill");

        let result = generator
            .generate(&GenerateTeamSkillRequest {
                task: "list the workspace".to_string(),
                skill_id: None,
                max_repair_attempts: 2,
            })
            .await
            .expect("generate");

        assert!(result.skill.steps.len() >= 3, "steps planned");
        assert!(result.validation_score >= 0.0);
        // 已注册到 skill seam。
        let skills = ctx
            .service::<dyn SkillRegistry>(&ah_contracts::keys::SKILL)
            .expect("skills");
        assert!(skills.get(&result.skill.id).is_some(), "registered");

        // 空任务显式报错。
        assert!(
            generator
                .generate(&GenerateTeamSkillRequest {
                    task: "".to_string(),
                    skill_id: None,
                    max_repair_attempts: 0,
                })
                .await
                .is_err()
        );

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn repairs_and_reports_first_try() {
        let root = std::env::temp_dir().join(format!("ah-tsk-repair-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let generator = ctx
            .service::<dyn TeamSkillGenerator>(&TEAM_SKILL)
            .expect("team skill");

        // 确定性计划含关键词 → 步骤;修复步骤在重试时追加。
        let (name, _desc, steps) = plan_for("read the config file", 0);
        assert!(!name.is_empty());
        assert!(steps.iter().any(|s| s.contains("Open the target")));
        let (_, _, steps2) = plan_for("read the config file", 1);
        assert!(steps2.iter().any(|s| s.contains("repair #1")));

        // 生成(可能一次通过或重试,mock 评估下两者皆可)。
        let result = generator
            .generate(&GenerateTeamSkillRequest {
                task: "summarize the findings".to_string(),
                skill_id: Some("team-summary".to_string()),
                max_repair_attempts: 3,
            })
            .await
            .expect("generate");
        assert_eq!(result.skill.id, "team-summary");

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
}
