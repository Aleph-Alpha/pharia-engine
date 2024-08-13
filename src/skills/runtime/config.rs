use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
}

pub trait SkillConfig {
    fn skills(&self) -> &[Skill];
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TomlConfig {
    skills: Vec<Skill>,
}

impl TomlConfig {
    pub fn from_default_file() -> Option<Self> {
        Self::new("skill_config.toml").ok()
    }

    pub fn new<P: AsRef<Path>>(p: P) -> anyhow::Result<Self> {
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
        assert_eq!(tc.skills.len(), 3);
    }
}
