use std::{env, net::SocketAddr};

use crate::{configuration_observer::OperatorConfig, skills::SkillExecutorConfig};

pub struct AppConfig {
    pub tcp_addr: SocketAddr,
    /// This base URL is used to do inference against models hosted by the Aleph Alpha inference
    /// stack, as well as used to fetch Tokenizers for said models.
    pub inference_addr: String,
    pub operator_config: OperatorConfig,
    /// This token is used to fetch tokenizers from AA inference api.
    pub aa_api_token: String,
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

        let aa_api_token = env::var("AA_API_TOKEN").expect("AA_API_TOKEN variable not set");

        AppConfig {
            tcp_addr: addr.parse().unwrap(),
            inference_addr,
            operator_config,
            aa_api_token,
        }
    }

    #[must_use]
    pub fn skill_executer_cfg(&self) -> SkillExecutorConfig<'_> {
        SkillExecutorConfig {
            namespaces: &self.operator_config.namespaces,
            api_base_url: self.inference_addr.clone(),
            api_token: self.aa_api_token.clone(),
        }
    }
}
