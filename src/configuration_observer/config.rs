use std::{collections::HashMap, env, fs, path::Path};

use serde::Deserialize;

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
                registry_type = "file"
                registry = "file://skills"
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
#[serde(rename_all = "snake_case", tag = "registry_type")]
pub enum NamespaceConfig {
    File {
        registry: String,
        config_url: String,
        config_access_token_env_var: Option<String>,
    },
    Oci {
        repository: String,
        registry: String,
        config_url: String,
        config_access_token_env_var: Option<String>,
    },
}

#[cfg(test)]
mod tests {

    use crate::configuration_observer::NamespaceConfig;

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
            registry_type = "oci"
            registry = "registry.gitlab.aleph-alpha.de"
            repository = "engineering/pharia-skills/skills"
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
            registry_type = "file"
            registry = "dummy_file_path"
            config_access_token_env_var = "GITLAB_CONFIG_ACCESS_TOKEN"
            "#,
        )
        .unwrap();
        let config_access_token_env_var = match config.namespaces.get("dummy_team").unwrap() {
            NamespaceConfig::File {
                config_access_token_env_var,
                ..
            }
            | NamespaceConfig::Oci {
                config_access_token_env_var,
                ..
            } => config_access_token_env_var.clone(),
        };
        assert_eq!(
            config_access_token_env_var.unwrap(),
            "GITLAB_CONFIG_ACCESS_TOKEN".to_owned()
        );
    }

    #[test]
    fn reads_from_file() {
        let config = OperatorConfig::from_file("config.toml").unwrap();
        assert!(config.namespaces.contains_key("pharia-kernel-team"));
    }
}
