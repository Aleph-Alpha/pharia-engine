use std::{collections::HashMap, env};

use anyhow::{anyhow, Context};
use serde_json::Value;

use crate::{
    configuration_observer::{namespace_from_url, Namespace, OperatorConfig},
    registries::{OciRegistry, SkillRegistry},
    skills::SkillPath,
};

use super::{
    engine::{Engine, Skill},
    Csi,
};

pub struct OperatorProvider {
    config: OperatorConfig,
    skill_providers: HashMap<String, SkillProvider>,
}

pub struct SkillProvider {
    skill_registry: Box<dyn SkillRegistry + Send>,
    skill_config: Box<dyn Namespace + Send>,
    skills: HashMap<String, CachedSkill>,
}

impl SkillProvider {
    pub fn new(
        skill_registry: impl SkillRegistry + Send + 'static,
        skill_config: Box<dyn Namespace + Send>,
    ) -> Self {
        SkillProvider {
            skill_registry: Box::new(skill_registry),
            skill_config,
            skills: HashMap::new(),
        }
    }

    pub async fn fetch(&mut self, name: &str, engine: &Engine) -> anyhow::Result<&CachedSkill> {
        if self.configured(name).await {
            self.internal_fetch(name, engine).await
        } else {
            Err(anyhow!("Skill {name} not configured."))
        }
    }

    async fn configured(&mut self, name: &str) -> bool {
        self.skill_config
            .synced_skills()
            .await
            .iter()
            .any(|s| s.name == name)
    }

    async fn internal_fetch(
        &mut self,
        name: &str,
        engine: &Engine,
    ) -> anyhow::Result<&CachedSkill> {
        if !self.skills.contains_key(name) {
            let bytes = self.skill_registry.load_skill(name).await?;
            let bytes = bytes.ok_or_else(|| anyhow!("Sorry, skill {name} not found."))?;
            let skill = CachedSkill::new(engine, bytes)
                .with_context(|| format!("Failed to initialize {name}."))?;
            self.skills.insert(name.to_owned(), skill);
        }
        let skill = self.skills.get(name).unwrap();
        Ok(skill)
    }
}

impl OperatorProvider {
    pub fn new(config: OperatorConfig) -> Self {
        OperatorProvider {
            config,
            skill_providers: HashMap::new(),
        }
    }

    pub fn skills(&self) -> impl Iterator<Item = String> {
        self.skill_providers
            .iter()
            .flat_map(|(namespace, provider)| {
                provider
                    .skills
                    .keys()
                    .map(|name| format!("{}/{name}", namespace.clone()))
            })
            .collect::<Vec<_>>()
            .into_iter()
    }

    pub fn invalidate(&mut self, skill_name: &str) -> bool {
        let path = SkillPath::from_str(skill_name);
        self.skill_providers
            .get_mut(&path.namespace)
            .is_some_and(|skill_provider| skill_provider.skills.remove(&path.name).is_some())
    }

    fn skill_provider(&mut self, namespace: &str) -> anyhow::Result<&mut SkillProvider> {
        let Some(ns) = self.config.namespaces.get(namespace) else {
            return Err(anyhow!("Namespace not configured."));
        };

        if !self.skill_providers.contains_key(namespace) {
            drop(dotenvy::dotenv());
            let username = env::var("SKILL_REGISTRY_USER").unwrap();
            let password = env::var("SKILL_REGISTRY_PASSWORD").unwrap();
            let skill_registry = OciRegistry::new(
                ns.repository.clone(),
                ns.registry.clone(),
                username,
                password,
            );

            let skill_config = namespace_from_url(&ns.config_url)?;
            let skill_provider = SkillProvider::new(skill_registry, skill_config);
            self.skill_providers
                .insert(namespace.to_owned(), skill_provider);
        }

        Ok(self
            .skill_providers
            .get_mut(namespace)
            .expect("Skill provider inserted."))
    }

    pub async fn fetch(
        &mut self,
        skill_name: &str,
        engine: &Engine,
    ) -> anyhow::Result<&CachedSkill> {
        let path = SkillPath::from_str(skill_name);
        let skill_provider = self.skill_provider(&path.namespace)?;

        skill_provider.fetch(&path.name, engine).await
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

    use crate::{configuration_observer::tests::LocalSkillConfig, registries::tests::FileRegistry};

    use super::*;

    impl OperatorProvider {
        fn empty() -> Self {
            let config = OperatorConfig::from_str("[namespaces]").unwrap();
            OperatorProvider::new(config)
        }

        fn with_namespace_and_skill(namespace: &str, skill: &str) -> Self {
            let skill_registry = FileRegistry::new();
            let skill_config = Box::new(
                LocalSkillConfig::from_str(&format!("skills = [{{ name = \"{skill}\" }}]"))
                    .unwrap(),
            );
            let skill_provider = SkillProvider::new(skill_registry, skill_config);

            let mut provider = OperatorProvider::empty();
            provider
                .skill_providers
                .insert(namespace.to_owned(), skill_provider);
            provider
        }
    }

    #[tokio::test]
    async fn skill_component_is_in_config() {
        let mut provider =
            OperatorProvider::with_namespace_and_skill("existing_namespace", "existing_skill");
        let engine = Engine::new().unwrap();

        let result = provider
            .fetch("existing_namespace/existing_skill", &engine)
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn skill_component_not_in_config() {
        let mut provider =
            OperatorProvider::with_namespace_and_skill("existing_namespace", "existing_skill");
        let engine = Engine::new().unwrap();

        let result = provider
            .fetch("existing_namespace/non_existing_skill", &engine)
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn namespace_not_in_config() {
        let mut provider =
            OperatorProvider::with_namespace_and_skill("existing_namespace", "existing_skill");
        let engine = Engine::new().unwrap();

        let result = provider
            .fetch("non_existing_namespace/existing_skill", &engine)
            .await;

        assert!(result.is_err());
    }
}
