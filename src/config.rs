use std::{env, net::SocketAddr};

pub struct AppConfig {
    pub tcp_addr: SocketAddr,
}

impl AppConfig {
    pub fn from_env() -> Self {
        drop(dotenvy::dotenv());

        let host = env::var("HOST").expect("HOST variable not set");
        let port = env::var("PORT").expect("PORT variable not set");

        AppConfig {
            tcp_addr: format!("{host}:{port}").parse().unwrap(),
        }
    }
}
