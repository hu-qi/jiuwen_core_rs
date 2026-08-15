//! # ah-plugins-skill
//!
//! 真实技能注册与评估(skill_creator/evaluator):技能 = 描述 + 步骤,文件后端
//! 持久化(dir/{id}.json);评估 = 以步骤为上下文真实委派 subagent 执行任务,
//! 再以 evolving 评估轨迹给出 verdict/score。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use ah_contracts::evolving::EvolvingRuntime;
use ah_contracts::keys::{EVOLVING, SKILL, SUBAGENT};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::skill::{Skill, SkillError, SkillEvaluation, SkillRegistry};
use ah_contracts::subagent::{SubagentRuntime, SubagentSpec};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 真实文件后端技能注册表。
pub struct FileSkillRegistry {
    dir: PathBuf,
    index: Mutex<std::collections::HashMap<String, Skill>>,
    subagent: Arc<dyn SubagentRuntime>,
    evolving: Arc<dyn EvolvingRuntime>,
}

impl FileSkillRegistry {
    /// 打开(或创建)技能目录;加载已有技能。
    pub fn open(
        dir: impl Into<PathBuf>,
        subagent: Arc<dyn SubagentRuntime>,
        evolving: Arc<dyn EvolvingRuntime>,
    ) -> Result<Self, SkillError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)
            .map_err(|e| SkillError(format!("create skill dir failed: {e}")))?;
        let mut index = std::collections::HashMap::new();
        for entry in std::fs::read_dir(&dir)
            .map_err(|e| SkillError(format!("read skill dir failed: {e}")))?
        {
            let entry = entry.map_err(|e| SkillError(format!("entry failed: {e}")))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(id) = name.strip_suffix(".json")
                && let Ok(text) = std::fs::read_to_string(entry.path())
                && let Ok(skill) = serde_json::from_str::<Skill>(&text)
            {
                index.insert(id.to_string(), skill);
            }
        }
        Ok(Self {
            dir,
            index: Mutex::new(index),
            subagent,
            evolving,
        })
    }

    fn path_for(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }
}

impl Seam for FileSkillRegistry {}

#[async_trait]
impl SkillRegistry for FileSkillRegistry {
    fn create(
        &self,
        id: &str,
        name: &str,
        description: &str,
        steps: Vec<String>,
    ) -> Result<Skill, SkillError> {
        if id.is_empty() || name.is_empty() {
            return Err(SkillError("skill id/name must not be empty".to_string()));
        }
        let mut index = self.index.lock().unwrap();
        if index.contains_key(id) {
            return Err(SkillError(format!("skill already exists: {id}")));
        }
        let skill = Skill {
            id: id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            steps,
            created_ms: now_ms(),
        };
        let text = serde_json::to_string(&skill)
            .map_err(|e| SkillError(format!("serialize skill: {e}")))?;
        std::fs::write(self.path_for(id), text)
            .map_err(|e| SkillError(format!("write skill: {e}")))?;
        index.insert(id.to_string(), skill.clone());
        Ok(skill)
    }

    fn get(&self, id: &str) -> Option<Skill> {
        self.index.lock().unwrap().get(id).cloned()
    }

    fn list(&self) -> Vec<Skill> {
        let mut skills: Vec<Skill> = self.index.lock().unwrap().values().cloned().collect();
        skills.sort_by(|a, b| a.id.cmp(&b.id));
        skills
    }

