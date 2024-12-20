use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context};
use serde::Deserialize;
use url::Url;

use crate::skill_loader::RegistryConfig;

use super::{
    namespace_description::{
        FileLoader, HttpLoader, NamespaceDescription, SkillDescription, WatchLoader,
    },
    NamespaceDescriptionLoader,
};

#[derive(Deserialize, PartialEq, Eq, Debug, Clone)]
pub struct OperatorConfig {
    #[serde(default)]
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

    /// Create an operator config which checks the local `skills` directory for
    /// a list of skills that are provided in the `skills` argument.
    /// Compared to the `NamespaceConfig::TeamOwned` variant, this removes one
    /// level of indirection (namespace config), allowing for easier testing.
    #[must_use]
    pub fn local(skills: &[&str]) -> Self {
        OperatorConfig {
            namespaces: [(
                "local".to_owned(),
                NamespaceConfig::InPlace {
                    skills: skills
                        .iter()
                        .map(|&name| SkillDescription {
                            name: name.to_owned(),
                            tag: None,
                        })
                        .collect(),
                    registry: Registry::File {
                        path: "skills".to_owned(),
                    },
                },
            )]
            .into(),
        }
    }

    /// Which namespaces is backed by which registry
    #[must_use]
    pub fn registry_config(&self) -> RegistryConfig {
        RegistryConfig::new(
            self.namespaces
                .iter()
                .map(|(k, v)| (k.to_owned(), v.registry()))
                .collect(),
        )
    }

    #[must_use]
    pub fn dev() -> Self {
        let namespaces = [(
            "dev".to_owned(),
            NamespaceConfig::Watch {
                directory: "skills".into(),
            },
        )]
        .into();
        Self { namespaces }
    }
}

#[derive(Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct RegistryAuth {
    user_env_var: String,
    password_env_var: String,
}

impl RegistryAuth {
    pub fn user(&self) -> String {
        env::var(self.user_env_var.as_str()).unwrap_or_else(|_| {
            panic!(
                "{} must be set if OCI registry is used.",
                self.user_env_var.as_str()
            )
        })
    }

    pub fn password(&self) -> String {
        env::var(self.password_env_var.as_str()).unwrap_or_else(|_| {
            panic!(
                "{} must be set if OCI registry is used.",
                self.password_env_var.as_str()
            )
        })
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
        #[serde(flatten)]
        auth: RegistryAuth,
    },
}

