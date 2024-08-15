use std::{collections::HashMap, fs, path::Path};

use serde::Deserialize;

#[derive(Deserialize)]
pub struct Config {
    pub namespaces: HashMap<String, Namespace>,
}

impl Config {
    pub fn from_str(config: &str) -> Self {
        toml::from_str(config).expect("Config is invalid")
    }

    pub fn from_file<P: AsRef<Path>>(p: P) -> Self {
        let config = fs::read_to_string(p).expect("Could not read config file");
        Self::from_str(&config)
    }
}

#[derive(Deserialize)]
pub struct Namespace {
    repository: String,
    registry: String,
    pub config_url: String,
}

#[cfg(test)]
mod tests {
    use super::Config;

    #[test]
    fn deserialize_config() {
        let config = Config::from_str(
            r#"
            [namespaces.pharia-kernel-team]
            config_url = "https://gitlab.aleph-alpha.de/api/v4/projects/966/repository/files/config.toml/raw?ref=main"
            registry = "registry.gitlab.aleph-alpha.de"
            repository = "engineering/pharia-skills/skills"
            "#,
        );
        assert!(config.namespaces.contains_key("pharia-kernel-team"));
    }

    #[test]
    fn reads_from_file() {
        let config = Config::from_file("config.toml");
        assert!(config.namespaces.contains_key("pharia-kernel-team"));
    }
}
