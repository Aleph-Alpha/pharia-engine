use std::collections::HashMap;

use crate::skills::SkillPath;

use crate::skill_loader::ConfiguredSkill;
use anyhow::anyhow;
use itertools::Itertools;
use tracing::info;

pub struct SkillConfiguration {
    known_skills: HashMap<SkillPath, String>,
    invalid_namespaces: HashMap<String, anyhow::Error>,
}

impl SkillConfiguration {
    pub fn new() -> Self {
        Self {
            known_skills: HashMap::new(),
            invalid_namespaces: HashMap::new(),
        }
    }

    pub fn upsert_skill(&mut self, skill: ConfiguredSkill) -> bool {
        info!("New or changed skill: {skill}");
        let skill_path = SkillPath::new(&skill.namespace, &skill.name);
        self.known_skills
            .insert(skill_path.clone(), skill.tag)
            .is_some()
    }
    pub fn remove_skill(&mut self, skill_path: &SkillPath) -> bool {
        info!("Removed skill: {skill_path}");
        self.known_skills.remove(skill_path).is_some()
    }
    /// All configured skills, sorted by namespace and name
    pub fn skills(&self) -> impl Iterator<Item = &SkillPath> {
        self.known_skills.keys().sorted_by(|a, b| {
            a.namespace
                .cmp(&b.namespace)
                .then_with(|| a.name.cmp(&b.name))
        })
    }
    pub fn add_invalid_namespace(&mut self, namespace: String, e: anyhow::Error) {
        self.invalid_namespaces.insert(namespace, e);
    }
    pub fn remove_invalid_namespace(&mut self, namespace: &str) {
        self.invalid_namespaces.remove(namespace);
    }
    /// Return the registered tag for a given skill
    pub fn tag(&self, skill_path: &SkillPath) -> anyhow::Result<Option<&str>> {
        if let Some(error) = self.invalid_namespaces.get(&skill_path.namespace) {
            return Err(anyhow!("Invalid namespace: {error}"));
        }
        Ok(self.known_skills.get(skill_path).map(String::as_str))
    }
}
