use std::{env, net::SocketAddr};

pub struct AppConfig {
    pub tcp_addr: SocketAddr,
    pub inference_addr: String,
}

impl AppConfig {
    pub fn from_env() -> Self {
        drop(dotenvy::dotenv());

        let addr = env::var("PHARIA_KERNEL_ADDRESS").unwrap_or_else(|_| "0.0.0.0:8081".to_owned());

        let inference_addr = env::var("AA_INFERENCE_ADDRESS")
            .unwrap_or_else(|_| "https://api.aleph-alpha.com".to_owned());

        AppConfig {
            tcp_addr: addr.parse().unwrap(),
            inference_addr,
        }
    }
}
