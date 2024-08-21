use std::{env, net::SocketAddr, path::PathBuf};

pub struct AppConfig {
    pub tcp_addr: SocketAddr,
    pub inference_addr: String,
    pub operator_config: PathBuf,
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

        let operator_config =
            env::var("OPERATOR_CONFIG_PATH").unwrap_or_else(|_| "config.toml".to_owned());
        let operator_config = operator_config
            .parse()
            .unwrap_or_else(|_| panic!("Invalid path {operator_config} for operator config"));

        AppConfig {
            tcp_addr: addr.parse().unwrap(),
            inference_addr,
            operator_config,
        }
    }
}
