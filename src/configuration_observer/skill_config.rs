use std::{env, path::Path};

use anyhow::anyhow;
use async_trait::async_trait;
use axum::http::HeaderValue;
use reqwest::header::AUTHORIZATION;
use serde::{Deserialize, Serialize};
use tracing::warn;
use url::Url;

#[derive(Debug, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
}

#[async_trait]
pub trait Namespace {
    async fn skills(&mut self) -> anyhow::Result<&[Skill]>;
}

pub fn namespace_from_url(raw_url: &str) -> anyhow::Result<Box<dyn Namespace + Send + 'static>> {
    let url = Url::parse(raw_url)?;
    match url.scheme() {
        "https" | "http" => Ok(Box::new(RemoteSkillConfig::from_url(raw_url))),
        "file" => {
            // remove leading "file://"
            let file_path = &raw_url[7..];

            let skill_config = LocalSkillConfig::new(file_path)?;

            Ok(Box::new(skill_config))
        }
        scheme => Err(anyhow!("Unsupported URL scheme: {scheme}")),
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct TomlSkillConfig {
    skills: Vec<Skill>,
}

impl TomlSkillConfig {
    pub fn from_str(skill_config: &str) -> anyhow::Result<Self> {
        let tc = toml::from_str(skill_config)?;
        Ok(tc)
    }
}

pub struct LocalSkillConfig {
    skills: Vec<Skill>,
}

impl LocalSkillConfig {
    pub fn new<P: AsRef<Path>>(p: P) -> anyhow::Result<Self> {
        let config = std::fs::read_to_string(p)?;
        Self::from_str(&config)
    }

    pub fn from_str(config: &str) -> anyhow::Result<Self> {
        let skills = TomlSkillConfig::from_str(config)?.skills;
        Ok(Self { skills })
    }
}

#[async_trait]
impl Namespace for LocalSkillConfig {
    async fn skills(&mut self) -> anyhow::Result<&[Skill]> {
        Ok(&self.skills)
    }
}

pub struct RemoteSkillConfig {
    skills: Vec<Skill>,
    token: Option<String>,
    url: String,
}

impl RemoteSkillConfig {
    pub fn from_url(url: &str) -> Self {
        drop(dotenvy::dotenv());
        let token = env::var("TEAM_CONFIG_TOKEN").ok();
        let skills = vec![];
        Self {
            skills,
            token,
            url: url.to_owned(),
        }
    }

    async fn load(&self) -> anyhow::Result<Vec<Skill>> {
        let mut req_builder = reqwest::Client::new().get(&self.url);
        if let Some(token) = &self.token {
            let mut auth_value = HeaderValue::from_str(&format!("Bearer {token}")).unwrap();
            auth_value.set_sensitive(true);
            req_builder = req_builder.header(AUTHORIZATION, auth_value);
        }
        let resp = req_builder.send().await?;

        let content = resp.text().await?;
        let config = TomlSkillConfig::from_str(&content)?;
        Ok(config.skills)
    }

    async fn sync(&mut self) {
        match self.load().await {
            Ok(skills) => {
                self.skills = skills;
            }
            Err(e) => {
                warn!("Failed to load remote skill config, fallback to existing config: {e}");
            }
        }
    }
}

#[async_trait]
impl Namespace for RemoteSkillConfig {
    async fn skills(&mut self) -> anyhow::Result<&[Skill]> {
        self.sync().await;
        Ok(&self.skills)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    impl RemoteSkillConfig {
        pub fn pharia_kernel_team() -> Self {
            drop(dotenvy::dotenv());
            let url = "https://gitlab.aleph-alpha.de/api/v4/projects/966/repository/files/config.toml/raw?ref=main";
            Self::from_url(url)
        }
    }

    #[test]
    fn load_skill_list_config_toml() {
        let tc: TomlSkillConfig = toml::from_str(
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
        let mut config = RemoteSkillConfig::pharia_kernel_team();

        // when fetch skill config
        let result = config.load().await;

        // then the configured skills must listed in the config
        assert!(result.is_ok());
        assert!(!config.skills().await.unwrap().is_empty());
    }
}
