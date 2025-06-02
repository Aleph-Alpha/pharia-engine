use anyhow::anyhow;
use derive_more::derive::{Constructor, Deref, IntoIterator};
use heck::ToKebabCase;
use serde::{Deserialize, Deserializer};
use std::{collections::HashMap, path::PathBuf};
use url::Url;
use utoipa::ToSchema;

use crate::skill_loader::RegistryConfig;

use super::{
    NamespaceDescriptionLoader,
    namespace_description::{
        FileLoader, HttpLoader, NamespaceDescription, SkillDescription, WatchLoader,
    },
};

#[derive(PartialEq, Eq, Hash, Debug, Clone, Deref, derive_more::Display, ToSchema)]
#[cfg_attr(test, derive(fake::Dummy))]
#[schema(pattern = "^[a-z0-9]+(-[a-z0-9]+)*$`")]
pub struct Namespace(#[cfg_attr(test, dummy(expr = "\"dummy\".into()"))] String);

impl Namespace {
    // Reasoning to choose 64 is it seems reasonable and we can increase it if there is a need.
    const MAX_LEN: usize = 64;

    pub fn new(input: impl Into<String>) -> anyhow::Result<Self> {
        let input = input.into();
        if Self::is_valid(&input) {
            Ok(Self(input))
        } else {
            Err(anyhow!(format!(
                "Invalid namespace `{input}`. Namespaces must be between 1 and 64 characters long and must follow the pattern `^[a-z0-9]+(-[a-z0-9]+)*$`"
            )))
        }
    }

    fn is_valid(input: &str) -> bool {
        !input.is_empty()
            && input == input.to_kebab_case()
            && input
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            && input.len() <= Self::MAX_LEN
    }
}

impl<'de> Deserialize<'de> for Namespace {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Namespace::new(s).map_err(serde::de::Error::custom)
    }
}

#[derive(Deserialize, PartialEq, Eq, Debug, Clone, Default, Constructor, IntoIterator, Deref)]
#[serde(transparent)]
pub struct NamespaceConfigs(HashMap<Namespace, NamespaceConfig>);

impl NamespaceConfigs {
    /// Create an operator config which checks the local `skills` directory for
    /// a list of skills that are provided in the `skills` argument.
    /// Compared to the `NamespaceConfig::TeamOwned` variant, this removes one
    /// level of indirection (namespace config), allowing for easier testing.
    #[must_use]
    pub fn local(skills: &[&str]) -> Self {
        NamespaceConfigs::new(
            [(
                Namespace::new("local").unwrap(),
                NamespaceConfig::InPlace {
                    skills: skills
                        .iter()
                        .map(|&name| SkillDescription::Programmable {
                            name: name.to_owned(),
                            tag: "latest".to_owned(),
                        })
                        .collect(),
                    registry: Registry::File {
                        path: "skills".to_owned(),
                    },
                },
            )]
            .into(),
        )
    }

    /// Which namespaces is backed by which registry
    #[must_use]
    pub fn registry_config(&self) -> RegistryConfig {
        RegistryConfig::new(
            self.iter()
                .map(|(k, v)| (k.to_owned(), v.registry()))
                .collect(),
        )
    }

    #[must_use]
    pub fn dev() -> Self {
        let namespaces = [(
            Namespace::new("dev").unwrap(),
            NamespaceConfig::Watch {
                directory: "skills".into(),
            },
        )]
        .into();
        Self::new(namespaces)
    }
}

#[derive(Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(untagged)]
pub enum Registry {
    // https://serde.rs/enum-representations.html#untagged
    // Serde will try to match the data against each variant in order and the first one that
    // deserializes successfully is the one returned.
    //
    // Therefore, we put `Oci` first as this is most likely the variant someone will use in production.
    #[serde(rename_all = "kebab-case")]
    Oci {
        registry: String,
        base_repository: String,
        #[serde(rename = "registry-user")]
        user: String,
        #[serde(rename = "registry-password")]
        password: String,
    },
    File {
        path: String,
    },
}

