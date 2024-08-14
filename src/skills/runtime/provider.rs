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

    pub fn allowed(&self, skill: &str) -> bool {
        if let Some(config) = &self.skill_config {
            config.skills().iter().any(|s| s.name == skill)
        } else {
            true
        }
    }

    pub async fn fetch(
        &mut self,
        skill_name: &str,
        engine: &Engine,
    ) -> anyhow::Result<&CachedSkill> {
        if self.allowed(skill_name) {
            self.internal_fetch(skill_name, engine).await
        } else {
            Err(anyhow!("Skill {skill_name} not configured."))
        }
    }

    async fn internal_fetch(
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

    use crate::skills::runtime::config::tests::StubConfig;

    use super::*;

    #[tokio::test]
    async fn skill_component_is_in_config() {
        let skill_config = StubConfig::new(&["greet_skill"]);
        let skill_config = Box::new(skill_config);
        let skill_registry = HashMap::<String, Vec<u8>>::new();
        let provider = SkillProvider::new(Box::new(skill_registry), Some(skill_config));

        let allowed = provider.allowed("greet_skill");

        assert!(allowed);
    }

    #[tokio::test]
    async fn skill_component_not_in_config() {
        let skill_config = StubConfig::new(&[]);
        let skill_config = Box::new(skill_config);
        let skill_registry = HashMap::<String, Vec<u8>>::new();
        let provider = SkillProvider::new(Box::new(skill_registry), Some(skill_config));
        let allowed = provider.allowed("greet_skill");
        assert!(!allowed);
    }

    #[test]
    fn loads_skill_config() {}
}
