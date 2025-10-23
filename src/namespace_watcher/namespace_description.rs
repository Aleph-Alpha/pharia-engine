use std::{
    fs::{self, DirEntry},
    path::PathBuf,
};

use anyhow::{Context, anyhow};
use async_trait::async_trait;
use axum::http::HeaderValue;
use reqwest::{StatusCode, header::AUTHORIZATION};
use serde::{Deserialize, Serialize};
use tracing::error;

use crate::{
    http::HttpClient, mcp::McpServerUrl, namespace_watcher::Namespace, tool::NativeToolName,
};

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
    // Experimental configuration for highly specialized Chat Skills. Not stable yet.
    Chat {
        name: String,
        version: String,
        model: String,
        system_prompt: String,
    },
    // This is currently the only type of skill that is communicated to people outside of the team
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
    async fn description(&self) -> NamespaceDescriptionResult;
}

#[derive(Default, Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct NamespaceDescription {
    pub skills: Vec<SkillDescription>,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerUrl>,
    #[serde(default)]
    pub native_tools: Vec<NativeToolName>,
}

impl NamespaceDescription {
    pub fn empty() -> Self {
        Self {
            skills: Vec::new(),
            mcp_servers: Vec::new(),
            native_tools: Vec::new(),
        }
    }

    pub fn from_str(config: &str) -> anyhow::Result<Self> {
        let tc = toml::from_str::<NamespaceDescription>(config)?;
        Ok(tc)
    }
}

pub struct WatchLoader {
    directory: PathBuf,
    mcp_servers: Vec<McpServerUrl>,
    native_tools: Vec<NativeToolName>,
}

impl WatchLoader {
    pub fn new(
        directory: PathBuf,
        mcp_servers: Vec<McpServerUrl>,
        native_tools: Vec<NativeToolName>,
    ) -> Self {
        Self {
            directory,
            mcp_servers,
            native_tools,
        }
    }
}

#[async_trait]
impl NamespaceDescriptionLoader for WatchLoader {
    async fn description(&self) -> NamespaceDescriptionResult {
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
            native_tools: self.native_tools.clone(),
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
    async fn description(&self) -> NamespaceDescriptionResult {
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
    client: HttpClient,
    url: String,
    token: Option<String>,
    namespace: Namespace,
}
impl HttpLoader {
    pub fn from_url(namespace: &Namespace, url: &str, token: Option<String>) -> Self {
        Self {
            // We do not need retries for namespace observing, as we do it continuously anyway.
            client: HttpClient::without_retry(),
            url: url.to_owned(),
            token,
            namespace: namespace.clone(),
        }
    }
}
#[async_trait]
impl NamespaceDescriptionLoader for HttpLoader {
    async fn description(&self) -> NamespaceDescriptionResult {
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
                match e.status().unwrap() {
                    StatusCode::TOO_MANY_REQUESTS => {
                        NamespaceDescriptionError::Recoverable(e.into())
                    }
                    StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                        error!(
                            "The token provided by the operator seems invalid or expired. Please \
                            check that a valid token is configured. This could mean either in the \
                            environment variable NAMESPACES__{ns}__CONFIG_ACCESS_TOKEN or in the \
                            config.toml file as config-access-token for the namespace {ns}",
                            ns = self.namespace
                        );
                        NamespaceDescriptionError::Unrecoverable(e.into())
                    }
                    _ => NamespaceDescriptionError::Unrecoverable(e.into()),
                }
            } else {
                NamespaceDescriptionError::Recoverable(e.into())
            }
        })?;

        let content = resp
            .text()
            .await
            .map_err(|e| NamespaceDescriptionError::Recoverable(e.into()))?;
        let desc = NamespaceDescription::from_str(&content).with_context(|| {
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
    async fn description(&self) -> NamespaceDescriptionResult {
        Ok(self.clone())
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn mcp_servers_are_loaded_from_config() {
        let config = r#"
        skills = []
        mcp-servers = ["localhost:8000", "localhost:8001"]
        "#;
        let tc = NamespaceDescription::from_str(config).unwrap();
        assert_eq!(
            tc.mcp_servers,
            vec!["localhost:8000".into(), "localhost:8001".into()]
        );
    }

    #[test]
    fn native_tools_are_loaded_from_config() {
        let config = r#"
        skills = []
        native-tools = ["add", "subtract", "saboteur"]
        "#;
        let tc = NamespaceDescription::from_str(config).unwrap();
        assert_eq!(
            tc.native_tools,
            vec![
                NativeToolName::Add,
                NativeToolName::Subtract,
                NativeToolName::Saboteur,
            ]
        );
    }

    #[test]
    fn invalid_native_tools_are_rejected() {
        let config = r#"
        skills = []
        native-tools = ["invalid"]
        "#;
        let tc = NamespaceDescription::from_str(config);
        assert!(tc.is_err());
    }

    #[test]
    fn load_skill_list_config_toml() {
        let config = r#"
        skills = [
            {name = "Goofy", tag = "v1.0.0-rc"},
            {type = "chat",name = "Pluto", version = "1", model = "pharia-1-llm-7b-control", system_prompt = "You are a helpful assistant."},
            {name = "Gamma"}
        ]
        "#;
        let tc = NamespaceDescription::from_str(config).unwrap();
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
        let config = r#"
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
            "#;
        let tc = NamespaceDescription::from_str(config).unwrap();
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