#[derive(Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(untagged)]
pub enum NamespaceConfig {
    /// Namespaces are our way to enable teams to deploy skills in self service via Git Ops. This
    /// implies that the skills in team owned namespaces are configured by a team rather than the
    /// operators of `PhariaKernel`, which in turn means we only refer the teams documentation here.
    #[serde(rename_all = "kebab-case")]
    TeamOwned {
        config_url: String,
        config_access_token: Option<String>,
        #[serde(flatten)]
        registry: Registry,
    },
    /// For development it is convenient to just watch a local repository for changing skills
    /// without the need for reconfiguration.
    Watch { directory: PathBuf },
    /// Rather than referencing a configuration where skills are listed, this variant just lists
    /// them in place in the application config. As such these skills are owned by the operators.
    /// This behavior is especially useful to make sure certain skills are found in integration
    /// tests.
    InPlace {
        skills: Vec<SkillDescription>,
        #[serde(flatten)]
        registry: Registry,
    },
}

impl NamespaceConfig {
    pub fn registry(&self) -> Registry {
        match self {
            NamespaceConfig::Watch { directory } => Registry::File {
                path: directory.to_str().unwrap().to_owned(),
            },
            NamespaceConfig::InPlace { registry, .. }
            | NamespaceConfig::TeamOwned { registry, .. } => registry.clone(),
        }
    }

