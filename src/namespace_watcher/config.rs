use std::{collections::HashMap, path::PathBuf};

use anyhow::anyhow;
use config::{Case, Config, Environment, File, FileFormat, FileSourceFile};
use heck::ToKebabCase;
use serde::{Deserialize, Deserializer};
use url::Url;

use crate::skill_loader::RegistryConfig;

use super::{
    namespace_description::{
        FileLoader, HttpLoader, NamespaceDescription, SkillDescription, WatchLoader,
    },
    NamespaceDescriptionLoader,
};

#[derive(PartialEq, Eq, Hash, Debug, Clone)]
pub struct Namespace(String);

impl Namespace {
    pub fn new(input: impl Into<String>) -> Self {
        Self(input.into().to_kebab_case())
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl<'de> Deserialize<'de> for Namespace {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(Namespace::new(s))
    }
}

#[derive(Deserialize, PartialEq, Eq, Debug, Clone)]
pub struct OperatorConfig {
    #[serde(default)]
    pub namespaces: HashMap<Namespace, NamespaceConfig>,
}

impl OperatorConfig {
    /// # Errors
    /// Cannot parse operator config from the provided file or the environment variables.
    pub fn new(config_file: &str) -> anyhow::Result<Self> {
        let file = File::with_name(config_file);
        let env = Self::environment();
        Self::from_sources(file, env)
    }

    /// A namespace can contain the characters `[a-z0-9-]` e.g. `pharia-kernel-team`.
    ///
    /// As only `SCREAMING_SNAKE_CASE` is widely supported for environment variable keys,
    /// we support it by converting each key into `kebab-case`.
    /// Because we have a nested configuration, we use double underscores as the separators.
    fn environment() -> Environment {
        Environment::with_convert_case(Case::Kebab).separator("__")
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
                Namespace::new("local"),
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
                .map(|(k, v)| (k.to_owned().0, v.registry()))
                .collect(),
        )
    }

    #[must_use]
    pub fn dev() -> Self {
        let namespaces = [(
            Namespace::new("dev"),
            NamespaceConfig::Watch {
                directory: "skills".into(),
            },
        )]
        .into();
        Self { namespaces }
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
    /// operators of Pharia Kernel, which in turn means we only refer the teams documentation here.
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
        let env_source = OperatorConfig::environment().source(Some(env_vars));

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
config-url = "a"
config-access-token = "a"
registry = "a"
base-repository = "a"
registry-user =  "a"
registry-password =  "a""#
        )?;
        let file_source = File::with_name(file_path.to_str().unwrap());
        let env_vars = HashMap::from([
            ("NAMESPACES__B__CONFIG_URL".to_owned(), "b".to_owned()),
            (
                "NAMESPACES__B__CONFIG_ACCESS_TOKEN".to_owned(),
                "b".to_owned(),
            ),
            ("NAMESPACES__B__REGISTRY".to_owned(), "b".to_owned()),
            ("NAMESPACES__B__BASE_REPOSITORY".to_owned(), "b".to_owned()),
            ("NAMESPACES__B__REGISTRY_USER".to_owned(), "b".to_owned()),
            (
                "NAMESPACES__B__REGISTRY_PASSWORD".to_owned(),
                "b".to_owned(),
            ),
        ]);
        let env_source = OperatorConfig::environment().source(Some(env_vars));

        // When loading from the sources
        let config = OperatorConfig::from_sources(file_source, env_source)?;

