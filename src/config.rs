use anyhow::Ok;
use config::{Case, Config, Environment, File, FileFormat, FileSourceFile};
use serde::Deserialize;
use std::{net::SocketAddr, time::Duration};

use crate::namespace_watcher::OperatorConfig;

mod defaults {
    use std::{net::SocketAddr, time::Duration};

    pub fn tcp_addr() -> SocketAddr {
        "0.0.0.0:8081".parse().unwrap()
    }

    pub fn metrics_addr() -> SocketAddr {
        "0.0.0.0:9000".parse().unwrap()
    }

    pub fn inference_addr() -> String {
        "https://inference-api.product.pharia.com".to_owned()
    }

    pub fn document_index_addr() -> String {
        "https://document-index.product.pharia.com".to_owned()
    }

    pub fn authorization_addr() -> String {
        "https://pharia-iam.product.pharia.com".to_owned()
    }

    pub fn namespace_update_interval() -> Duration {
        Duration::from_secs(10)
    }

    pub fn log_level() -> String {
        "info".to_owned()
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct AppConfig {
    #[serde(rename = "pharia-kernel-address", default = "defaults::tcp_addr")]
    pub tcp_addr: SocketAddr,
    /// Address to expose metrics on
    #[serde(
        rename = "pharia-kernel-metrics-address",
        default = "defaults::metrics_addr"
    )]
    pub metrics_addr: SocketAddr,
    /// This base URL is used to do inference against models hosted by the Aleph Alpha inference
    /// stack, as well as used to fetch Tokenizers for said models.
    #[serde(rename = "aa-inference-address", default = "defaults::inference_addr")]
    pub inference_addr: String,
    /// This base URL is used to do search hosted by the Aleph Alpha Document Index.
    #[serde(
        rename = "document-index-address",
        default = "defaults::document_index_addr"
    )]
    pub document_index_addr: String,
    /// This base URL is used to authorize an `PHARIA_AI_TOKEN` for use by the kernel
    #[serde(
        rename = "authorization-address",
        default = "defaults::authorization_addr"
    )]
    pub authorization_addr: String,
    #[serde(flatten)]
    pub operator_config: OperatorConfig,
    #[serde(
        with = "humantime_serde",
        default = "defaults::namespace_update_interval"
    )]
    pub namespace_update_interval: Duration,
    #[serde(default = "defaults::log_level")]
    pub log_level: String,
    pub open_telemetry_endpoint: Option<String>,
    #[serde(default)]
    pub use_pooling_allocator: bool,
}

impl AppConfig {
    /// # Panics
    ///
    /// Will panic if the environment variables `inference_addr` or `authorization_addr` are provided but empty.
    ///
    /// # Errors
    ///
    /// Cannot parse operator config from the provided file or the environment variables.
    pub fn new() -> anyhow::Result<Self> {
        let file = File::with_name("operator-config.toml").required(false);
        let env = Self::environment();
        Self::from_sources(file, env)
    }

    fn from_sources(
        file: File<FileSourceFile, FileFormat>,
        env: Environment,
    ) -> anyhow::Result<Self> {
        let mut config = Config::builder()
            .add_source(file)
            .add_source(env)
            .build()?
            .try_deserialize::<Self>()?;

        assert!(
            !config.inference_addr.is_empty(),
            "The inference address must be available."
        );

        assert!(
            !config.authorization_addr.is_empty(),
            "The authorization address must be available."
        );

        if ["debug", "trace"].contains(&config.log_level.as_str()) {
            // Don't allow third-party crates to go below info unless they user passed in a
            // RUST_LOG formatted string themselves
            config.log_level = format!("info,pharia_kernel={}", config.log_level);
        }

        Ok(config)
    }

    /// A namespace can contain the characters `[a-z0-9-]` e.g. `pharia-kernel-team`.
    ///
    /// As only `SCREAMING_SNAKE_CASE` is widely supported for environment variable keys,
    /// we support it by converting each key into `kebab-case`.
    /// Because we have a nested configuration, we use double underscores as the separators.
    fn environment() -> Environment {
        Environment::with_convert_case(Case::Kebab).separator("__")
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            tcp_addr: defaults::tcp_addr(),
            metrics_addr: defaults::metrics_addr(),
            inference_addr: defaults::inference_addr(),
            document_index_addr: defaults::document_index_addr(),
            authorization_addr: defaults::authorization_addr(),
            operator_config: OperatorConfig::default(),
            namespace_update_interval: defaults::namespace_update_interval(),
            log_level: defaults::log_level(),
            open_telemetry_endpoint: None,
            use_pooling_allocator: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs, time::Duration};

