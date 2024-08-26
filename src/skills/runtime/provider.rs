use std::{collections::HashMap, env};

use anyhow::{anyhow, Context};
use serde_json::Value;

use crate::{
    configuration_observer::NamespaceConfig,
    registries::{FileRegistry, OciRegistry, SkillRegistry},
    skills::SkillPath,
};

use super::{
    engine::{Engine, Skill},
    Csi,
};

pub struct SkillProvider {
    known_skills: HashMap<SkillPath, Option<String>>,
    cached_skills: HashMap<SkillPath, CachedSkill>,
    skill_registries: HashMap<String, Box<dyn SkillRegistry + Send>>,
}

impl SkillProvider {
    pub fn new(namespaces: &HashMap<String, NamespaceConfig>) -> Self {
        let skill_registries = namespaces
            .iter()
            .map(|(k, v)| Self::registry(v).map(|r| (k.clone(), r)))
            .collect::<anyhow::Result<HashMap<_, _>>>()
            .expect("All namespace registry in operator config must be valid.");
        SkillProvider {
            known_skills: HashMap::new(),
            cached_skills: HashMap::new(),
            skill_registries,
        }
    }

    fn registry(
        namespace_config: &NamespaceConfig,
    ) -> anyhow::Result<Box<dyn SkillRegistry + Send>> {
        let registry: Box<dyn SkillRegistry + Send> = match namespace_config {
            NamespaceConfig::File { registry, .. } => Box::new(FileRegistry::with_url(registry)?),

            NamespaceConfig::Oci {
                repository,
                registry,
                ..
            } => {
                drop(dotenvy::dotenv());
                let username = env::var("SKILL_REGISTRY_USER")
                    .expect("SKILL_REGISTRY_USER must be set if OCI registry is used.");
                let password = env::var("SKILL_REGISTRY_PASSWORD")
                    .expect("SKILL_REGISTRY_PASSWORD must be set if OCI registry is used.");
                Box::new(OciRegistry::new(
                    repository.clone(),
                    registry.clone(),
                    username,
                    password,
                ))
            }
        };
        Ok(registry)
    }

    pub fn upsert_skill(&mut self, skill: &SkillPath, tag: Option<String>) {
        if self.known_skills.insert(skill.clone(), tag).is_some() {
            self.invalidate(skill);
        }
    }

    pub fn remove_skill(&mut self, skill: &SkillPath) {
        self.known_skills.remove(skill);
        self.invalidate(skill);
    }

    pub fn skills(&self) -> impl Iterator<Item = &SkillPath> {
        self.known_skills.keys()
    }

    pub fn loaded_skills(&self) -> impl Iterator<Item = &SkillPath> + '_ {
        self.cached_skills.keys()
    }

    pub fn invalidate(&mut self, skill_path: &SkillPath) -> bool {
        self.cached_skills.remove(skill_path).is_some()
    }

    pub async fn fetch(
        &mut self,
        skill_path: &SkillPath,
        engine: &Engine,
    ) -> anyhow::Result<&CachedSkill> {
        if !self.cached_skills.contains_key(skill_path) {
            let Some(tag) = self.known_skills.get(skill_path) else {
                return Err(anyhow!("Skill {skill_path} not configured."));
            };

            let Some(registry) = self.skill_registries.get(&skill_path.namespace) else {
                return Err(anyhow!(
                    "Namespace {} not configured.",
                    skill_path.namespace
                ));
            };

            let bytes = registry
                .load_skill(&skill_path.name, tag.as_deref().unwrap_or("latest"))
                .await?;
            let bytes = bytes.ok_or_else(|| anyhow!("Sorry, skill {skill_path} not found."))?;
            let skill = CachedSkill::new(engine, bytes)
                .with_context(|| format!("Failed to initialize {skill_path}."))?;
            self.cached_skills.insert(skill_path.clone(), skill);
        }
        Ok(self.cached_skills.get(skill_path).expect("Skill present."))
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

    impl SkillProvider {
        fn with_namespace_and_skill(skill_path: &SkillPath) -> Self {
            let ns_cfg = NamespaceConfig::File {
                registry: "file://skills".to_owned(),
                config_url: "file://namespace.toml".to_owned(),
            };
            let mut namespaces = HashMap::new();
            namespaces.insert(skill_path.namespace.clone(), ns_cfg);

            let mut provider = SkillProvider::new(&namespaces);
            provider.upsert_skill(skill_path, None);
            provider
        }
    }

    #[tokio::test]
    async fn skill_component_is_in_config() {
        let skill_path = SkillPath::dummy();
        let mut provider = SkillProvider::with_namespace_and_skill(&skill_path);
        let engine = Engine::new().unwrap();

        let result = provider.fetch(&skill_path, &engine).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn skill_component_not_in_config() {
        let skill_path = SkillPath::dummy();
        let mut provider = SkillProvider::with_namespace_and_skill(&skill_path);
        let engine = Engine::new().unwrap();

        let result = provider
            .fetch(
                &SkillPath::new(&skill_path.namespace, "non_existing_skill"),
                &engine,
            )
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn namespace_not_in_config() {
        let skill_path = SkillPath::dummy();
        let mut provider = SkillProvider::with_namespace_and_skill(&skill_path);
        let engine = Engine::new().unwrap();

        let result = provider
            .fetch(
                &SkillPath::new("non_existing_namespace", &skill_path.name),
                &engine,
            )
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn cached_skill_removed() {
        // given one cached skill
        let skill_path = SkillPath::new("local", "greet_skill");
        let mut provider = SkillProvider::with_namespace_and_skill(&skill_path);
        let engine = Engine::new().unwrap();
        provider.fetch(&skill_path, &engine).await.unwrap();

        // when we remove the skill
        provider.remove_skill(&skill_path);

        // then the skill is no longer cached
        assert!(provider.loaded_skills().next().is_none());
    }
}
