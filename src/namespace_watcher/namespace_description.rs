use std::{
    fs::{self, DirEntry},
    path::PathBuf,
};

use anyhow::{anyhow, Context};
use async_trait::async_trait;
use axum::http::HeaderValue;
use reqwest::header::AUTHORIZATION;
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum NamespaceDescriptionError {
    #[error(transparent)]
    Recoverable(anyhow::Error),
    #[error("Unrecoverable error loading namespace configuration: {0}")]
    Unrecoverable(anyhow::Error),
}

type NamespaceDescriptionResult = Result<NamespaceDescription, NamespaceDescriptionError>;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct SkillDescription {
    pub name: String,
    pub tag: Option<String>,
}

impl SkillDescription {
    fn from(value: DirEntry) -> anyhow::Result<Self> {
        let name = value
            .path()
            .file_stem()
            .ok_or_else(|| anyhow!("Invalid file name for skill."))?
            .to_str()
            .ok_or_else(|| anyhow!("Invalid UTF-8 name for skill."))?
            .to_owned();
        Ok(Self { name, tag: None })
    }
}

// this is actual more of a namespace skill config
// namespace is not good, as a namespace could list skills,
// but could also load skills
#[async_trait]
pub trait NamespaceDescriptionLoader {
    async fn description(&mut self) -> NamespaceDescriptionResult;
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NamespaceDescription {
    pub skills: Vec<SkillDescription>,
}

impl NamespaceDescription {
    pub fn from_str(config: &str) -> anyhow::Result<Self> {
        let tc = toml::from_str(config)?;
        Ok(tc)
    }
}

pub struct WatchLoader {
    directory: PathBuf,
}

impl WatchLoader {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }
}

#[async_trait]
impl NamespaceDescriptionLoader for WatchLoader {
    async fn description(&mut self) -> NamespaceDescriptionResult {
        if !self.directory.is_dir() {
            return Err(NamespaceDescriptionError::Unrecoverable(anyhow!(
                "The directory to watch '{:?}' is not a directory.",
                self.directory
            )));
        }
        let skills = fs::read_dir(self.directory.clone())
            .map_err(|e| NamespaceDescriptionError::Unrecoverable(e.into()))?
            .filter_map(|result| {
                result
                    .ok()
                    .and_then(|entry| SkillDescription::from(entry).ok())
            })
            .collect();
        Ok(NamespaceDescription { skills })
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
    async fn description(&mut self) -> NamespaceDescriptionResult {
        let config = std::fs::read_to_string(&self.path)
            .with_context(|| format!("Unable to read file {}", self.path.to_string_lossy()))
            .map_err(NamespaceDescriptionError::Unrecoverable)?;
        let desc = NamespaceDescription::from_str(&config)
            .with_context(|| {
                format!(
                "Unable to parse file {} into a valid configuration for a team owned namespace.",
                self.path.to_string_lossy()
            )
            })
            .map_err(NamespaceDescriptionError::Unrecoverable)?;
        Ok(desc)
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
    async fn description(&mut self) -> NamespaceDescriptionResult {
        let mut req_builder = reqwest::Client::new().get(&self.url);
        if let Some(token) = &self.token {
            let mut auth_value = HeaderValue::from_str(&format!("Bearer {token}")).unwrap();
            auth_value.set_sensitive(true);
            req_builder = req_builder.header(AUTHORIZATION, auth_value);
        }
        let resp = req_builder
            .send()
            .await
            .map_err(|e| NamespaceDescriptionError::Recoverable(e.into()))?;

        resp.error_for_status_ref()
            .map_err(|e| NamespaceDescriptionError::Recoverable(e.into()))?;

        let content = resp
            .text()
            .await
            .map_err(|e| NamespaceDescriptionError::Recoverable(e.into()))?;
        let desc = NamespaceDescription::from_str(&content).with_context(|| {
            format!(
                "Unable to parse file at '{}' into a valid configuration for a team owned namespace.",
                self.url
            )
        }).map_err(NamespaceDescriptionError::Recoverable)?;
        Ok(desc)
    }
}

#[async_trait]
impl NamespaceDescriptionLoader for NamespaceDescription {
    async fn description(&mut self) -> NamespaceDescriptionResult {
        Ok(self.clone())
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

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
}
