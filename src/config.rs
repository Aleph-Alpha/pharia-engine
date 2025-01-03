use anyhow::anyhow;
use serde::Deserialize;
use std::{env, net::SocketAddr, time::Duration};

use crate::namespace_watcher::OperatorConfig;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct AppConfig {
    #[serde(rename = "pharia-kernel-address")]
    pub tcp_addr: SocketAddr,
    /// Address to expose metrics on
    #[serde(rename = "pharia-kernel-metrics-address")]
    pub metrics_addr: SocketAddr,
    /// This base URL is used to do inference against models hosted by the Aleph Alpha inference
    /// stack, as well as used to fetch Tokenizers for said models.
    #[serde(rename = "aa-inference-address")]
    pub inference_addr: String,
    /// This base URL is used to do search hosted by the Aleph Alpha Document Index.
    #[serde(rename = "document-index-address")]
    pub document_index_addr: String,
    /// This base URL is used to authorize an `PHARIA_AI_TOKEN` for use by the kernel
    #[serde(rename = "authorization-address")]
    pub authorization_addr: String,
    #[serde(flatten)]
    pub operator_config: OperatorConfig,
    #[serde(with = "humantime_serde")]
    pub namespace_update_interval: Duration,
    pub log_level: String,
    pub open_telemetry_endpoint: Option<String>,
    pub use_pooling_allocator: bool,
}

impl AppConfig {
    /// # Panics
    ///
    /// Will panic if the `PHARIA_KERNEL_ADDRESS` environment variable is not parseable as a TCP Address.
    ///
    /// # Errors
    ///
    /// Will return error if operator config exists but is invalid.
    pub fn from_env() -> anyhow::Result<Self> {
        drop(dotenvy::dotenv());

        let addr = env::var("PHARIA_KERNEL_ADDRESS").unwrap_or_else(|_| "0.0.0.0:8081".to_owned());
        let metrics_addr =
            env::var("PHARIA_KERNEL_METRICS_ADDRESS").unwrap_or_else(|_| "0.0.0.0:9000".to_owned());

        let inference_addr = env::var("AA_INFERENCE_ADDRESS")
            .unwrap_or_else(|_| "https://inference-api.product.pharia.com".to_owned());

        let document_index_addr = env::var("DOCUMENT_INDEX_ADDRESS")
            .unwrap_or_else(|_| "https://document-index.product.pharia.com".to_owned());

        let authorization_addr = env::var("AUTHORIZATION_ADDRESS")
            .unwrap_or_else(|_| "https://pharia-iam.product.pharia.com".to_owned());

        let log_level = match env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_owned()) {
            level if ["debug", "trace"].contains(&level.as_str()) => {
                // Don't allow third-party crates to go below info unless they user passed in a
                // RUST_LOG formatted string themselves
                format!("info,pharia_kernel={level}")
            }
            level => level,
        };

        let open_telemetry_endpoint = env::var("OPEN_TELEMETRY_ENDPOINT").ok();

        assert!(
            !inference_addr.is_empty(),
            "The inference address must be available."
        );
        assert!(
            !authorization_addr.is_empty(),
            "The authorization address must be available."
        );

        let namespace_update_interval: humantime::Duration = env::var("NAMESPACE_UPDATE_INTERVAL")
            .as_deref()
            .unwrap_or("10s")
            .parse()?;

        let operator_config = OperatorConfig::new("operator-config.toml")
            .map_err(|err| anyhow!("The provided operator configuration must be valid: {err}"))?;

        let use_pooling_allocator = env::var("USE_POOLING_ALLOCATOR")
            .as_deref()
            .unwrap_or("false")
            .parse()?;

        Ok(AppConfig {
            tcp_addr: addr.parse().unwrap(),
            metrics_addr: metrics_addr.parse().unwrap(),
            inference_addr,
            document_index_addr,
            authorization_addr,
            operator_config,
            namespace_update_interval: namespace_update_interval.into(),
            log_level,
            open_telemetry_endpoint,
            use_pooling_allocator,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, time::Duration};

    use config::{Case, Config, Environment};

    use crate::AppConfig;

    impl AppConfig {
        /// A namespace can contain the characters `[a-z0-9-]` e.g. `pharia-kernel-team`.
        ///
        /// As only `SCREAMING_SNAKE_CASE` is widely supported for environment variable keys,
        /// we support it by converting each key into `kebab-case`.
        /// Because we have a nested configuration, we use double underscores as the separators.
        fn environment() -> Environment {
            Environment::with_convert_case(Case::Kebab).separator("__")
        }
    }

    #[test]
    fn load_app_config_from_one_source() -> anyhow::Result<()> {
        // Given a hashmap with variables
        let env_vars = HashMap::from([
            (
                "PHARIA_KERNEL_ADDRESS".to_owned(),
                "0.0.0.0:8081".to_owned(),
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
            ("LOG_LEVEL".to_owned(), "info".to_owned()),
            (
                "OPEN_TELEMETRY_ENDPOINT".to_owned(),
                "open-telemetry".to_owned(),
            ),
            ("USE_POOLING_ALLOCATOR".to_owned(), "true".to_owned()),
            ("NAMESPACES__DEV__DIRECTORY".to_owned(), "skills".to_owned()),
        ]);

        // When we build the source from the environment variables
        let source = AppConfig::environment().source(Some(env_vars));
        let config = Config::builder()
            .add_source(source)
            .build()?
            .try_deserialize::<AppConfig>()?;

        assert_eq!(config.operator_config.namespaces.len(), 1);
        assert_eq!(config.namespace_update_interval, Duration::from_secs(10));
        Ok(())
    }
}
