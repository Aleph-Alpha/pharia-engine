use anyhow::anyhow;
use std::{env, io, net::SocketAddr, time::Duration};

use crate::namespace_watcher::OperatorConfig;

#[derive(Clone)]
pub struct AppConfig {
    pub tcp_addr: SocketAddr,
    /// Address to expose metrics on
    pub metrics_addr: SocketAddr,
    /// This base URL is used to do inference against models hosted by the Aleph Alpha inference
    /// stack, as well as used to fetch Tokenizers for said models.
    pub inference_addr: String,
    /// This base URL is used to do search hosted by the Aleph Alpha Document Index.
    pub document_index_addr: String,
    pub operator_config: OperatorConfig,
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
            .unwrap_or_else(|_| "https://api.aleph-alpha.com".to_owned());

        let document_index_addr = env::var("DOCUMENT_INDEX_ADDRESS")
            .unwrap_or_else(|_| "https://document-index.aleph-alpha.com".to_owned());

        let log_level = env::var("LOG_LEVEL").unwrap_or_else(|_| "error".to_owned());

        let open_telemetry_endpoint = env::var("OPEN_TELEMETRY_ENDPOINT").ok();

        assert!(
            !inference_addr.is_empty(),
            "The inference address must be provided."
        );

        let namespace_update_interval: humantime::Duration = env::var("NAMESPACE_UPDATE_INTERVAL")
            .as_deref()
            .unwrap_or("10s")
            .parse()?;

        let operator_config = match OperatorConfig::from_file("operator-config.toml") {
            Ok(operator_config) => operator_config,
            Err(err) => match err.downcast::<io::Error>() {
                Ok(ioerror) if ioerror.kind() == io::ErrorKind::NotFound => {
                    // println! as the logger is not yet instantiated
                    println!("Info: The 'operator-config.toml' is not found, fallback to the namespace 'dev' with the path 'skills' as the registry.");
                    OperatorConfig::dev()
                }
                Ok(err) => {
                    return Err(anyhow!(
                        "The provided operator configuration must be valid: {err}"
                    ))
                }
                Err(err) => {
                    return Err(anyhow!(
                        "The provided operator configuration must be valid: {err}"
                    ))
                }
            },
        };

        let use_pooling_allocator = env::var("USE_POOLING_ALLOCATOR")
            .as_deref()
            .unwrap_or("false")
            .parse()?;

        Ok(AppConfig {
            tcp_addr: addr.parse().unwrap(),
            metrics_addr: metrics_addr.parse().unwrap(),
            inference_addr,
            document_index_addr,
            operator_config,
            namespace_update_interval: namespace_update_interval.into(),
            log_level,
            open_telemetry_endpoint,
            use_pooling_allocator,
        })
    }
}
