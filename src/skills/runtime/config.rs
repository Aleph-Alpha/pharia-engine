use std::{env, path::Path};

use async_trait::async_trait;
use axum::http::HeaderValue;
use reqwest::header::AUTHORIZATION;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
}

#[async_trait]
pub trait SkillConfig {
    fn skills(&self) -> &[Skill];
    async fn load(&mut self) -> anyhow::Result<()>;
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

#[async_trait]
impl SkillConfig for LocalConfig {
    fn skills(&self) -> &[Skill] {
        &self.skills
    }

    async fn load(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}

pub struct RemoteConfig {
    skills: Vec<Skill>,
    token: String,
    url: String,
}

impl RemoteConfig {
    pub fn from_env() -> Self {
        drop(dotenvy::dotenv());
        let token = env::var("REMOTE_SKILL_CONFIG_TOKEN")
            .expect("Remote skill config token must be provided.");
        let url =
            env::var("REMOTE_SKILL_CONFIG_URL").expect("Remote skill config URL must be provided.");
        let skills = vec![];
        Self { skills, token, url }
    }
}

#[async_trait]
impl SkillConfig for RemoteConfig {
    fn skills(&self) -> &[Skill] {
        &self.skills
    }

    async fn load(&mut self) -> anyhow::Result<()> {
        let mut auth_value = HeaderValue::from_str(&format!("Bearer {}", self.token)).unwrap();
        auth_value.set_sensitive(true);
        let client = reqwest::Client::new();
        let resp = client
            .get(&self.url)
            .header(AUTHORIZATION, auth_value)
            .send()
            .await?;
        let content = resp.text().await?;
        let config = TomlConfig::from_str(&content)?;
        self.skills = config.skills;
        Ok(())
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

    #[async_trait]
    impl SkillConfig for StubConfig {
        fn skills(&self) -> &[Skill] {
            &self.skills
        }

        async fn load(&mut self) -> anyhow::Result<()> {
            Ok(())
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

    #[tokio::test]
    async fn load_gitlab_config() {
        // Given a gitlab skill config
        let mut config = RemoteConfig::from_env();

        // when fetch skill config
        let result = config.load().await;

        // then the configured skills must listed in the config
        assert!(result.is_ok());
        assert!(!config.skills().is_empty());
    }
}
