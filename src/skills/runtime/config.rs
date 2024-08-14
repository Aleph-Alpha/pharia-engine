use std::{env, path::Path};

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
}

pub trait SkillConfig {
    fn skills(&self) -> &[Skill];
}

#[derive(Debug, Serialize, Deserialize)]
struct TomlConfig {
    skills: Vec<Skill>,
}

impl TomlConfig {
    pub fn from_str(skill_config: &str) -> anyhow::Result<Self> {
        let tc = toml::from_str(skill_config)?;
        Ok(tc)
    }
}

pub struct LocalConfig {
    skills: Vec<Skill>,
}

impl LocalConfig {
    pub fn from_default_file() -> Option<Self> {
        Self::new("skill_config.toml").ok()
    }

    pub fn new<P: AsRef<Path>>(p: P) -> anyhow::Result<Self> {
        let config = std::fs::read_to_string(p)?;
        let skills = TomlConfig::from_str(&config)?.skills;
        Ok(Self { skills })
    }
}

impl SkillConfig for LocalConfig {
    fn skills(&self) -> &[Skill] {
        &self.skills
    }
}

pub struct GitlabConfig {
    skills: Vec<Skill>,
    token: String,
    url: String,
}

impl GitlabConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        drop(dotenvy::dotenv());
        let token = env::var("GITLAB_SKILL_CONFIG_TOKEN")?;
        let url = env::var("GITLAB_SKILL_CONFIG_URL")?;
        let skills = vec![];
        let mut config = GitlabConfig { skills, token, url };
        config.fetch()?;
        Ok(config)
    }

    pub fn fetch(&mut self) -> anyhow::Result<()> {
        todo!()
    }
}

impl SkillConfig for GitlabConfig {
    fn skills(&self) -> &[Skill] {
        &self.skills
    }
}

#[cfg(test)]
pub mod tests {
    use crate::skills::runtime::config::TomlConfig;

    use super::*;

    pub struct StubConfig {
        skills: Vec<Skill>,
    }

    impl StubConfig {
        pub fn new(skills: &[&str]) -> Self {
            let skills = skills
                .iter()
                .map(|&s| Skill { name: s.to_owned() })
                .collect();
            Self { skills }
        }
    }

    impl SkillConfig for StubConfig {
        fn skills(&self) -> &[Skill] {
            &self.skills
        }
    }

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

    #[test]
    fn load_gitlab_config() {
        // Given a gitlab skill config
        let mut config = GitlabConfig::from_env().unwrap();

        // when fetch skill config
        config.fetch().unwrap();

        // then the configured skills must listed in the config
        assert!(!config.skills().is_empty());
    }
}
