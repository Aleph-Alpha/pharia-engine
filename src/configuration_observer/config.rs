use std::{collections::HashMap, env, fs, path::Path};

use serde::Deserialize;

#[derive(Deserialize)]
pub struct OperatorConfig {
    pub namespaces: HashMap<String, NamespaceConfig>,
}

impl OperatorConfig {
    pub fn from_env_or_default() -> anyhow::Result<Self> {
        drop(dotenvy::dotenv());
        let config_path = env::var("OPERATOR_CONFIG_PATH").unwrap_or("config.toml".to_owned());
        Self::from_file(config_path)
    }

    pub fn from_file<P: AsRef<Path>>(p: P) -> anyhow::Result<Self> {
        let config = fs::read_to_string(p)?;
        Self::from_str(&config)
    }

    pub fn from_str(config: &str) -> anyhow::Result<Self> {
        Ok(toml::from_str(config)?)
    }
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "snake_case", tag = "registry_type")]
pub enum NamespaceConfig {
    File {
        registry: String,
        config_url: String,
    },
    Oci {
        repository: String,
        registry: String,
        config_url: String,
    },
}

#[cfg(test)]
mod tests {

    use super::OperatorConfig;

    impl OperatorConfig {
        pub fn empty() -> Self {
            Self::from_str("[namespaces]").unwrap()
        }

        pub fn local() -> Self {
            Self::from_str(
                r#"
                    [namespaces.local]
                    config_url = "file://skill_config.toml"
                    registry_type = "file"
                    registry = "file://skills"
                "#,
            )
            .unwrap()
        }
    }

    #[test]
    fn deserialize_config_with_file_registry() {
        let config = OperatorConfig::from_str(
            r#"
            [namespaces.local]
            config_url = "file://skill_config.toml"
            registry_type = "file"
            registry = "file://skills"
            "#,
        )
        .unwrap();
        assert!(config.namespaces.contains_key("local"));
    }

    #[test]
    fn deserialize_config_with_oci_registry() {
        let config = OperatorConfig::from_str(
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
    fn reads_from_file() {
        let config = OperatorConfig::from_file("config.toml").unwrap();
        assert!(config.namespaces.contains_key("pharia-kernel-team"));
    }
}
