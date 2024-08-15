use std::collections::HashMap;

use anyhow::{anyhow, Context};
use serde_json::Value;

use crate::registries::SkillRegistry;

use super::{
    engine::{Engine, Skill},
    skill_config::{RemoteSkillConfig, SkillConfig},
    Config, Csi,
};

pub struct SkillProvider {
    skills: HashMap<String, CachedSkill>,
    skill_registry: Box<dyn SkillRegistry + Send>,
    config: Option<Config>,
    skill_configs: HashMap<String, Box<dyn SkillConfig + Send>>,
}

impl SkillProvider {
    pub fn new(skill_registry: Box<dyn SkillRegistry + Send>, config: Option<Config>) -> Self {
        SkillProvider {
            skills: HashMap::new(),
            skill_registry,
            config,
            skill_configs: HashMap::new(),
        }
    }

    pub fn skills(&self) -> impl Iterator<Item = &str> {
        self.skills.keys().map(String::as_ref)
    }

    pub fn invalidate(&mut self, skill: &str) -> bool {
        self.skills.remove(skill).is_some()
    }

    pub async fn allowed(&mut self, namespace: &str, skill_name: &str) -> bool {
        let skill_config = if let Some(sc) = self.skill_configs.get_mut(namespace) {
            sc
        } else {
            let Some(cfg) = &self.config else {
                //if no config is available then fallback to old behavior in order to be
                //backward compatible
                return true;
            };
            let Some(ns) = cfg.namespaces.get(namespace) else {
                //if config is available but namespace isn't deny skill usage
                return false;
            };
            let sc = Box::new(RemoteSkillConfig::from_url(&ns.config_url));
            self.skill_configs.insert(namespace.to_owned(), sc);
            self.skill_configs.get_mut(namespace).unwrap()
        };

        skill_config
            .skills()
            .await
            .iter()
            .any(|s| s.name == skill_name)
    }

    pub async fn fetch(
        &mut self,
        skill_name: &str,
        engine: &Engine,
    ) -> anyhow::Result<&CachedSkill> {
        let (ns, sn) = match skill_name.split_once('/') {
            Some(nssn) => nssn,
            None => ("pharia-kernel-team", skill_name),
        };

        if self.allowed(ns, sn).await {
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

    use super::*;

    #[tokio::test]
    async fn skill_component_is_in_config() {
        let skill_registry = HashMap::<String, Vec<u8>>::new();
        let config = Config::from_file("config.toml");
        let mut provider = SkillProvider::new(Box::new(skill_registry), Some(config));

        let allowed = provider.allowed("pharia-kernel-team", "greet_skill").await;

        assert!(allowed);
    }

    #[tokio::test]
    async fn skill_component_not_in_config() {
        let skill_registry = HashMap::<String, Vec<u8>>::new();
        let config = Config::from_file("config.toml");
        let mut provider = SkillProvider::new(Box::new(skill_registry), Some(config));
        let allowed = provider
            .allowed("pharia-kernel-team", "non_existing_skill")
            .await;
        assert!(!allowed);
    }

    #[test]
    fn loads_skill_config() {}
}
