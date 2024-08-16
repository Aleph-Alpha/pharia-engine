use std::{collections::HashMap, env};

use anyhow::{anyhow, Context};
use serde_json::Value;

use crate::registries::{OciRegistry, SkillRegistry};

use super::{
    engine::{Engine, Skill},
    skill_config::{RemoteSkillConfig, SkillConfig},
    Config, Csi,
};

pub struct NamespaceProvider {
    skills: HashMap<String, CachedSkill>,
    skill_registry: Box<dyn SkillRegistry + Send>,
    config: Option<Config>,
    skill_providers: HashMap<String, SkillProvider>,
}

pub struct SkillProvider {
    skill_registry: Box<dyn SkillRegistry + Send>,
    skill_config: Box<dyn SkillConfig + Send>,
}

impl SkillProvider {
    pub fn new(
        skill_registry: impl SkillRegistry + Send + 'static,
        skill_config: impl SkillConfig + Send + 'static,
    ) -> Self {
        SkillProvider {
            skill_registry: Box::new(skill_registry),
            skill_config: Box::new(skill_config),
        }
    }
}

#[derive(Debug, Clone)]
struct SkillPath {
    pub namespace: String,
    pub name: String,
}

impl SkillPath {
    fn from_str(s: &str) -> Self {
        let (namespace, name) = s
            .split_once('/')
            .unwrap_or_else(|| ("pharia-kernel-team", s));
        Self {
            namespace: namespace.to_owned(),
            name: name.to_owned(),
        }
    }
}
impl NamespaceProvider {
    pub fn new(skill_registry: Box<dyn SkillRegistry + Send>, config: Option<Config>) -> Self {
        NamespaceProvider {
            skills: HashMap::new(),
            skill_registry,
            config,
            skill_providers: HashMap::new(),
        }
    }

    pub fn skills(&self) -> impl Iterator<Item = &str> {
        self.skills.keys().map(String::as_ref)
    }

    pub fn invalidate(&mut self, skill: &str) -> bool {
        self.skills.remove(skill).is_some()
    }

    pub async fn allowed(&mut self, path: SkillPath) -> bool {
        let skill_provider = if let Some(sp) = self.skill_providers.get_mut(&path.namespace) {
            sp
        } else {
            let Some(cfg) = &self.config else {
                //if no config is available then fallback to old behavior in order to be
                //backward compatible
                return true;
            };
            let Some(ns) = cfg.namespaces.get(&path.namespace) else {
                //if config is available but namespace isn't deny skill usage
                return false;
            };
            let username = env::var("SKILL_REGISTRY_USER").unwrap();
            let password = env::var("SKILL_REGISTRY_PASSWORD").unwrap();
            let skill_registry = OciRegistry::new(
                ns.repository.clone(),
                ns.registry.clone(),
                username,
                password,
            );
            let skill_config = RemoteSkillConfig::from_url(&ns.config_url);
            let provider = SkillProvider::new(skill_registry, skill_config);
            self.skill_providers
                .insert(path.namespace.clone(), provider);
            self.skill_providers.get_mut(&path.namespace).unwrap()
        };

        skill_provider
            .skill_config
            .skills()
            .await
            .iter()
            .any(|s| s.name == path.name)
    }

    pub async fn fetch(
        &mut self,
        skill_name: &str,
        engine: &Engine,
    ) -> anyhow::Result<&CachedSkill> {
        let path = SkillPath::from_str(skill_name);
        if self.allowed(path.clone()).await {
            self.internal_fetch(path.clone(), skill_name, engine).await
        } else {
            Err(anyhow!("Skill {path:?} not configured."))
        }
    }

    async fn internal_fetch(
        &mut self,
        path: SkillPath,
        skill_name: &str,
        engine: &Engine,
    ) -> anyhow::Result<&CachedSkill> {
        if !self.skills.contains_key(&path.name) {
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

    use crate::{registries::FileRegistry, skills::runtime::skill_config::LocalSkillConfig};

    use super::*;

    impl NamespaceProvider {
        fn empty() -> Self {
            let skill_registry = HashMap::<String, Vec<u8>>::new();
            let config = Config::from_str("[namespaces]");
            NamespaceProvider::new(Box::new(skill_registry), Some(config))
        }

        fn with_namespace_and_skill(namespace: &str, skill: &str) -> Self {
            let skill_registry = FileRegistry::new();
            let skill_config =
                LocalSkillConfig::from_str(&format!("skills = [{{ name = \"{skill}\" }}]"))
                    .unwrap();
            let skill_provider = SkillProvider::new(skill_registry, skill_config);

            let mut provider = NamespaceProvider::empty();
            provider
                .skill_providers
                .insert(namespace.to_owned(), skill_provider);
            provider
        }
    }

    #[tokio::test]
    async fn skill_component_is_in_config() {
        let mut provider =
            NamespaceProvider::with_namespace_and_skill("existing_namespace", "existing_skill");
        let path = SkillPath {
            namespace: "existing_namespace".to_owned(),
            name: "existing_skill".to_owned(),
        };
        let allowed = provider.allowed(path).await;

        assert!(allowed);
    }

    #[tokio::test]
    async fn skill_component_not_in_config() {
        let mut provider =
            NamespaceProvider::with_namespace_and_skill("existing_namespace", "existing_skill");

        let path = SkillPath {
            namespace: "existing_namespace".to_owned(),
            name: "non_existing_skill".to_owned(),
        };
        let allowed = provider.allowed(path).await;

        assert!(!allowed);
    }

    #[tokio::test]
    async fn namespace_not_in_config() {
        let mut provider =
            NamespaceProvider::with_namespace_and_skill("existing_namespace", "existing_skill");

        let path = SkillPath {
            namespace: "non_existing_namespace".to_owned(),
            name: "existing_skill".to_owned(),
        };
        let allowed = provider.allowed(path).await;

        assert!(!allowed);
    }
}
