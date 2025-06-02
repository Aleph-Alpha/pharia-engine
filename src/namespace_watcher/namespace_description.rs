use std::{
    fs::{self, DirEntry},
    path::PathBuf,
};

use anyhow::{Context, anyhow};
use async_trait::async_trait;
use axum::http::HeaderValue;
use reqwest::{StatusCode, header::AUTHORIZATION};
use serde::{Deserialize, Serialize};

use crate::{http::HttpClient, tool::McpServerUrl};

#[derive(Debug, thiserror::Error)]
pub enum NamespaceDescriptionError {
    #[error(transparent)]
    Recoverable(anyhow::Error),
    #[error("Unrecoverable error loading namespace configuration: {0}")]
    Unrecoverable(anyhow::Error),
}

type NamespaceDescriptionResult = Result<NamespaceDescription, NamespaceDescriptionError>;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum SkillDescription {
    Chat {
        name: String,
        version: String,
        model: String,
        system_prompt: String,
    },
    #[serde(untagged)]
    Programmable {
        name: String,
        #[serde(default = "default_tag")]
        tag: String,
    },
}

fn default_tag() -> String {
    "latest".to_owned()
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
        Ok(Self::Programmable {
            name,
            tag: default_tag(),
        })
    }
}

// this is actual more of a namespace skill config
// namespace is not good, as a namespace could list skills,
// but could also load skills
#[async_trait]
pub trait NamespaceDescriptionLoader {
    async fn description(&self, beta: bool) -> NamespaceDescriptionResult;
}

#[derive(Default, Debug, Serialize, Deserialize, Clone)]
pub struct NamespaceDescription {
    pub skills: Vec<SkillDescription>,
    // Maintain backwards compatibility with the old config format which did not include mcp servers.
    #[serde(default)]
    pub mcp_servers: Vec<McpServerUrl>,
}

impl NamespaceDescription {
    pub fn empty() -> Self {
        Self {
            skills: vec![],
            mcp_servers: vec![],
        }
    }

    pub fn from_str(config: &str, beta: bool) -> anyhow::Result<Self> {
        let tc =
            if beta {
                toml::from_str(config)?
            } else {
                #[derive(Deserialize)]
                struct SkillDescriptionStable {
                    name: String,
                    #[serde(default = "default_tag")]
                    tag: String,
                }

                #[derive(Deserialize)]
                struct NamespaceDescriptionStable {
                    skills: Vec<SkillDescriptionStable>,
                    #[serde(default)]
                    mcp_servers: Vec<McpServerUrl>,
                }

                let tc = toml::from_str::<NamespaceDescriptionStable>(config)?;
                NamespaceDescription {
                    skills: tc
                        .skills
                        .into_iter()
                        .map(|SkillDescriptionStable { name, tag }| {
                            SkillDescription::Programmable { name, tag }
                        })
                        .collect(),
                    mcp_servers: tc.mcp_servers,
                }
            };
        Ok(tc)
    }
}

pub struct WatchLoader {
    directory: PathBuf,
    mcp_servers: Vec<McpServerUrl>,
}

impl WatchLoader {
    pub fn new(directory: PathBuf, mcp_servers: Vec<McpServerUrl>) -> Self {
        Self {
            directory,
            mcp_servers,
        }
    }
}

