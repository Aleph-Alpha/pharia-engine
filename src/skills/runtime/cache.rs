use std::collections::HashMap;

use anyhow::{anyhow, Context, Error};
use wasmtime::{component::Component, Engine};

use crate::registries::SkillRegistry;

pub struct SkillCache {
    components: HashMap<String, CachedComponent>,
    skill_registry: Box<dyn SkillRegistry + Send>,
}

impl SkillCache {
    pub fn new(skill_registry: Box<dyn SkillRegistry + Send>) -> Self {
        SkillCache {
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

    pub async fn fetch(&mut self, skill_name: &str, engine: &Engine) -> Result<&Component, Error> {
        if !self.components.contains_key(skill_name) {
            let bytes = self.skill_registry.load_skill(skill_name).await?;
            let bytes = bytes.ok_or_else(|| anyhow!("Sorry, skill {skill_name} not found."))?;
            let component = Component::new(engine, bytes)
                .with_context(|| format!("Failed to initialize {skill_name}."))?;
            let skill = CachedComponent::new(component);
            self.components.insert(skill_name.to_owned(), skill);
        }
        let skill = self.components.get(skill_name).unwrap();
        Ok(&skill.component)
    }
}

pub struct CachedComponent {
    component: Component,
}

impl CachedComponent {
    pub fn new(component: Component) -> Self {
        Self { component }
    }
}

#[cfg(test)]
mod tests {}
