//! Ability manager seam for callable agent capabilities.

use async_trait::async_trait;

use crate::seam::Seam;
use crate::skill::Skill;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Ability {
    pub id: String,
    pub name: String,
    pub description: String,
    pub skill_id: Option<String>,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AbilityExecution {
    pub ability_id: String,
    pub output: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AbilityError(pub String);

impl core::fmt::Display for AbilityError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for AbilityError {}

#[async_trait]
pub trait AbilityManager: Seam {
    fn register(&self, ability: Ability) -> Result<(), AbilityError>;
    fn get(&self, id: &str) -> Option<Ability>;
    fn list(&self) -> Vec<Ability>;
    fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), AbilityError>;
    async fn execute(&self, id: &str, task: &str) -> Result<AbilityExecution, AbilityError>;
    fn skills(&self) -> Vec<Skill>;
}
