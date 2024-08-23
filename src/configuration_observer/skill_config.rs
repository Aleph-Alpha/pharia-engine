use std::{env, path::PathBuf};

use anyhow::anyhow;
use async_trait::async_trait;
use axum::http::HeaderValue;
use reqwest::header::AUTHORIZATION;
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
}

#[async_trait]
pub trait Namespace {
    async fn skills(&mut self) -> anyhow::Result<Vec<Skill>>;
}

pub fn namespace_from_url(raw_url: &str) -> anyhow::Result<Box<dyn Namespace + Send + 'static>> {
    let url = Url::parse(raw_url)?;
    match url.scheme() {
        "https" | "http" => Ok(Box::new(RemoteNamespace::from_url(raw_url))),
        "file" => {
            // remove leading "file://"
            let file_path = &raw_url[7..];

            let skill_config = LocalNamespace::new(file_path.into());

            Ok(Box::new(skill_config))
        }
        scheme => Err(anyhow!("Unsupported URL scheme: {scheme}")),
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct SkillConfig {
    skills: Vec<Skill>,
}

impl SkillConfig {
    pub fn from_str(skill_config: &str) -> anyhow::Result<Self> {
        let tc = toml::from_str(skill_config)?;
        Ok(tc)
    }
}

pub struct LocalNamespace {
    path: PathBuf,
}

impl LocalNamespace {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

#[async_trait]
impl Namespace for LocalNamespace {
    async fn skills(&mut self) -> anyhow::Result<Vec<Skill>> {
        let config = std::fs::read_to_string(&self.path)?;
        let skills = SkillConfig::from_str(&config)?.skills;
        Ok(skills)
    }
}

pub struct RemoteNamespace {
    token: Option<String>,
    url: String,
}

impl RemoteNamespace {
    pub fn from_url(url: &str) -> Self {
        drop(dotenvy::dotenv());
        let token = env::var("TEAM_CONFIG_TOKEN").ok();
        Self {
            token,
            url: url.to_owned(),
        }
    }
}

#[async_trait]
impl Namespace for RemoteNamespace {
    async fn skills(&mut self) -> anyhow::Result<Vec<Skill>> {
        let mut req_builder = reqwest::Client::new().get(&self.url);
        if let Some(token) = &self.token {
            let mut auth_value = HeaderValue::from_str(&format!("Bearer {token}")).unwrap();
            auth_value.set_sensitive(true);
            req_builder = req_builder.header(AUTHORIZATION, auth_value);
        }
        let resp = req_builder.send().await?;

        resp.error_for_status_ref()?;

        let content = resp.text().await?;
        let config = SkillConfig::from_str(&content)?;
        Ok(config.skills)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    impl RemoteNamespace {
        pub fn pharia_kernel_team() -> Self {
            drop(dotenvy::dotenv());
            let url = "https://gitlab.aleph-alpha.de/api/v4/projects/966/repository/files/config.toml/raw?ref=main";
            Self::from_url(url)
        }
    }

    #[test]
    fn load_skill_list_config_toml() {
        let tc: SkillConfig = toml::from_str(
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
        let mut config = RemoteNamespace::pharia_kernel_team();

        // when fetch skill config
        let skills = config.skills().await.unwrap();

        // then the configured skills must listed in the config
        assert!(!skills.is_empty());
    }
}
