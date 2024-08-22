use std::{
    collections::{HashMap, HashSet},
    env,
};

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
    known_skills: HashSet<SkillPath>,
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
            known_skills: HashSet::new(),
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
                let username = env::var("SKILL_REGISTRY_USER").unwrap();
                let password = env::var("SKILL_REGISTRY_PASSWORD").unwrap();
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

    pub fn add_skill(&mut self, skill: SkillPath) {
        self.known_skills.insert(skill);
    }

    pub fn remove_skill(&mut self, skill: &SkillPath) {
        self.known_skills.remove(skill);
    }

    pub fn skills(&self) -> impl Iterator<Item = SkillPath> {
        self.known_skills.clone().into_iter()
    }

    pub fn loaded_skills(&self) -> impl Iterator<Item = String> + '_ {
        self.cached_skills.keys().map(ToString::to_string)
    }

    pub fn invalidate(&mut self, skill_path: &SkillPath) -> bool {
        self.cached_skills.remove(skill_path).is_some()
    }

    pub async fn fetch(
        &mut self,
        skill_path: &SkillPath,
        engine: &Engine,
    ) -> anyhow::Result<&CachedSkill> {
        if !self.known_skills.contains(skill_path) {
            return Err(anyhow!("Skill {skill_path} not configured."));
        }

        if !self.cached_skills.contains_key(skill_path) {
            let Some(registry) = self.skill_registries.get(&skill_path.namespace) else {
                return Err(anyhow!(
                    "Namespace {} not configured.",
                    skill_path.namespace
                ));
            };

            let bytes = registry.load_skill(&skill_path.name).await?;
            let bytes =
                bytes.ok_or_else(|| anyhow!("Sorry, skill {} not found.", skill_path.name))?;
            let skill = CachedSkill::new(engine, bytes)
                .with_context(|| format!("Failed to initialize {}.", skill_path.name))?;
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
                config_url: "file://skill_config.toml".to_owned(),
            };
            let mut namespaces = HashMap::new();
            namespaces.insert(skill_path.namespace.clone(), ns_cfg);

            let mut provider = SkillProvider::new(&namespaces);
            provider.add_skill(skill_path.clone());
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
}
