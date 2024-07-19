use std::{collections::HashMap, future::Future};

use anyhow::Error;

use super::wasm::SkillComponent;

pub struct SkillCache {
    components: HashMap<String, SkillComponent>,
}

impl SkillCache {
    pub fn new() -> Self {
        SkillCache {
            components: HashMap::new(),
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
        load_skill: impl Future<Output = Result<SkillComponent, Error>>,
    ) -> Result<&SkillComponent, Error> {
        // Assert skill is in cache
        if !self.components.contains_key(skill_name) {
            let skill = load_skill.await?;
            self.components.insert(skill_name.to_owned(), skill);
        }
        let c = self.components.get(skill_name).unwrap();

        //check for component update

        Ok(c)
    }
}

#[cfg(test)]
mod tests {}
