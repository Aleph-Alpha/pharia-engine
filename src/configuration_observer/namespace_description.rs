use std::path::PathBuf;

use async_trait::async_trait;
use axum::http::HeaderValue;
use reqwest::header::AUTHORIZATION;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Skill {
    pub name: String,
    pub tag: Option<String>,
}

// this is actual more of a namespace skill config
// namespace is not good, as a namespace could list skills,
// but could also load skills
#[async_trait]
pub trait NamespaceDescriptionLoader {
    async fn description(&mut self) -> anyhow::Result<NamespaceDescription>;
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NamespaceDescription {
    pub skills: Vec<Skill>,
}

impl NamespaceDescription {
    pub fn from_str(config: &str) -> anyhow::Result<Self> {
        let tc = toml::from_str(config)?;
        Ok(tc)
    }
}

pub struct FileLoader {
    path: PathBuf,
}

impl FileLoader {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

#[async_trait]
impl NamespaceDescriptionLoader for FileLoader {
    async fn description(&mut self) -> anyhow::Result<NamespaceDescription> {
        let config = std::fs::read_to_string(&self.path)?;
        NamespaceDescription::from_str(&config)
    }
}
pub struct HttpLoader {
    url: String,
    token: Option<String>,
}
impl HttpLoader {
    pub fn from_url(url: &str, token: Option<String>) -> Self {
        Self {
            url: url.to_owned(),
            token,
        }
    }
}
#[async_trait]
impl NamespaceDescriptionLoader for HttpLoader {
    async fn description(&mut self) -> anyhow::Result<NamespaceDescription> {
        let mut req_builder = reqwest::Client::new().get(&self.url);
        if let Some(token) = &self.token {
            let mut auth_value = HeaderValue::from_str(&format!("Bearer {token}")).unwrap();
            auth_value.set_sensitive(true);
            req_builder = req_builder.header(AUTHORIZATION, auth_value);
        }
        let resp = req_builder.send().await?;

        resp.error_for_status_ref()?;

        let content = resp.text().await?;
        NamespaceDescription::from_str(&content)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::env;

    impl HttpLoader {
        pub fn pharia_kernel_team() -> Self {
            drop(dotenvy::dotenv());
            let url = "https://gitlab.aleph-alpha.de/api/v4/projects/966/repository/files/config.toml/raw?ref=main";
            let access_token = env::var("GITLAB_CONFIG_ACCESS_TOKEN")
                .expect("GITLAB_CONFIG_ACCESS_TOKEN must be set");
            Self::from_url(url, Some(access_token))
        }
    }

    #[test]
    fn load_skill_list_config_toml() {
        let tc: NamespaceDescription = toml::from_str(
            r#"
            skills = [
                {name = "Goofy", tag = "v1.0.0-rc"},
                {name = "Pluto"},
                {name = "Gamma"}
            ]
            "#,
        )
        .unwrap();
        assert_eq!(tc.skills.len(), 3);
        assert_eq!(tc.skills[0].tag.as_ref().unwrap(), "v1.0.0-rc");
        assert!(tc.skills[1].tag.is_none());
    }

    #[tokio::test]
    async fn load_gitlab_config() {
        // Given a gitlab namespace config
        drop(dotenvy::dotenv());
        let url = "https://gitlab.aleph-alpha.de/api/v4/projects/887/repository/files/namespace.toml/raw?ref=main";
        let access_token =
            env::var("GITLAB_CONFIG_ACCESS_TOKEN").expect("GITLAB_CONFIG_ACCESS_TOKEN must be set");
        let mut config = HttpLoader::from_url(url, Some(access_token));

        // when fetch namespace config
        let description = config.description().await.unwrap();

        // then the configured skills must listed in the config
        assert!(!description.skills.is_empty());
    }
}