#[async_trait]
impl NamespaceDescriptionLoader for WatchLoader {
    async fn description(&self, _beta: bool) -> NamespaceDescriptionResult {
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
        Ok(NamespaceDescription {
            skills,
            mcp_servers: self.mcp_servers.clone(),
        })
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
    async fn description(&self, beta: bool) -> NamespaceDescriptionResult {
        let config = std::fs::read_to_string(&self.path)
            .with_context(|| format!("Unable to read file {}", self.path.to_string_lossy()))
            .map_err(NamespaceDescriptionError::Unrecoverable)?;
        let desc = NamespaceDescription::from_str(&config, beta)
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
    client: HttpClient,
    url: String,
    token: Option<String>,
}
impl HttpLoader {
    pub fn from_url(url: &str, token: Option<String>) -> Self {
        Self {
            // We do not need retries for namespace observing, as we do it continuously anyway.
            client: HttpClient::new(false),
            url: url.to_owned(),
            token,
        }
    }
}
#[async_trait]
impl NamespaceDescriptionLoader for HttpLoader {
    async fn description(&self, beta: bool) -> NamespaceDescriptionResult {
        let mut req_builder = self.client.get(&self.url);
        if let Some(token) = &self.token {
            let mut auth_value = HeaderValue::from_str(&format!("Bearer {token}"))
                .with_context(|| format!("Invalid token configured for accessing '{}'.", self.url))
                .map_err(NamespaceDescriptionError::Unrecoverable)?;
            auth_value.set_sensitive(true);
            req_builder = req_builder.header(AUTHORIZATION, auth_value);
        }
        let resp = req_builder
            .send()
            .await
            .map_err(|e| NamespaceDescriptionError::Recoverable(e.into()))?;

        resp.error_for_status_ref().map_err(|e| {
            if e.status()
                .expect("Status must be set if an error is generated because it is set")
                .is_client_error()
            {
                if e.status().unwrap() == StatusCode::TOO_MANY_REQUESTS {
                    NamespaceDescriptionError::Recoverable(e.into())
                } else {
                    NamespaceDescriptionError::Unrecoverable(e.into())
                }
            } else {
                NamespaceDescriptionError::Recoverable(e.into())
            }
        })?;

        let content = resp
            .text()
            .await
            .map_err(|e| NamespaceDescriptionError::Recoverable(e.into()))?;
        let desc = NamespaceDescription::from_str(&content, beta).with_context(|| {
            format!(
                "Unable to parse file at '{}' into a valid configuration for a team owned namespace.",
                self.url
            )
        }).map_err(NamespaceDescriptionError::Unrecoverable)?;
        Ok(desc)
    }
}

#[async_trait]
impl NamespaceDescriptionLoader for NamespaceDescription {
    async fn description(&self, _beta: bool) -> NamespaceDescriptionResult {
        Ok(self.clone())
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn tools_are_loaded_from_config_with_beta_flag() {
        let description = r#"
        skills = []
        mcp_servers = ["localhost:8000", "localhost:8001"]
        "#;
        let tc: NamespaceDescription = NamespaceDescription::from_str(description, true).unwrap();
        assert_eq!(
            tc.mcp_servers,
            vec!["localhost:8000".into(), "localhost:8001".into()]
        );
    }

    #[test]
    fn tools_are_loaded_from_config_without_beta_flag() {
        let description = r#"
        skills = []
        mcp_servers = ["localhost:8000", "localhost:8001"]
        "#;
        let tc: NamespaceDescription = NamespaceDescription::from_str(description, false).unwrap();
        assert_eq!(
            tc.mcp_servers,
            vec!["localhost:8000".into(), "localhost:8001".into()]
        );
    }

    #[test]
    fn load_skill_list_config_toml() {
        let tc: NamespaceDescription = toml::from_str(
            r#"
            skills = [
                {name = "Goofy", tag = "v1.0.0-rc"},
                {type = "chat",name = "Pluto", version = "1", model = "pharia-1-llm-7b-control", system_prompt = "You are a helpful assistant."},
                {name = "Gamma"}
            ]
            "#,
        )
        .unwrap();
        assert_eq!(tc.skills.len(), 3);
        assert!(
            matches!(&tc.skills[0], SkillDescription::Programmable { name, tag } if name == "Goofy" && tag == "v1.0.0-rc")
        );
        assert!(
            matches!(&tc.skills[1], SkillDescription::Chat { name, version, model, system_prompt } if name == "Pluto" && version == "1" && model == "pharia-1-llm-7b-control" && system_prompt == "You are a helpful assistant.")
        );
        assert!(
            matches!(&tc.skills[2], SkillDescription::Programmable { name, tag } if name == "Gamma" && tag == "latest")
        );
    }

    #[test]
    fn load_skill_list_config_toml_alternate_syntax() {
        let tc: NamespaceDescription = toml::from_str(
            r#"
            [[skills]]
            name = "Goofy"
            tag = "v1.0.0-rc"
            [[skills]]
            type = "chat"
            name = "Pluto"
            version = "1"
            model = "pharia-1-llm-7b-control"
            system_prompt = "You are a helpful assistant."
            [[skills]]
            name = "Gamma"
            "#,
        )
        .unwrap();
        assert_eq!(tc.skills.len(), 3);
        assert!(
            matches!(&tc.skills[0], SkillDescription::Programmable { name, tag } if name == "Goofy" && tag == "v1.0.0-rc")
        );
        assert!(
            matches!(&tc.skills[1], SkillDescription::Chat { name, version, model, system_prompt } if name == "Pluto" && version == "1" && model == "pharia-1-llm-7b-control" && system_prompt == "You are a helpful assistant.")
        );
        assert!(
            matches!(&tc.skills[2], SkillDescription::Programmable { name, tag } if name == "Gamma" && tag == "latest")
        );
    }
}