        // Then both namespaces are loaded
        assert_eq!(config.namespaces.len(), 2);
        let namespace_a = Namespace::new("a");
        assert!(config.namespaces.contains_key(&namespace_a));
        let namespace_b = Namespace::new("b");
        assert!(config.namespaces.contains_key(&namespace_b));
        Ok(())
    }

    #[test]
    fn load_one_namespace_from_two_partial_sources() -> anyhow::Result<()> {
        // Given a TOML file and environment variables
        let config_url = "https://acme.com/latest/config.toml";
        let config_access_token = "ACME_CONFIG_ACCESS_TOKEN";
        let registry = "registry.acme.com";
        let base_repository = "engineering/skills";
        let user = "DUMMY_USER";
        let password = "DUMMY_PASSWORD";
        let dir = tempdir()?;
        let file_path = dir.path().join("operator-config.toml");
        let mut file = fs::File::create_new(&file_path)?;
        writeln!(
            file,
            "[namespaces.acme]
config-access-token = \"{config_access_token}\"
registry = \"{registry}\"
base-repository = \"{base_repository}\"
registry-password =  \"{password}\"
        "
        )?;
        let file_source = File::with_name(file_path.to_str().unwrap());
        let env_vars = HashMap::from([
            (
                "NAMESPACES__ACME__CONFIG_URL".to_owned(),
                config_url.to_owned(),
            ),
            (
                "NAMESPACES__ACME__REGISTRY_USER".to_owned(),
                user.to_owned(),
            ),
        ]);
        let env_source = OperatorConfig::environment().source(Some(env_vars));

        // When loading from the sources
        let config = OperatorConfig::from_sources(file_source, env_source)?;

        // Then both sources are applied, with the values from environment variables having higher precedence
        assert_eq!(config.namespaces.len(), 1);
        let namespace_config = NamespaceConfig::TeamOwned {
            config_url: config_url.to_owned(),
            config_access_token: Some(config_access_token.to_owned()),
            registry: Registry::Oci {
                registry: registry.to_owned(),
                base_repository: base_repository.to_owned(),
                user: user.to_owned(),
                password: password.to_owned(),
            },
        };
        let namespace = Namespace::new("acme");
        assert_eq!(
            config.namespaces.get(&namespace).unwrap(),
            &namespace_config
        );
        Ok(())
    }

    #[test]
    fn deserialize_empty_operator_config() -> anyhow::Result<()> {
        // Given a hashmap with variables
        let env_vars = HashMap::from([]);

        // When we build the source from the environment variables
        let source = OperatorConfig::environment().source(Some(env_vars));
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
                "NAMESPACES__PLAY_GROUND__REGISTRY".to_owned(),
                "registry.gitlab.aleph-alpha.de".to_owned(),
            ),
            (
                "NAMESPACES__PLAY_GROUND__BASE_REPOSITORY".to_owned(),
                "engineering/pharia-skills/skills".to_owned(),
            ),
            (
                "NAMESPACES__PLAY_GROUND__REGISTRY_USER".to_owned(),
                "SKILL_REGISTRY_USER".to_owned(),
            ),
            (
                "NAMESPACES__PLAY_GROUND__REGISTRY_PASSWORD".to_owned(),
                "SKILL_REGISTRY_PASSWORD".to_owned(),
            ),
        ]);

        // When we build the source from the environment variables
        let source = OperatorConfig::environment().source(Some(env_vars));
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
        let namespace = Namespace::new("local");
        assert!(config.namespaces.contains_key(&namespace));
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
        let namespace = Namespace::new("local");
        let local_namespace = config.namespaces.get(&namespace).unwrap();

        let registry = local_namespace.registry();

        assert!(matches!(registry, Registry::File { path } if path == "skills"));
    }

    #[test]
    fn deserialize_config_with_oci_registry() {
        let config = OperatorConfig::from_toml(
            r#"
            [namespaces.pharia-kernel-team]
            config-url = "https://dummy_url"
            registry = "registry.gitlab.aleph-alpha.de"
            base-repository = "engineering/pharia-skills/skills"
            registry-user = "DUMMY_USER"
            registry-password = "DUMMY_PASSWORD"
            "#,
        )
        .unwrap();
        let namespace = Namespace::new("pharia-kernel-team");
        let pharia_kernel_team = config.namespaces.get(&namespace).unwrap();
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
        let config = OperatorConfig::from_toml(
            r#"
            [namespaces.dummy-team]
            config-url = "file://dummy_config_url"
            config-access-token = "GITLAB_CONFIG_ACCESS_TOKEN"
            path = "dummy_file_path"
            "#,
        )
        .unwrap();
        let namespace = Namespace::new("dummy-team");
        let namespace_cfg = config.namespaces.get(&namespace).unwrap();
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
        let namespace = Namespace::new("pharia-kernel-team");
        assert!(config.namespaces.contains_key(&namespace));
    }

    #[test]
    fn prioritizes_oci_registry() {
        // When deserializing a config which contains both, an oci and a file registry
        // for the same namespace
        let config = toml::from_str::<OperatorConfig>(
            r#"
            [namespaces.pharia-kernel-team]
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

        let key = Namespace::new("pharia-kernel-team");
        let namespace = config.namespaces.get(&key).unwrap();

        // Then the `Oci` variants is prioritized
        assert!(matches!(namespace.registry(), Registry::Oci { .. }));
    }

    #[test]
    fn deserializes_multiple_namespaces() {
        let config = toml::from_str::<OperatorConfig>(
            r#"
            [namespaces.pharia-kernel-team]
            config-url = "https://dummy_url"
            config-access-token = "GITLAB_CONFIG_ACCESS_TOKEN"
            registry = "registry.gitlab.aleph-alpha.de"
            base-repository = "engineering/pharia-skills/skills"
            registry-user = "PHARIA_KERNEL_TEAM_REGISTRY_USER"
            registry-password = "PHARIA_KERNEL_TEAM_REGISTRY_PASSWORD"

            [namespaces.pharia-kernel-team-local]
            config-url = "https://dummy_url"
            config-access-token = "GITLAB_CONFIG_ACCESS_TOKEN"
            path = "/temp/skills"
            "#,
        )
        .unwrap();

        let expected = OperatorConfig {
            namespaces: [
                (
                    Namespace::new("pharia-kernel-team"),
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
                    Namespace::new("pharia-kernel-team-local"),
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
}