#[derive(Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(untagged)]
pub enum NamespaceConfig {
    /// Namespaces are our way to enable teams to deploy skills in self service via Git Ops. This
    /// implies that the skills in team owned namespaces are configured by a team rather than the
    /// operators of Pharia Kernel, which in turn means we only refer the teams documentation here.
    TeamOwned {
        config_url: String,
        config_access_token_env_var: Option<String>,
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
            NamespaceConfig::Watch { directory } => {
                Ok(Box::new(WatchLoader::new(directory.to_owned())))
            }
            NamespaceConfig::InPlace {
                skills,
                registry: _,
            } => Ok(Box::new(NamespaceDescription {
                skills: skills.clone(),
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use config::{Config, Environment, File, FileFormat};

    use crate::namespace_watcher::config::{Registry, RegistryAuth};

    use crate::namespace_watcher::tests::NamespaceConfig;

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
    fn load_from_two_empty_sources() -> anyhow::Result<()> {
        // Given a TOML file and environment variables
        let file_source = File::from_str("", FileFormat::Toml);
        let env_vars = HashMap::new();
        let env_source = Environment::default()
            .separator("__")
            .source(Some(env_vars));

        // When loading from the sources
        let config = Config::builder()
            .add_source(file_source)
            .add_source(env_source)
            .build()?
            .try_deserialize::<OperatorConfig>()?;

        // Then both sources are applied, with the values from environment variables having precedence
        assert_eq!(config.namespaces.len(), 0);
        Ok(())
    }

    #[test]
    fn load_from_two_partial_sources() -> anyhow::Result<()> {
        // Given a TOML file and environment variables
        let config_url = "https://acme.com/latest/config.toml";
        let config_access_token_env_var = "ACME_CONFIG_ACCESS_TOKEN";
        let registry = "registry.acme.com";
        let repository = "engineering/skills";
        let user_env_var = "SKILL_REGISTRY_USER";
        let password_env_var = "SKILL_REGISTRY_PASSWORD";
        let file_source = File::from_str(
            &format!(
                "[namespaces.acme]
config_url = \"to_be_overwritten\"
config_access_token_env_var = \"{config_access_token_env_var}\"

[namespaces.acme.registry]
type = \"oci\"
registry = \"{registry}\"
repository = \"{repository}\"
password_env_var =  \"{password_env_var}\"
            ",
            ),
            FileFormat::Toml,
        );
        let env_vars = HashMap::from([
            (
                "NAMESPACES__ACME__CONFIG_URL".to_owned(),
                config_url.to_owned(),
            ),
            (
                "NAMESPACES__ACME__REGISTRY__USER_ENV_VAR".to_owned(),
                user_env_var.to_owned(),
            ),
        ]);
        let env_source = Environment::default()
            .separator("__")
            .source(Some(env_vars));

        // When loading from the sources
        let config = Config::builder()
            .add_source(file_source)
            .add_source(env_source)
            .build()?
            .try_deserialize::<OperatorConfig>()?;

        // Then both sources are applied, with the values from environment variables having higher precedence
        assert_eq!(config.namespaces.len(), 1);
        let namespace_config = NamespaceConfig::TeamOwned {
            config_url: config_url.to_owned(),
            config_access_token_env_var: Some(config_access_token_env_var.to_owned()),
            registry: Registry::Oci {
                registry: registry.to_owned(),
                repository: repository.to_owned(),
                auth: RegistryAuth {
                    user_env_var: user_env_var.to_owned(),
                    password_env_var: password_env_var.to_owned(),
                },
            },
        };
        assert_eq!(config.namespaces.get("acme").unwrap(), &namespace_config);
        Ok(())
    }

    #[test]
    fn deserialize_registry_from_env() -> anyhow::Result<()> {
        // Given some environment variable
        let registry = "gitlab.aleph-alpha.de".to_owned();
        let repository = "engineering/pharia-skills/skills".to_owned();
        let user_env_var = "SKILL_REGISTRY_USER".to_owned();
        let password_env_var = "SKILL_REGISTRY_PASSWORD".to_owned();
        let env_vars = HashMap::from([
            ("TYPE".to_owned(), "oci".to_owned()),
            ("REGISTRY".to_owned(), registry.clone()),
            ("REPOSITORY".to_owned(), repository.clone()),
            ("USER_ENV_VAR".to_owned(), user_env_var.clone()),
            ("PASSWORD_ENV_VAR".to_owned(), password_env_var.clone()),
        ]);

        // When we  build the source from the environment variables
        let source = Environment::default().source(Some(env_vars));
        let config = Config::builder()
            .add_source(source)
            .build()?
            .try_deserialize::<Registry>()?;

        // Then we can build the config from the source
        let expected = Registry::Oci {
            registry,
            repository,
            auth: RegistryAuth {
                user_env_var,
                password_env_var,
            },
        };
        assert_eq!(config, expected);
        Ok(())
    }

    #[test]
    fn deserialize_namespace_config_from_env() -> anyhow::Result<()> {
        let registry = "gitlab.aleph-alpha.de".to_owned();
        let repository = "engineering/pharia-skills/skills".to_owned();
        let user_env_var = "SKILL_REGISTRY_USER".to_owned();
        let password_env_var = "SKILL_REGISTRY_PASSWORD".to_owned();
        let config_url = "https://gitlab.aleph-alpha.de/playground".to_owned();
        let config_access_token_env_var = "GITLAB_CONFIG_ACCESS_TOKEN".to_owned();

        let env_vars = HashMap::from([
            ("REGISTRY__TYPE".to_owned(), "oci".to_owned()),
            ("REGISTRY__REGISTRY".to_owned(), registry.clone()),
            ("REGISTRY__REPOSITORY".to_owned(), repository.clone()),
            ("REGISTRY__USER_ENV_VAR".to_owned(), user_env_var.clone()),
            (
                "REGISTRY__PASSWORD_ENV_VAR".to_owned(),
                password_env_var.clone(),
            ),
            ("CONFIG_URL".to_owned(), config_url.clone()),
            (
                "CONFIG_ACCESS_TOKEN_ENV_VAR".to_owned(),
                config_access_token_env_var.clone(),
            ),
        ]);

        let source = Environment::default()
            .separator("__")
            .source(Some(env_vars));
        let config = Config::builder()
            .add_source(source)
            .build()?
            .try_deserialize::<NamespaceConfig>()?;

        let expected = NamespaceConfig::TeamOwned {
            config_url,
            config_access_token_env_var: Some(config_access_token_env_var.clone()),
            registry: Registry::Oci {
                registry,
                repository,
                auth: RegistryAuth {
                    user_env_var,
                    password_env_var,
                },
            },
        };
        assert_eq!(config, expected);
        Ok(())
    }

    #[test]
    fn deserialize_empty_operator_config() -> anyhow::Result<()> {
        // Given a hashmap with variables
        let env_vars = HashMap::from([]);

        // When we build the source from the environment variables
        let source = Environment::default().separator("_").source(Some(env_vars));
        let config = Config::builder()
            .add_source(source)
            .build()?
            .try_deserialize::<OperatorConfig>()?;

        assert_eq!(config, OperatorConfig::empty());
        Ok(())
    }

    #[test]
    fn deserialize_operator_config_with_namespaces() -> anyhow::Result<()> {
        // Given a hashmap with variables
        let env_vars = HashMap::from([
            (
                "NAMESPACES__PLAY_GROUND__CONFIG_URL".to_owned(),
                "https://gitlab.aleph-alpha.de/playground".to_owned(),
            ),
            (
                "NAMESPACES__PLAY_GROUND__CONFIG_ACCESS_TOKEN_ENV_VAR".to_owned(),
                "GITLAB_CONFIG_ACCESS_TOKEN".to_owned(),
            ),
            (
                "NAMESPACES__PLAY_GROUND__REGISTRY__TYPE".to_owned(),
                "oci".to_owned(),
            ),
            (
                "NAMESPACES__PLAY_GROUND__REGISTRY__REGISTRY".to_owned(),
                "registry.gitlab.aleph-alpha.de".to_owned(),
            ),
            (
                "NAMESPACES__PLAY_GROUND__REGISTRY__REPOSITORY".to_owned(),
                "engineering/pharia-skills/skills".to_owned(),
            ),
            (
                "NAMESPACES__PLAY_GROUND__REGISTRY__USER_ENV_VAR".to_owned(),
                "SKILL_REGISTRY_USER".to_owned(),
            ),
            (
                "NAMESPACES__PLAY_GROUND__REGISTRY__PASSWORD_ENV_VAR".to_owned(),
                "SKILL_REGISTRY_PASSWORD".to_owned(),
            ),
        ]);

        // When we build the source from the environment variables
        let source = Environment::default()
            .separator("__")
            .source(Some(env_vars));
        let config = Config::builder()
            .add_source(source)
            .build()?
            .try_deserialize::<OperatorConfig>()?;
        assert_eq!(config.namespaces.len(), 1);
        Ok(())
    }

    #[test]
    fn deserialize_config_with_file_registry() {
        let config = OperatorConfig::local(&[]);
        assert!(config.namespaces.contains_key("local"));
    }

    #[test]
    fn deserialize_watch_config() {
        let config = OperatorConfig::from_toml(
            r#"
            [namespaces.local]
            directory = "skills"
            "#,
        )
        .unwrap();
        let local_namespace = config.namespaces.get("local").unwrap();

        let registry = local_namespace.registry();

        assert!(matches!(registry, Registry::File { path } if path == "skills"));
    }

    #[test]
    fn deserialize_config_with_oci_registry() {
        let config = OperatorConfig::from_toml(
            r#"
            [namespaces.pharia-kernel-team]
            config_url = "https://dummy_url"

            [namespaces.pharia-kernel-team.registry]
            type = "oci"
            registry = "registry.gitlab.aleph-alpha.de"
            repository = "engineering/pharia-skills/skills"
            user_env_var = "SKILL_REGISTRY_USER"
            password_env_var = "SKILL_REGISTRY_PASSWORD"
            "#,
        )
        .unwrap();
        let pharia_kernel_team = config.namespaces.get("pharia-kernel-team").unwrap();
        assert_eq!(
            pharia_kernel_team.registry(),
            Registry::Oci {
                registry: "registry.gitlab.aleph-alpha.de".to_owned(),
                repository: "engineering/pharia-skills/skills".to_owned(),
                auth: RegistryAuth {
                    user_env_var: "SKILL_REGISTRY_USER".to_owned(),
                    password_env_var: "SKILL_REGISTRY_PASSWORD".to_owned(),
                },
            }
        );
    }

    #[test]
    fn deserialize_config_with_config_access_token() {
        let config = OperatorConfig::from_toml(
            r#"
            [namespaces.dummy_team]
            config_url = "file://dummy_config_url"
            config_access_token_env_var = "GITLAB_CONFIG_ACCESS_TOKEN"

            [namespaces.dummy_team.registry]
            type = "file"
            path = "dummy_file_path"
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
    fn deserializes_multiple_namespaces() {
        let config = toml::from_str::<OperatorConfig>(
            r#"
            [namespaces.pharia-kernel-team]
            config_url = "https://dummy_url"
            config_access_token_env_var = "GITLAB_CONFIG_ACCESS_TOKEN"

            [namespaces.pharia-kernel-team.registry]
            type = "oci"
            registry = "registry.gitlab.aleph-alpha.de"
            repository = "engineering/pharia-skills/skills"
            user_env_var = "PHARIA_KERNEL_TEAM_REGISTRY_USER"
            password_env_var = "PHARIA_KERNEL_TEAM_REGISTRY_PASSWORD"

            [namespaces.pharia-kernel-team-local]
            config_url = "https://dummy_url"
            config_access_token_env_var = "GITLAB_CONFIG_ACCESS_TOKEN"

            [namespaces.pharia-kernel-team-local.registry]
            type = "file"
            path = "/temp/skills"
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
                            auth: RegistryAuth {
                                user_env_var: "PHARIA_KERNEL_TEAM_REGISTRY_USER".to_owned(),
                                password_env_var: "PHARIA_KERNEL_TEAM_REGISTRY_PASSWORD".to_owned(),
                            },
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