    use config::Config;
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn load_debug_log_level() -> anyhow::Result<()> {
        // Given a hashmap with debug log level
        let dir = tempdir()?;
        let file_path = dir.path().join("operator-config.toml");
        fs::File::create_new(&file_path)?;
        let file_source = File::with_name(file_path.to_str().unwrap());
        let env_vars = HashMap::from([("LOG_LEVEL".to_owned(), "debug".to_owned())]);
        let env_source = AppConfig::environment().source(Some(env_vars));

        // When we build the source from the environment variables
        let config = AppConfig::from_sources(file_source, env_source)?;

        // Then the debug log level is only applied for Pharia Kernel
        assert_eq!(config.log_level, "info,pharia_kernel=debug");
        Ok(())
    }

    #[test]
    fn load_app_config_from_one_source() -> anyhow::Result<()> {
        // Given a hashmap with variables
        let env_vars = HashMap::from([
            (
                "PHARIA_KERNEL_ADDRESS".to_owned(),
                "192.123.1.1:8081".to_owned(),
            ),
            (
                "PHARIA_KERNEL_METRICS_ADDRESS".to_owned(),
                "0.0.0.0:9000".to_owned(),
            ),
            (
                "AA_INFERENCE_ADDRESS".to_owned(),
                "https://inference-api.product.pharia.com".to_owned(),
            ),
            (
                "DOCUMENT_INDEX_ADDRESS".to_owned(),
                "https://document-index.product.pharia.com".to_owned(),
            ),
            (
                "AUTHORIZATION_ADDRESS".to_owned(),
                "https://pharia-iam.product.pharia.com".to_owned(),
            ),
            ("NAMESPACE_UPDATE_INTERVAL".to_owned(), "10s".to_owned()),
            ("LOG_LEVEL".to_owned(), "dummy".to_owned()),
            (
                "OPEN_TELEMETRY_ENDPOINT".to_owned(),
                "open-telemetry".to_owned(),
            ),
            ("USE_POOLING_ALLOCATOR".to_owned(), "true".to_owned()),
            ("NAMESPACES__DEV__DIRECTORY".to_owned(), "skills".to_owned()),
        ]);
        let dir = tempdir()?;
        let file_path = dir.path().join("operator-config.toml");
        fs::File::create_new(&file_path)?;
        let file_source = File::with_name(file_path.to_str().unwrap());
        let env_source = AppConfig::environment().source(Some(env_vars));

        // When we build the source from the environment variables
        let config = AppConfig::from_sources(file_source, env_source)?;

        assert_eq!(config.tcp_addr, "192.123.1.1:8081".parse().unwrap());
        assert_eq!(config.log_level, "dummy");
        assert_eq!(config.operator_config.namespaces.len(), 1);
        assert_eq!(config.namespace_update_interval, Duration::from_secs(10));
        Ok(())
    }

    #[test]
    fn load_default_app_config() -> anyhow::Result<()> {
        // Given a config without any sources
        let config = Config::builder().build()?;

        // When we deserialize it into AppConfig
        let config = config.try_deserialize::<AppConfig>()?;

        // Then the config contains the default values
        assert_eq!(config.tcp_addr, "0.0.0.0:8081".parse().unwrap());
        assert_eq!(config.metrics_addr, "0.0.0.0:9000".parse().unwrap());
        assert_eq!(
            config.inference_addr,
            "https://inference-api.product.pharia.com"
        );
        assert_eq!(
            config.document_index_addr,
            "https://document-index.product.pharia.com"
        );
        assert_eq!(
            config.authorization_addr,
            "https://pharia-iam.product.pharia.com"
        );
        assert_eq!(config.namespace_update_interval, Duration::from_secs(10));
        assert_eq!(config.log_level, "info");
        assert!(config.open_telemetry_endpoint.is_none());
        assert!(!config.use_pooling_allocator);
        assert!(config.operator_config.namespaces.is_empty());
        Ok(())
    }
}
