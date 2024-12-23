use std::{collections::HashMap, path::PathBuf};

use anyhow::anyhow;
use config::{Config, Environment, File, FileFormat, FileSourceFile};
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
    /// Cannot parse operator config from the provided file or the environment variables.
    pub fn new(config_file: &str) -> anyhow::Result<Self> {
        let file = File::with_name(config_file);
        let env = Environment::default().separator("__");
        Self::from_sources(file, env)
    }

    fn from_sources(
        file: File<FileSourceFile, FileFormat>,
        env: Environment,
    ) -> anyhow::Result<Self> {
        let config = Config::builder()
            .add_source(file)
            .add_source(env)
            .build()?
            .try_deserialize::<OperatorConfig>()?;
        Ok(config)
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
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Registry {
    File {
        path: String,
    },
    Oci {
        #[serde(rename = "name")]
        registry: String,
        repository: String,
        user: String,
        password: String,
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
        config_access_token: Option<String>,
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
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;

    use heck::ToKebabCase;
    use serde::Deserializer;
    use serde_json::json;
    use tempfile::tempdir;

    use super::*;

    impl OperatorConfig {
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
    fn load_from_two_empty_sources() -> anyhow::Result<()> {
        // Given a TOML file and environment variables
        let dir = tempdir()?;
        let file_path = dir.path().join("operator-config.toml");
        fs::File::create_new(&file_path)?;
        let file_source = File::with_name(file_path.to_str().unwrap());
        let env_vars = HashMap::new();
        let env_source = Environment::default()
            .separator("__")
            .source(Some(env_vars));

        // When loading from the sources
        let config = OperatorConfig::from_sources(file_source, env_source)?;

        // Then both sources are applied, with the values from environment variables having precedence
        assert_eq!(config.namespaces.len(), 0);
        Ok(())
    }

    #[test]
    fn load_two_namespaces_from_independent_sources() -> anyhow::Result<()> {
        // Given a TOML file and environment variables
        let dir = tempdir()?;
        let file_path = dir.path().join("operator-config.toml");
        let mut file = fs::File::create_new(&file_path)?;
        writeln!(
            file,
            r#"[namespaces.a]
config_url = "a"
config_access_token = "a"

[namespaces.a.registry]
type = "oci"
name = "a"
repository = "a"
user =  "a"
password =  "a""#
        )?;
        let file_source = File::with_name(file_path.to_str().unwrap());
        let env_vars = HashMap::from([
            ("NAMESPACES__B__CONFIG_URL".to_owned(), "b".to_owned()),
            (
                "NAMESPACES__B__CONFIG_ACCESS_TOKEN".to_owned(),
                "b".to_owned(),
            ),
            ("NAMESPACES__B__REGISTRY__TYPE".to_owned(), "oci".to_owned()),
            ("NAMESPACES__B__REGISTRY__NAME".to_owned(), "b".to_owned()),
            (
                "NAMESPACES__B__REGISTRY__REPOSITORY".to_owned(),
                "b".to_owned(),
            ),
            ("NAMESPACES__B__REGISTRY__USER".to_owned(), "b".to_owned()),
            (
                "NAMESPACES__B__REGISTRY__PASSWORD".to_owned(),
                "b".to_owned(),
            ),
        ]);
        let env_source = Environment::default()
            .separator("__")
            .source(Some(env_vars));

        // When loading from the sources
        let config = OperatorConfig::from_sources(file_source, env_source)?;

        // Then both namespaces are loaded
        assert_eq!(config.namespaces.len(), 2);
        assert!(config.namespaces.contains_key("a"));
        assert!(config.namespaces.contains_key("b"));
        Ok(())
    }

    #[test]
    fn load_one_namespace_from_two_partial_sources() -> anyhow::Result<()> {
        // Given a TOML file and environment variables
        let config_url = "https://acme.com/latest/config.toml";
        let config_access_token = "ACME_CONFIG_ACCESS_TOKEN";
        let registry = "registry.acme.com";
        let repository = "engineering/skills";
        let user = "DUMMY_USER";
        let password = "DUMMY_PASSWORD";
        let dir = tempdir()?;
        let file_path = dir.path().join("operator-config.toml");
        let mut file = fs::File::create_new(&file_path)?;
        writeln!(
            file,
            "[namespaces.acme]
config_url = \"to_be_overwritten\"
config_access_token = \"{config_access_token}\"

[namespaces.acme.registry]
type = \"oci\"
name = \"{registry}\"
repository = \"{repository}\"
password =  \"{password}\"
        "
        )?;
        let file_source = File::with_name(file_path.to_str().unwrap());
        let env_vars = HashMap::from([
            (
                "NAMESPACES__ACME__CONFIG_URL".to_owned(),
                config_url.to_owned(),
            ),
            (
                "NAMESPACES__ACME__REGISTRY__USER".to_owned(),
                user.to_owned(),
            ),
        ]);
        let env_source = Environment::default()
            .separator("__")
            .source(Some(env_vars));

        // When loading from the sources
        let config = OperatorConfig::from_sources(file_source, env_source)?;

        // Then both sources are applied, with the values from environment variables having higher precedence
        assert_eq!(config.namespaces.len(), 1);
        let namespace_config = NamespaceConfig::TeamOwned {
            config_url: config_url.to_owned(),
            config_access_token: Some(config_access_token.to_owned()),
            registry: Registry::Oci {
                registry: registry.to_owned(),
                repository: repository.to_owned(),
                user: user.to_owned(),
                password: password.to_owned(),
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
        let user = "DUMMY_USER".to_owned();
        let password = "DUMMY_PASSWORD".to_owned();
        let env_vars = HashMap::from([
            ("TYPE".to_owned(), "oci".to_owned()),
            ("NAME".to_owned(), registry.clone()),
            ("REPOSITORY".to_owned(), repository.clone()),
            ("USER".to_owned(), user.clone()),
            ("PASSWORD".to_owned(), password.clone()),
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
            user,
            password,
        };
        assert_eq!(config, expected);
        Ok(())
    }

    #[test]
    fn deserialize_namespace_config_from_env() -> anyhow::Result<()> {
        let registry = "gitlab.aleph-alpha.de".to_owned();
        let repository = "engineering/pharia-skills/skills".to_owned();
        let user = "DUMMY_USER".to_owned();
        let password = "DUMMY_PASSWORD".to_owned();
        let config_url = "https://gitlab.aleph-alpha.de/playground".to_owned();
        let config_access_token = "GITLAB_CONFIG_ACCESS_TOKEN".to_owned();

        let env_vars = HashMap::from([
            ("REGISTRY__TYPE".to_owned(), "oci".to_owned()),
            ("REGISTRY__NAME".to_owned(), registry.clone()),
            ("REGISTRY__REPOSITORY".to_owned(), repository.clone()),
            ("REGISTRY__USER".to_owned(), user.clone()),
            ("REGISTRY__PASSWORD".to_owned(), password.clone()),
            ("CONFIG_URL".to_owned(), config_url.clone()),
            (
                "CONFIG_ACCESS_TOKEN".to_owned(),
                config_access_token.clone(),
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
            config_access_token: Some(config_access_token.clone()),
            registry: Registry::Oci {
                registry,
                repository,
                user,
                password,
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
                "NAMESPACES__PLAY_GROUND__CONFIG_ACCESS_TOKEN".to_owned(),
                "GITLAB_CONFIG_ACCESS_TOKEN".to_owned(),
            ),
            (
                "NAMESPACES__PLAY_GROUND__REGISTRY__TYPE".to_owned(),
                "oci".to_owned(),
            ),
            (
                "NAMESPACES__PLAY_GROUND__REGISTRY__NAME".to_owned(),
                "registry.gitlab.aleph-alpha.de".to_owned(),
            ),
            (
                "NAMESPACES__PLAY_GROUND__REGISTRY__REPOSITORY".to_owned(),
                "engineering/pharia-skills/skills".to_owned(),
            ),
            (
                "NAMESPACES__PLAY_GROUND__REGISTRY__USER".to_owned(),
                "SKILL_REGISTRY_USER".to_owned(),
            ),
            (
                "NAMESPACES__PLAY_GROUND__REGISTRY__PASSWORD".to_owned(),
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
            [namespaces.pharia_kernel_team]
            config_url = "https://dummy_url"

            [namespaces.pharia_kernel_team.registry]
            type = "oci"
            name = "registry.gitlab.aleph-alpha.de"
            repository = "engineering/pharia-skills/skills"
            user = "DUMMY_USER"
            password = "DUMMY_PASSWORD"
            "#,
        )
        .unwrap();
        let pharia_kernel_team = config.namespaces.get("pharia_kernel_team").unwrap();
        assert_eq!(
            pharia_kernel_team.registry(),
            Registry::Oci {
                registry: "registry.gitlab.aleph-alpha.de".to_owned(),
                repository: "engineering/pharia-skills/skills".to_owned(),
                user: "DUMMY_USER".to_owned(),
                password: "DUMMY_PASSWORD".to_owned(),
            }
        );
    }

    #[test]
    fn deserialize_config_with_config_access_token() {
        let config = OperatorConfig::from_toml(
            r#"
            [namespaces.dummy-team]
            config_url = "file://dummy_config_url"
            config_access_token = "GITLAB_CONFIG_ACCESS_TOKEN"

            [namespaces.dummy-team.registry]
            type = "file"
            path = "dummy_file_path"
            "#,
        )
        .unwrap();
        let namespace_cfg = config.namespaces.get("dummy-team").unwrap();
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
    fn reads_from_file() {
        drop(dotenvy::dotenv());
        let config = OperatorConfig::new("operator-config.toml").unwrap();
        assert!(config.namespaces.contains_key("pharia_kernel_team"));
    }

    #[test]
    fn deserializes_multiple_namespaces() {
        let config = toml::from_str::<OperatorConfig>(
            r#"
            [namespaces.pharia_kernel_team]
            config_url = "https://dummy_url"
            config_access_token = "GITLAB_CONFIG_ACCESS_TOKEN"

            [namespaces.pharia_kernel_team.registry]
            type = "oci"
            name = "registry.gitlab.aleph-alpha.de"
            repository = "engineering/pharia-skills/skills"
            user = "PHARIA_KERNEL_TEAM_REGISTRY_USER"
            password = "PHARIA_KERNEL_TEAM_REGISTRY_PASSWORD"

            [namespaces.pharia_kernel_team-local]
            config_url = "https://dummy_url"
            config_access_token = "GITLAB_CONFIG_ACCESS_TOKEN"

            [namespaces.pharia_kernel_team-local.registry]
            type = "file"
            path = "/temp/skills"
            "#,
        )
        .unwrap();

        let expected = OperatorConfig {
            namespaces: [
                (
                    "pharia_kernel_team".to_owned(),
                    NamespaceConfig::TeamOwned {
                        config_url: "https://dummy_url".to_owned(),
                        config_access_token: Some("GITLAB_CONFIG_ACCESS_TOKEN".to_owned()),
                        registry: Registry::Oci {
                            registry: "registry.gitlab.aleph-alpha.de".to_owned(),
                            repository: "engineering/pharia-skills/skills".to_owned(),
                            user: "PHARIA_KERNEL_TEAM_REGISTRY_USER".to_owned(),
                            password: "PHARIA_KERNEL_TEAM_REGISTRY_PASSWORD".to_owned(),
                        },
                    },
                ),
                (
                    "pharia_kernel_team-local".to_owned(),
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
        };

        assert_eq!(config, expected);
    }

    fn keys_to_kebab_case<'de, D>(deserializer: D) -> Result<HashMap<String, String>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let map: HashMap<String, String> = Deserialize::deserialize(deserializer)?;
        let map = map
            .into_iter()
            .map(|(k, v)| (k.to_kebab_case(), v))
            .collect();
        Ok(map)
    }

    #[test]
    fn custom_deserialization() -> anyhow::Result<()> {
        // given
        #[derive(Deserialize)]
        struct MyStruct {
            #[serde(deserialize_with = "keys_to_kebab_case")]
            my_map: HashMap<String, String>,
        }
        let json = json!({"my_map": {"my_key":"my_value"}});

        // when
        let my_struct = serde_json::from_value::<MyStruct>(json)?;

        // then
        assert!(my_struct.my_map.contains_key("my-key"));
        Ok(())
    }
}
