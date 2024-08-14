use std::{env, path::Path};

use axum::http::HeaderValue;
use reqwest::header::AUTHORIZATION;
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

pub struct RemoteConfig {
    skills: Vec<Skill>,
    token: String,
    url: String,
}

impl RemoteConfig {
    pub async fn from_env() -> anyhow::Result<Self> {
        drop(dotenvy::dotenv());
        let token = env::var("REMOTE_SKILL_CONFIG_TOKEN")?;
        let url = env::var("REMOTE_SKILL_CONFIG_URL")?;
        let skills = vec![];
        let mut config = RemoteConfig { skills, token, url };
        config.fetch().await?;
        Ok(config)
    }

    pub async fn fetch(&mut self) -> anyhow::Result<()> {
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

impl SkillConfig for RemoteConfig {
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

    #[tokio::test]
    async fn load_gitlab_config() {
        // Given a gitlab skill config
        let fut = RemoteConfig::from_env();

        // when fetch skill config
        let config = fut.await.unwrap();

        // then the configured skills must listed in the config
        assert!(!config.skills().is_empty());
    }
}
