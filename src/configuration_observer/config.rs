use std::{collections::HashMap, env, fs, path::Path};

use anyhow::{anyhow, Context};
use serde::Deserialize;
use url::Url;

use super::{
    namespace_description::{FileLoader, HttpLoader},
    NamespaceDescriptionLoader,
};

#[derive(Deserialize, PartialEq, Eq, Debug)]
pub struct OperatorConfig {
    pub namespaces: HashMap<String, NamespaceConfig>,
}

impl OperatorConfig {
    /// # Errors
    /// Cannot parse config or cannot read from file.
    pub fn from_file<P: AsRef<Path>>(p: P) -> anyhow::Result<Self> {
        let config = fs::read_to_string(p)?;
        Self::from_toml(&config)
    }

    /// # Errors
    /// Cannot parse config.
    pub fn from_toml(config: &str) -> anyhow::Result<Self> {
        Ok(toml::from_str(config)?)
    }

    /// # Panics
    /// Cannot parse config.
    #[must_use]
    pub fn local() -> Self {
        Self::from_toml(
            r#"
                [namespaces.local]
                config_url = "file://namespace.toml"
                registry = { type = "file", path = "skills" }
            "#,
        )
        .unwrap()
    }
}
#[derive(Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Registry {
    File {
        path: String,
    },
    Oci {
        registry: String,
        repository: String,
    },
}

#[derive(Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(untagged)]
pub enum NamespaceConfig {
    TeamOwned {
        config_url: String,
        config_access_token_env_var: Option<String>,
        registry: Registry,
    },
}

impl NamespaceConfig {
    pub fn loader(&self) -> anyhow::Result<Box<dyn NamespaceDescriptionLoader + Send + 'static>> {
        match self {
            NamespaceConfig::TeamOwned {
                config_url,
                config_access_token_env_var,
                // Registry does not change dynamically at the moment
                registry: _,
            } => {
                let url = Url::parse(config_url)?;
                match url.scheme() {
                    "https" | "http" => {
                        let config_access_token = config_access_token_env_var
                            .as_ref()
                            .map(env::var)
                            .transpose()
                            .with_context(|| {
                                format!(
                                    "Missing environment variable: {}",
                                    config_access_token_env_var.as_ref().unwrap()
                                )
                            })?;
                        Ok(Box::new(HttpLoader::from_url(
                            config_url,
                            config_access_token,
                        )))
                    }
                    "file" => {
                        // remove leading "file://"
                        let file_path = &config_url[7..];
                        let loader = FileLoader::new(file_path.into());
                        Ok(Box::new(loader))
                    }
                    scheme => Err(anyhow!("Unsupported URL scheme: {scheme}")),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::configuration_observer::{config::Registry, NamespaceConfig};

    use super::OperatorConfig;

    impl OperatorConfig {
        /// # Panics
        /// Cannot parse config.
        #[must_use]
        pub fn empty() -> Self {
            Self::from_toml("[namespaces]").unwrap()
        }
    }

    #[test]
    fn deserialize_config_with_file_registry() {
        let config = OperatorConfig::local();
        assert!(config.namespaces.contains_key("local"));
    }

    #[test]
    fn deserialize_config_with_oci_registry() {
        let config = OperatorConfig::from_toml(
            r#"
            [namespaces.pharia-kernel-team]
            config_url = "https://dummy_url"
            registry = { type = "oci", registry = "registry.gitlab.aleph-alpha.de", repository = "engineering/pharia-skills/skills" }
            "#,
        ).unwrap();
        assert!(config.namespaces.contains_key("pharia-kernel-team"));
    }

    #[test]
    fn deserialize_config_with_config_access_token() {
        let config = OperatorConfig::from_toml(
            r#"
            [namespaces.dummy_team]
            config_url = "file://dummy_config_url"
            config_access_token_env_var = "GITLAB_CONFIG_ACCESS_TOKEN"
            registry = { type = "file", path = "dummy_file_path" }
            "#,
        )
        .unwrap();
        let namespace_cfg = config.namespaces.get("dummy_team").unwrap();
        let expected = NamespaceConfig::TeamOwned {
            config_url: "file://dummy_config_url".to_owned(),
            config_access_token_env_var: Some("GITLAB_CONFIG_ACCESS_TOKEN".to_owned()),
            registry: Registry::File {
                path: "dummy_file_path".to_owned(),
            },
        };
        assert_eq!(namespace_cfg, &expected);
    }

    #[test]
    fn reads_from_file() {
        let config = OperatorConfig::from_file("operator-config.toml").unwrap();
        assert!(config.namespaces.contains_key("pharia-kernel-team"));
    }

    #[test]
    fn deserializes_new_config() {
        let config = toml::from_str::<OperatorConfig>(
            r#"
            [namespaces.pharia-kernel-team]
            config_url = "https://dummy_url"
            config_access_token_env_var = "GITLAB_CONFIG_ACCESS_TOKEN"
            registry = { type = "oci", registry = "registry.gitlab.aleph-alpha.de", repository = "engineering/pharia-skills/skills" }

            [namespaces.pharia-kernel-team-local]
            config_url = "https://dummy_url"
            config_access_token_env_var = "GITLAB_CONFIG_ACCESS_TOKEN"
            registry = { type = "file", path = "/temp/skills" }
            "#,
        )
        .unwrap();

        let expected = OperatorConfig {
            namespaces: [
                (
                    "pharia-kernel-team".to_owned(),
                    NamespaceConfig::TeamOwned {
                        config_url: "https://dummy_url".to_owned(),
                        config_access_token_env_var: Some("GITLAB_CONFIG_ACCESS_TOKEN".to_owned()),
                        registry: Registry::Oci {
                            registry: "registry.gitlab.aleph-alpha.de".to_owned(),
                            repository: "engineering/pharia-skills/skills".to_owned(),
                        },
                    },
                ),
                (
                    "pharia-kernel-team-local".to_owned(),
                    NamespaceConfig::TeamOwned {
                        config_url: "https://dummy_url".to_owned(),
                        config_access_token_env_var: Some("GITLAB_CONFIG_ACCESS_TOKEN".to_owned()),
                        registry: Registry::File {
                            path: "/temp/skills".to_owned(),
                        },
                    },
                ),
            ]
            .into_iter()
            .collect(),
        };

        assert_eq!(config, expected);
    }
}