    pub fn loader(
        &self,
    ) -> anyhow::Result<Box<dyn NamespaceDescriptionLoader + Send + Sync + 'static>> {
        match self {
            NamespaceConfig::TeamOwned {
                config_url,
                config_access_token,
                // Registry does not change dynamically at the moment
                registry: _,
            } => {
                let url = Url::parse(config_url)?;
                match url.scheme() {
                    "https" | "http" => Ok(Box::new(HttpLoader::from_url(
                        config_url,
                        config_access_token.clone(),
                    ))),
                    "file" => {
                        // remove leading "file://"
                        let file_path = &config_url[7..];
                        let loader = FileLoader::new(file_path.into());
                        Ok(Box::new(loader))
                    }
                    scheme => Err(anyhow!("Unsupported URL scheme: {scheme}")),
                }
            }
            NamespaceConfig::Watch { directory } => {
                Ok(Box::new(WatchLoader::new(directory.to_owned())))
            }
            NamespaceConfig::InPlace {
                skills,
                registry: _,
            } => Ok(Box::new(NamespaceDescription {
                skills: skills.clone(),
                mcp_servers: vec![],
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl NamespaceConfigs {
        /// # Panics
        /// Cannot parse config.
        #[must_use]
        pub fn empty() -> Self {
            Self::from_toml("[namespaces]").unwrap()
        }

        /// # Errors
        /// Cannot parse config.
        pub fn from_toml(config: &str) -> anyhow::Result<Self> {
            Ok(toml::from_str(config)?)
        }
    }

    #[test]
    fn long_namespace_is_rejected() {
        // Given a very long string
        let name = "a".repeat(100);

        // When constructing a namespace from it, then we receive an error
        Namespace::new(name).unwrap_err();
    }

    #[test]
    fn non_ascii_chars_are_rejected() {
        let name = "nameø"; // spell-checker:disable-line
        Namespace::new(name).unwrap_err();
    }

    #[test]
    fn deserialize_config_with_file_registry() {
        let config = NamespaceConfigs::local(&[]);
        let namespace = Namespace::new("local").unwrap();
        assert!(config.contains_key(&namespace));
    }

    #[test]
    fn deserialize_watch_config() {
        let config = NamespaceConfigs::from_toml(
            r#"
            [local]
            directory = "skills"
            "#,
        )
        .unwrap();
        let namespace = Namespace::new("local").unwrap();
        let local_namespace = config.get(&namespace).unwrap();

        let registry = local_namespace.registry();

        assert!(matches!(registry, Registry::File { path } if path == "skills"));
    }

    #[test]
    fn deserialize_config_with_oci_registry() {
        let config = NamespaceConfigs::from_toml(
            r#"
            [pharia-kernel-team]
            config-url = "https://dummy_url"
            registry = "registry.gitlab.aleph-alpha.de"
            base-repository = "engineering/pharia-skills/skills"
            registry-user = "DUMMY_USER"
            registry-password = "DUMMY_PASSWORD"
            "#,
        )
        .unwrap();
        let namespace = Namespace::new("pharia-kernel-team").unwrap();
        let pharia_kernel_team = config.get(&namespace).unwrap();
        assert_eq!(
            pharia_kernel_team.registry(),
            Registry::Oci {
                registry: "registry.gitlab.aleph-alpha.de".to_owned(),
                base_repository: "engineering/pharia-skills/skills".to_owned(),
                user: "DUMMY_USER".to_owned(),
                password: "DUMMY_PASSWORD".to_owned(),
            }
        );
    }

    #[test]
    fn deserialize_config_with_config_access_token() {
        let config = NamespaceConfigs::from_toml(
            r#"
            [dummy-team]
            config-url = "file://dummy_config_url"
            config-access-token = "GITLAB_CONFIG_ACCESS_TOKEN"
            path = "dummy_file_path"
            "#,
        )
        .unwrap();
        let namespace = Namespace::new("dummy-team").unwrap();
        let namespace_cfg = config.get(&namespace).unwrap();
        let expected = NamespaceConfig::TeamOwned {
            config_url: "file://dummy_config_url".to_owned(),
            config_access_token: Some("GITLAB_CONFIG_ACCESS_TOKEN".to_owned()),
            registry: Registry::File {
                path: "dummy_file_path".to_owned(),
            },
        };
        assert_eq!(namespace_cfg, &expected);
    }

    #[test]
    fn prioritizes_oci_registry() {
        // When deserializing a config which contains both, an oci and a file registry
        // for the same namespace
        let config = toml::from_str::<NamespaceConfigs>(
            r#"
            [pharia-kernel-team]
            config-url = "https://dummy_url"
            config-access-token = "GITLAB_CONFIG_ACCESS_TOKEN"
            registry = "registry.gitlab.aleph-alpha.de"
            base-repository = "engineering/pharia-skills/skills"
            registry-user = "PHARIA_KERNEL_TEAM_REGISTRY_USER"
            registry-password = "PHARIA_KERNEL_TEAM_REGISTRY_PASSWORD"
            path = "local-path"
            "#,
        )
        .unwrap();

        let key = Namespace::new("pharia-kernel-team").unwrap();
        let namespace = config.get(&key).unwrap();

        // Then the `Oci` variants is prioritized
        assert!(matches!(namespace.registry(), Registry::Oci { .. }));
    }

    #[test]
    fn deserializes_multiple_namespaces() {
        let config = toml::from_str::<NamespaceConfigs>(
            r#"
            [pharia-kernel-team]
            config-url = "https://dummy_url"
            config-access-token = "GITLAB_CONFIG_ACCESS_TOKEN"
            registry = "registry.gitlab.aleph-alpha.de"
            base-repository = "engineering/pharia-skills/skills"
            registry-user = "PHARIA_KERNEL_TEAM_REGISTRY_USER"
            registry-password = "PHARIA_KERNEL_TEAM_REGISTRY_PASSWORD"

            [pharia-kernel-team-local]
            config-url = "https://dummy_url"
            config-access-token = "GITLAB_CONFIG_ACCESS_TOKEN"
            path = "/temp/skills"
            "#,
        )
        .unwrap();

        let expected = NamespaceConfigs::new(
            [
                (
                    Namespace::new("pharia-kernel-team").unwrap(),
                    NamespaceConfig::TeamOwned {
                        config_url: "https://dummy_url".to_owned(),
                        config_access_token: Some("GITLAB_CONFIG_ACCESS_TOKEN".to_owned()),
                        registry: Registry::Oci {
                            registry: "registry.gitlab.aleph-alpha.de".to_owned(),
                            base_repository: "engineering/pharia-skills/skills".to_owned(),
                            user: "PHARIA_KERNEL_TEAM_REGISTRY_USER".to_owned(),
                            password: "PHARIA_KERNEL_TEAM_REGISTRY_PASSWORD".to_owned(),
                        },
                    },
                ),
                (
                    Namespace::new("pharia-kernel-team-local").unwrap(),
                    NamespaceConfig::TeamOwned {
                        config_url: "https://dummy_url".to_owned(),
                        config_access_token: Some("GITLAB_CONFIG_ACCESS_TOKEN".to_owned()),
                        registry: Registry::File {
                            path: "/temp/skills".to_owned(),
                        },
                    },
                ),
            ]
            .into_iter()
            .collect(),
        );
        assert_eq!(config, expected);
    }
}
