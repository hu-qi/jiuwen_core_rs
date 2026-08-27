use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ah_contracts::ability::{Ability, AbilityError, AbilityExecution, AbilityManager};
use ah_contracts::effect::Effect;
use ah_contracts::keys::{ABILITIES, SKILL};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::skill::{Skill, SkillRegistry};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;

pub struct LocalAbilityManager {
    abilities: Mutex<HashMap<String, Ability>>,
    skills: Arc<dyn SkillRegistry>,
    state_path: PathBuf,
}

impl LocalAbilityManager {
    fn load(skills: Arc<dyn SkillRegistry>, state_path: PathBuf) -> Result<Self, AbilityError> {
        let abilities = if state_path.exists() {
            let text = std::fs::read_to_string(&state_path)
                .map_err(|e| AbilityError(format!("read ability state: {e}")))?;
            serde_json::from_str::<Vec<Ability>>(&text)
                .map_err(|e| AbilityError(format!("parse ability state: {e}")))?
                .into_iter()
                .map(|ability| (ability.id.clone(), ability))
                .collect()
        } else {
            HashMap::new()
        };
        Ok(Self {
            abilities: Mutex::new(abilities),
            skills,
            state_path,
        })
    }

    fn persist(&self, abilities: &HashMap<String, Ability>) -> Result<(), AbilityError> {
        if let Some(parent) = self.state_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| AbilityError(format!("create ability state directory: {e}")))?;
        }
        let mut values: Vec<_> = abilities.values().cloned().collect();
        values.sort_by(|a, b| a.id.cmp(&b.id));
        let text = serde_json::to_string_pretty(&values)
            .map_err(|e| AbilityError(format!("serialize ability state: {e}")))?;
        std::fs::write(&self.state_path, text)
            .map_err(|e| AbilityError(format!("write ability state: {e}")))
    }
}

impl Seam for LocalAbilityManager {}

#[async_trait]
impl AbilityManager for LocalAbilityManager {
    fn register(&self, ability: Ability) -> Result<(), AbilityError> {
        if ability.id.trim().is_empty() || ability.name.trim().is_empty() {
            return Err(AbilityError(
                "ability id/name must not be empty".to_string(),
            ));
        }
        let mut abilities = self.abilities.lock().unwrap();
        if abilities.contains_key(&ability.id) {
            return Err(AbilityError(format!(
                "ability already exists: {}",
                ability.id
            )));
        }
        abilities.insert(ability.id.clone(), ability);
        self.persist(&abilities)?;
        Ok(())
    }

    fn get(&self, id: &str) -> Option<Ability> {
        self.abilities.lock().unwrap().get(id).cloned()
    }

    fn list(&self) -> Vec<Ability> {
        let mut values: Vec<_> = self.abilities.lock().unwrap().values().cloned().collect();
        values.sort_by(|a, b| a.id.cmp(&b.id));
        values
    }

    fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), AbilityError> {
        let mut abilities = self.abilities.lock().unwrap();
        let ability = abilities
            .get_mut(id)
            .ok_or_else(|| AbilityError(format!("ability not found: {id}")))?;
        ability.enabled = enabled;
        self.persist(&abilities)
    }

    async fn execute(&self, id: &str, task: &str) -> Result<AbilityExecution, AbilityError> {
        if task.trim().is_empty() {
            return Err(AbilityError("task must not be empty".to_string()));
        }
        let ability = self
            .get(id)
            .ok_or_else(|| AbilityError(format!("ability not found: {id}")))?;
        if !ability.enabled {
            return Err(AbilityError(format!("ability disabled: {id}")));
        }
        let skill_id = ability
            .skill_id
            .ok_or_else(|| AbilityError(format!("ability has no skill: {id}")))?;
        let evaluation = self
            .skills
            .evaluate(&skill_id, task)
            .await
            .map_err(|e| AbilityError(format!("skill execution failed: {e}")))?;
        Ok(AbilityExecution {
            ability_id: id.to_string(),
            output: evaluation.answer,
        })
    }

    fn skills(&self) -> Vec<Skill> {
        self.skills.list()
    }
}

