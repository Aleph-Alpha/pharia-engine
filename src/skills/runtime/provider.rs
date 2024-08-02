use std::collections::HashMap;

use anyhow::{anyhow, Context, Error};
use wasmtime::{component::Component, Engine};

use crate::registries::SkillRegistry;

use super::linker::SupportedVersion;

pub struct SkillProvider {
    components: HashMap<String, CachedComponent>,
    skill_registry: Box<dyn SkillRegistry + Send>,
}

impl SkillProvider {
    pub fn new(skill_registry: Box<dyn SkillRegistry + Send>) -> Self {
        SkillProvider {
            components: HashMap::new(),
            skill_registry,
        }
    }

    pub fn skills(&self) -> impl Iterator<Item = &str> {
        self.components.keys().map(String::as_ref)
    }

    pub fn invalidate(&mut self, skill: &str) -> bool {
        self.components.remove(skill).is_some()
    }

    pub async fn fetch(
        &mut self,
        skill_name: &str,
        engine: &Engine,
    ) -> Result<&CachedComponent, Error> {
        if !self.components.contains_key(skill_name) {
            let bytes = self.skill_registry.load_skill(skill_name).await?;
            let bytes = bytes.ok_or_else(|| anyhow!("Sorry, skill {skill_name} not found."))?;
            let skill = CachedComponent::new(engine, bytes)
                .with_context(|| format!("Failed to initialize {skill_name}."))?;
            self.components.insert(skill_name.to_owned(), skill);
        }
        let skill = self.components.get(skill_name).unwrap();
        Ok(skill)
    }
}

pub struct CachedComponent {
    component: Component,
    skill_version: SupportedVersion,
}

impl CachedComponent {
    pub fn new(engine: &Engine, bytes: impl AsRef<[u8]>) -> anyhow::Result<Self> {
        let skill_version = SupportedVersion::extract(&bytes)?;
        let component = Component::new(engine, bytes)?;
        Ok(Self {
            component,
            skill_version,
        })
    }

    pub fn component(&self) -> &Component {
        &self.component
    }

    pub fn skill_version(&self) -> SupportedVersion {
        self.skill_version
    }
}

#[cfg(test)]
mod tests {}
