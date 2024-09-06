use std::{env, net::SocketAddr};

use crate::configuration_observer::OperatorConfig;

pub struct AppConfig {
    pub tcp_addr: SocketAddr,
    /// This base URL is used to do inference against models hosted by the Aleph Alpha inference
    /// stack, as well as used to fetch Tokenizers for said models.
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

        assert!(
            !inference_addr.is_empty(),
            "The inference address must be provided."
        );

        let operator_config = OperatorConfig::from_file("operator-config.toml")
            .expect("The provided operator configuration must be valid.");

        AppConfig {
            tcp_addr: addr.parse().unwrap(),
            inference_addr,
            operator_config,
        }
    }
}
