use std::{env, net::SocketAddr};

pub struct AppConfig {
    pub tcp_addr: SocketAddr,
    pub inference_addr: String,
}

impl AppConfig {
    pub fn from_env() -> Self {
        drop(dotenvy::dotenv());

        let host = env::var("HOST").expect("HOST variable not set");
        let port = env::var("PORT").expect("PORT variable not set");

        let inference_addr = env::var("AA_INFERENCE_ADDRESS")
            .unwrap_or_else(|_| "https://api.aleph-alpha.com".to_owned());

        AppConfig {
            tcp_addr: format!("{host}:{port}").parse().unwrap(),
            inference_addr,
        }
    }
}
