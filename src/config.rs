use std::{env, net::SocketAddr};

use tracing::info;

use crate::configuration_observer::OperatorConfig;

pub struct AppConfig {
    pub tcp_addr: SocketAddr,
    pub inference_addr: String,
    pub operator_config: OperatorConfig,
}

impl AppConfig {
    /// # Panics
    ///
    /// Will panic if the `PHARIA_KERNEL_ADDRESS` environment variable is not parseable as a TCP Address.
    #[must_use]
    pub fn from_env() -> Self {
        drop(dotenvy::dotenv());

        let addr = env::var("PHARIA_KERNEL_ADDRESS").unwrap_or_else(|_| "0.0.0.0:8081".to_owned());

        let inference_addr = env::var("AA_INFERENCE_ADDRESS")
            .unwrap_or_else(|_| "https://api.aleph-alpha.com".to_owned());

        let operator_config = if let Ok(operator_config_path) = env::var("OPERATOR_CONFIG_PATH") {
            OperatorConfig::from_file(&operator_config_path)
                .expect("The provided operator configuration must be valid.")
        } else {
            info!("No operator config provided. Fallback to the default remote config.");
            OperatorConfig::remote()
        };

        AppConfig {
            tcp_addr: addr.parse().unwrap(),
            inference_addr,
            operator_config,
        }
    }
}
