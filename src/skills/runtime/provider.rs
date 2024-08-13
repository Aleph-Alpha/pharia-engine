use std::collections::HashMap;

use anyhow::{anyhow, Context};
use serde_json::Value;

use crate::registries::SkillRegistry;

use super::{
    config::SkillConfig,
    engine::{Engine, Skill},
    Csi,
};

pub struct SkillProvider {
    skills: HashMap<String, CachedSkill>,
    skill_registry: Box<dyn SkillRegistry + Send>,
    skill_config: Option<Box<dyn SkillConfig + Send>>,
}

impl SkillProvider {
    pub fn new(
        skill_registry: Box<dyn SkillRegistry + Send>,
        skill_config: Option<Box<dyn SkillConfig + Send>>,
    ) -> Self {
        SkillProvider {
            skills: HashMap::new(),
            skill_registry,
            skill_config,
        }
    }

    pub fn skills(&self) -> impl Iterator<Item = &str> {
        self.skills.keys().map(String::as_ref)
    }

    pub fn invalidate(&mut self, skill: &str) -> bool {
        self.skills.remove(skill).is_some()
    }

    pub async fn fetch(
        &mut self,
        skill_name: &str,
        engine: &Engine,
    ) -> anyhow::Result<&CachedSkill> {
        if !self.skills.contains_key(skill_name) {
            let bytes = self.skill_registry.load_skill(skill_name).await?;
            let bytes = bytes.ok_or_else(|| anyhow!("Sorry, skill {skill_name} not found."))?;
            let skill = CachedSkill::new(engine, bytes)
                .with_context(|| format!("Failed to initialize {skill_name}."))?;
            self.skills.insert(skill_name.to_owned(), skill);
        }
        let skill = self.skills.get(skill_name).unwrap();
        Ok(skill)
    }
}

pub struct CachedSkill {
    skill: Skill,
}

impl CachedSkill {
    pub fn new(engine: &Engine, bytes: impl AsRef<[u8]>) -> anyhow::Result<Self> {
        let skill = engine.instantiate_pre_skill(bytes)?;
        Ok(Self { skill })
    }

    pub async fn run(
        &self,
        engine: &Engine,
        ctx: Box<dyn Csi + Send>,
        input: Value,
    ) -> anyhow::Result<Value> {
        self.skill.run(engine, ctx, input).await
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn loads_skill_config() {}
}
