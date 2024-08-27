use std::{collections::HashMap, env, fs, path::Path};

use anyhow::anyhow;
use serde::Deserialize;
use url::Url;

use super::{
    namespace_description::{FileLoader, HttpLoader},
    NamespaceDescriptionLoader,
};

#[derive(Deserialize)]
pub struct OperatorConfig {
    pub namespaces: HashMap<String, NamespaceConfig>,
}

impl OperatorConfig {
    /// # Errors
    /// Cannot parse config or cannot read from file.
    pub fn from_env_or_default() -> anyhow::Result<Self> {
        drop(dotenvy::dotenv());
        let config_path = env::var("OPERATOR_CONFIG_PATH").unwrap_or("config.toml".to_owned());
        Self::from_file(config_path)
    }

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

    /// # Panics
    /// Cannot parse config.
    #[must_use]
    pub fn remote() -> Self {
        Self::from_toml(
            r#"
                [namespaces.pharia-kernel-team]
                config_url = "https://gitlab.aleph-alpha.de/api/v4/projects/966/repository/files/config.toml/raw?ref=main"
                registry_type = "oci"
                registry = "registry.gitlab.aleph-alpha.de"
                repository = "engineering/pharia-skills/skills"
            "#,
        )
        .unwrap()
    }
}
#[derive(Deserialize, Clone)]
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

#[derive(Deserialize, Clone)]
pub struct NamespaceConfig {
    pub config_url: String,
    pub config_access_token_env_var: Option<String>,
    pub registry: Registry,
}

impl NamespaceConfig {
    pub fn loader(&self) -> anyhow::Result<Box<dyn NamespaceDescriptionLoader + Send + 'static>> {
        let url = Url::parse(&self.config_url)?;
        match url.scheme() {
            "https" | "http" => {
                let config_access_token = self
                    .config_access_token_env_var
                    .as_ref()
                    .map(env::var)
                    .transpose()?;
                Ok(Box::new(HttpLoader::from_url(
                    &self.config_url,
                    config_access_token,
                )))
            }
            "file" => {
                // remove leading "file://"
                let file_path = &self.config_url[7..];
                let loader = FileLoader::new(file_path.into());
                Ok(Box::new(loader))
            }
            scheme => Err(anyhow!("Unsupported URL scheme: {scheme}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::configuration_observer::config::Registry;

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
            config_url = "https://gitlab.aleph-alpha.de/api/v4/projects/966/repository/files/config.toml/raw?ref=main"
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
        let config_access_token_env_var = config
            .namespaces
            .get("dummy_team")
            .unwrap()
            .config_access_token_env_var
            .as_ref()
            .unwrap();
        assert_eq!(config_access_token_env_var, "GITLAB_CONFIG_ACCESS_TOKEN");
    }

    #[test]
    fn reads_from_file() {
        let config = OperatorConfig::from_file("config.toml").unwrap();
        assert!(config.namespaces.contains_key("pharia-kernel-team"));
    }

    #[test]
    fn deserializes_new_config() {
        let config = toml::from_str::<OperatorConfig>(
            r#"
            [namespaces.pharia-kernel-team]
            config_url = "https://gitlab.aleph-alpha.de/api/v4/projects/966/repository/files/config.toml/raw?ref=main"
            config_access_token_env_var = "GITLAB_CONFIG_ACCESS_TOKEN"
            registry = { type = "oci", registry = "registry.gitlab.aleph-alpha.de", repository = "engineering/pharia-skills/skills" }

            [namespaces.pharia-kernel-team-local]
            config_url = "https://gitlab.aleph-alpha.de/api/v4/projects/966/repository/files/config.toml/raw?ref=main"
            config_access_token_env_var = "GITLAB_CONFIG_ACCESS_TOKEN"
            registry = { type = "file", path = "/temp/skills" }
            "#,
        )
        .unwrap();

        assert_eq!(config.namespaces.len(), 2);
        assert!(matches!(
            &config.namespaces["pharia-kernel-team"].registry,
            Registry::Oci { .. }
        ));
        assert!(matches!(
            &config.namespaces["pharia-kernel-team-local"].registry,
            Registry::File { .. }
        ));
    }
}
