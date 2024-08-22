use std::{
    collections::{HashMap, HashSet},
    env,
};

use anyhow::{anyhow, Context};
use serde_json::Value;

use crate::{
    configuration_observer::{namespace_from_url, Namespace, NamespaceConfig, OperatorConfig},
    registries::{FileRegistry, OciRegistry, SkillRegistry},
    skills::SkillPath,
};

use super::{
    engine::{Engine, Skill},
    Csi,
};

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

pub struct OperatorProvider {
    config: OperatorConfig,
    skill_providers: HashMap<String, SkillProvider>,
    known_skills: HashSet<SkillPath>,
    cached_skills: HashMap<SkillPath, CachedSkill>,
    skill_registries: HashMap<String, Box<dyn SkillRegistry + Send>>,
}

impl OperatorProvider {
    pub fn new(config: OperatorConfig, namespaces: &HashMap<String, NamespaceConfig>) -> Self {
        let skill_registries = namespaces
            .iter()
            .map(|(k, v)| Self::registry(v).map(|r| (k.clone(), r)))
            .collect::<anyhow::Result<HashMap<_, _>>>()
            .expect("All namespace registry in operator config must be valid.");
        OperatorProvider {
            config,
            skill_providers: HashMap::new(),
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

    pub fn loaded_skills(&self) -> impl Iterator<Item = String> {
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
            let skill_provider = match ns {
                NamespaceConfig::File {
                    registry,
                    config_url,
                } => {
                    let skill_registry = FileRegistry::with_url(registry)?;
                    let skill_config = namespace_from_url(config_url)?;
                    SkillProvider::new(skill_registry, skill_config)
                }
                NamespaceConfig::Oci {
                    repository,
                    registry,
                    config_url,
                } => {
                    drop(dotenvy::dotenv());
                    let username = env::var("SKILL_REGISTRY_USER").unwrap();
                    let password = env::var("SKILL_REGISTRY_PASSWORD").unwrap();
                    let skill_registry =
                        OciRegistry::new(repository.clone(), registry.clone(), username, password);
                    let skill_config = namespace_from_url(config_url)?;
                    SkillProvider::new(skill_registry, skill_config)
                }
            };

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

        if !self.known_skills.contains(&path) {
            return Err(anyhow!("Skill {path} not configured."));
        }

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

    use super::*;

    impl OperatorProvider {
        fn with_namespace_and_skill(skill_path: &SkillPath) -> Self {
            let config = OperatorConfig::from_str("[namespaces]").unwrap();

            let ns_cfg = NamespaceConfig::File {
                registry: "file://skills".to_owned(),
                config_url: "file://skill_config.toml".to_owned(),
            };
            let mut namespaces = HashMap::new();
            namespaces.insert(skill_path.namespace.clone(), ns_cfg);

            let mut provider = OperatorProvider::new(config, &namespaces);
            provider.add_skill(skill_path.clone());
            provider
        }
    }

    #[tokio::test]
    async fn skill_component_is_in_config() {
        let skill_path = SkillPath::dummy();
        let mut provider = OperatorProvider::with_namespace_and_skill(&skill_path);
        let engine = Engine::new().unwrap();

        let result = provider.fetch(&skill_path.to_string(), &engine).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn skill_component_not_in_config() {
        let skill_path = SkillPath::dummy();
        let mut provider = OperatorProvider::with_namespace_and_skill(&skill_path);
        let engine = Engine::new().unwrap();

        let result = provider
            .fetch(
                &format!("{}/non_existing_skill", skill_path.namespace),
                &engine,
            )
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn namespace_not_in_config() {
        let skill_path = SkillPath::dummy();
        let mut provider = OperatorProvider::with_namespace_and_skill(&skill_path);
        let engine = Engine::new().unwrap();

        let result = provider
            .fetch(
                &format!("non_existing_namespace/{}", skill_path.name),
                &engine,
            )
            .await;

        assert!(result.is_err());
    }
}