pub struct AbilityPlugin {
    state_path: PathBuf,
}

impl AbilityPlugin {
    pub fn new(state_path: impl Into<PathBuf>) -> Self {
        Self {
            state_path: state_path.into(),
        }
    }
}

impl Default for AbilityPlugin {
    fn default() -> Self {
        Self::new("runtime/abilities.json")
    }
}

impl Plugin for AbilityPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-ability"
    }
    fn provides(&self) -> Vec<ServiceKey> {
        vec![ABILITIES]
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
        let manager: Arc<dyn AbilityManager> = Arc::new(
            LocalAbilityManager::load(skills, self.state_path.clone()).map_err(|e| {
                PluginError::Apply {
                    plugin: self.name(),
                    message: e.0,
                }
            })?,
        );
        Ok(vec![ctx.register(ABILITIES, manager)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct NoSkills;
    impl Seam for NoSkills {}
    #[async_trait]
    impl SkillRegistry for NoSkills {
        fn create(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: Vec<String>,
        ) -> Result<Skill, ah_contracts::skill::SkillError> {
            Err(ah_contracts::skill::SkillError("unused".into()))
        }
        fn get(&self, _: &str) -> Option<Skill> {
            None
        }
        fn list(&self) -> Vec<Skill> {
            vec![]
        }
        fn remove(&self, _: &str) -> Result<(), ah_contracts::skill::SkillError> {
            Ok(())
        }
        async fn evaluate(
            &self,
            _: &str,
            _: &str,
        ) -> Result<ah_contracts::skill::SkillEvaluation, ah_contracts::skill::SkillError> {
            Err(ah_contracts::skill::SkillError("unused".into()))
        }
    }

    #[test]
    fn plugin_rejects_missing_skill_service() {
        let ctx = Context::new();
        let error = AbilityPlugin::default().apply(&ctx).unwrap_err();
        assert!(error.to_string().contains("skill seam not registered"));
    }

    #[test]
    fn rejects_corrupt_persisted_state() {
        let path =
            std::env::temp_dir().join(format!("ah-ability-corrupt-{}.json", std::process::id()));
        std::fs::write(&path, "not-json").unwrap();
        let result = LocalAbilityManager::load(Arc::new(NoSkills), path.clone());
        assert!(result.is_err());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn persists_and_reloads_enabled_state() {
        let path =
            std::env::temp_dir().join(format!("ah-ability-persist-{}.json", std::process::id()));
        let first = LocalAbilityManager {
            abilities: Mutex::new(HashMap::new()),
            skills: Arc::new(NoSkills),
            state_path: path.clone(),
        };
        first
            .register(Ability {
                id: "persisted".into(),
                name: "Persisted".into(),
                description: "".into(),
                skill_id: None,
                enabled: true,
            })
            .unwrap();
        first.set_enabled("persisted", false).unwrap();
        let second = LocalAbilityManager::load(Arc::new(NoSkills), path.clone()).unwrap();
        assert!(!second.get("persisted").unwrap().enabled);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn validates_and_controls_abilities() {
        let manager = LocalAbilityManager {
            abilities: Mutex::new(HashMap::new()),
            skills: Arc::new(NoSkills),
            state_path: std::env::temp_dir().join("ah-ability-test.json"),
        };
        assert!(
            manager
                .register(Ability {
                    id: "".into(),
                    name: "x".into(),
                    description: "".into(),
                    skill_id: None,
                    enabled: true
                })
                .is_err()
        );
        manager
            .register(Ability {
                id: "a".to_string(),
                name: "A".to_string(),
                description: "".to_string(),
                skill_id: None,
                enabled: true,
            })
            .unwrap();
        assert!(
            manager
                .register(Ability {
                    id: "a".to_string(),
                    name: "A".to_string(),
                    description: "".to_string(),
                    skill_id: None,
                    enabled: true
                })
                .is_err()
        );
        manager.set_enabled("a", false).unwrap();
        assert!(manager.execute("a", "task").await.is_err());
        assert!(manager.execute("a", "").await.is_err());
    }
}
