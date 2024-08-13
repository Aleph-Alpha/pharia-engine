use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Skill {
    name: String,
}

pub trait SkillConfig {
    fn skills(&self) -> &[Skill];
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TomlConfig {
    skills: Vec<Skill>,
}

impl TomlConfig {
    pub fn new(p: &Path) -> anyhow::Result<Self> {
        let config = std::fs::read_to_string(p)?;
        Self::from_str(&config)
    }

    pub fn from_str(skill_config: &str) -> anyhow::Result<Self> {
        let tc = toml::from_str(skill_config)?;
        Ok(tc)
    }
}

impl SkillConfig for TomlConfig {
    fn skills(&self) -> &[Skill] {
        &self.skills
    }
}

#[cfg(test)]
mod tests {
    use crate::skills::runtime::config::TomlConfig;

    #[test]
    fn load_skill_list_config_toml() {
        let tc: TomlConfig = toml::from_str(
            r#"
            skills = [
                {name = "Goofy"},
                {name = "Pluto"},
                {name = "Gamma"}
            ]
            "#,
        )
        .unwrap();
        assert!(tc.skills.len() == 3);
    }
}