    fn remove(&self, id: &str) -> Result<(), SkillError> {
        let path = self.path_for(id);
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| SkillError(format!("remove skill: {e}")))?;
        }
        self.index.lock().unwrap().remove(id);
        Ok(())
    }

    async fn evaluate(&self, skill_id: &str, task: &str) -> Result<SkillEvaluation, SkillError> {
        let skill = self
            .get(skill_id)
            .ok_or_else(|| SkillError(format!("skill not found: {skill_id}")))?;
        let context = format!(
            "You are applying skill '{}'.\nDescription: {}\nSteps:\n{}",
            skill.name,
            skill.description,
            skill
                .steps
                .iter()
                .enumerate()
                .map(|(i, s)| format!("{}. {s}", i + 1))
                .collect::<Vec<_>>()
                .join("\n")
        );
        let session_id = format!("skill-{skill_id}-{}", task.len());
        let result = self
            .subagent
            .run(SubagentSpec {
                id: session_id.clone(),
                task: task.to_string(),
                context: Some(context),
                budget: Some(6),
                allowed_tools: None,
            })
            .await
            .map_err(|e| SkillError(format!("subagent failed: {e}")))?;
        // evolving 评估真实轨迹(会话日志抽取)。
        let trajectory = self
            .evolving
            .extract_session(task, &session_id)
            .map_err(|e| SkillError(format!("extract trajectory: {e}")))?;
        let evaluation = self
            .evolving
            .evaluate(&trajectory)
            .await
            .map_err(|e| SkillError(format!("evaluate trajectory: {e}")))?;
        Ok(SkillEvaluation {
            skill_id: skill_id.to_string(),
            task: task.to_string(),
            verdict: evaluation.verdict,
            score: evaluation.score,
            answer: result.answer,
            iterations: result.iterations_used,
        })
    }
}

/// skill 插件:注入 subagent + evolving,提供 skill seam。
pub struct SkillPlugin {
    dir: PathBuf,
}

impl SkillPlugin {
    /// 以技能目录创建插件。
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

impl Plugin for SkillPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-skill"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![SKILL]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![SUBAGENT, EVOLVING]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let subagent = ctx
            .service::<dyn SubagentRuntime>(&SUBAGENT)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "subagent seam not registered".to_string(),
            })?;
        let evolving = ctx
            .service::<dyn EvolvingRuntime>(&EVOLVING)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "evolving seam not registered".to_string(),
            })?;
        let registry =
            FileSkillRegistry::open(self.dir.clone(), subagent, evolving).map_err(|e| {
                PluginError::Apply {
                    plugin: self.name(),
                    message: e.0,
                }
            })?;
        let registry: Arc<dyn SkillRegistry> = Arc::new(registry);
        Ok(vec![ctx.register(SKILL, registry)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::SKILL;
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
            StdArc::new(SkillPlugin::new(root.join("skills"))),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[test]
    fn skill_crud_roundtrip_and_reopen() {
        let root = std::env::temp_dir().join(format!("ah-skill-crud-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let skills = ctx.service::<dyn SkillRegistry>(&SKILL).expect("skill");

        skills
            .create(
                "explore",
                "explore workspace",
                "list and read files to understand the workspace",
                vec![
                    "list the workspace".to_string(),
                    "read key files".to_string(),
                ],
            )
            .expect("create");
        assert!(
            skills.create("explore", "x", "y", vec![]).is_err(),
            "dup id errors"
        );
        assert_eq!(
            skills.get("explore").expect("get").name,
            "explore workspace"
        );
        assert_eq!(skills.list().len(), 1);
        skills.remove("explore").expect("remove");
        assert!(skills.get("explore").is_none());
        drop(effects);

        // 重开:持久化恢复(再建一条验证)。
        let reopened = build_ctx(&root);
        let skills2 = reopened
            .0
            .service::<dyn SkillRegistry>(&SKILL)
            .expect("skill");
        assert!(skills2.get("explore").is_none(), "removed stays removed");
        skills2
            .create("persist", "p", "desc", vec![])
            .expect("create");
        drop(reopened.1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn skill_evaluate_runs_real_subagent_and_evolving() {
        let root = std::env::temp_dir().join(format!("ah-skill-eval-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let skills = ctx.service::<dyn SkillRegistry>(&SKILL).expect("skill");
        skills
            .create(
                "listskill",
                "list workspace",
                "list the workspace",
                vec!["list the workspace".to_string()],
            )
            .expect("create");

        let evaluation = skills
            .evaluate("listskill", "list the workspace")
            .await
            .expect("evaluate");
        assert!(evaluation.answer.contains("mock final answer"));
        assert!((0.0..=1.0).contains(&evaluation.score));
        assert!(evaluation.iterations >= 1);
        // 不存在的技能显式报错。
        assert!(skills.evaluate("nope", "x").await.is_err());

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
}
