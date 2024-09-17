mod config;
mod configuration_observer;
mod csi;
mod inference;
mod language_selection;
mod logging;
mod registries;
mod shell;
mod skill_provider;
mod skills;
mod tokenizers;

use anyhow::{Context, Error};
use configuration_observer::{ConfigurationObserver, NamespaceDescriptionLoaders};
use csi::CsiDrivers;
use futures::Future;
use logging::initialize_tracing;
use skill_provider::SkillProvider;
use tokenizers::Tokenizers;
use tracing::error;

use self::{inference::Inference, skills::SkillExecutor};

pub use config::AppConfig;
pub use configuration_observer::OperatorConfig;

/// Boots up all the actors making up the kernel. The result of this method is also a future, which
/// signals that all resources have been shutdown.
///
/// # Errors
///
/// Errors if the configuration is invalid
pub async fn run(
    app_config: AppConfig,
    shutdown_signal: impl Future<Output = ()> + Send + 'static,
) -> Result<impl Future<Output = ()>, Error> {
    initialize_tracing(&app_config)?;

    // Boot up the drivers which power the CSI. Right now we only have inference.
    let inference = Inference::new(app_config.inference_addr.clone());
    let tokenizers = Tokenizers::new(app_config.inference_addr.clone()).unwrap();
    let csi_drivers = CsiDrivers {
        inference: inference.api(),
        tokenizers: tokenizers.api(),
    };
    let skill_provider = SkillProvider::new(&app_config.operator_config.namespaces);

    // Boot up runtime we need to execute Skills
    let skill_executor = SkillExecutor::new(csi_drivers, skill_provider.api());

    // Boot up the configuration observer
    let loaders = Box::new(
        NamespaceDescriptionLoaders::new(app_config.operator_config)
            .context("Unable to read the configuration for namespaces")?,
    );

    let mut configuration_observer = ConfigurationObserver::with_config(
        skill_provider.api(),
        loaders,
        app_config.namespace_update_interval,
    );

    // Wait for first pass of the configuration so that the configured skills are loaded
    configuration_observer.wait_for_ready().await;

    let shell_shutdown = shell::run(
        app_config.tcp_addr,
        skill_executor.api(),
        skill_provider.api(),
        shutdown_signal,
    )
    .await;

    Ok(async {
        // Make skills available via http interface. If we get the signal for shutdown the future
        // will complete.
        if let Err(e) = shell_shutdown.await {
            // We do **not** want to bubble up an error during shell initialization or execution. We
            // want to shutdown the other actors before finishing this function.
            error!("Could not boot shell: {e}");
        }

        // Shutdown everything we started. We reverse the order for the shutdown so all the required
        // actors are still answering for each component.
        configuration_observer.wait_for_shutdown().await;
        skill_executor.wait_for_shutdown().await;
        skill_provider.wait_for_shutdown().await;
        tokenizers.wait_for_shutdown().await;
        inference.wait_for_shutdown().await;
    })
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::future::ready;
    use std::sync::LazyLock;
    use std::time::Duration;

    use configuration_observer::OperatorConfig;
    use dotenvy::dotenv;
    use tokio_test::assert_ok;

    use super::*;

    /// API Token used by tests to authenticate requests.
    pub fn api_token() -> &'static str {
        static API_TOKEN: LazyLock<String> = LazyLock::new(|| {
            drop(dotenv());
            env::var("AA_API_TOKEN").expect("AA_API_TOKEN variable not set")
        });
        &API_TOKEN
    }

    /// Inference address used by tests.
    pub fn inference_address() -> &'static str {
        static AA_INFERENCE_ADDRESS: LazyLock<String> = LazyLock::new(|| {
            drop(dotenv());
            env::var("AA_INFERENCE_ADDRESS")
                .unwrap_or_else(|_| "https://api.aleph-alpha.com".to_owned())
        });
        &AA_INFERENCE_ADDRESS
    }

    /// Inference address used by tests.
    pub fn document_index_address() -> &'static str {
        static DOCUMENT_INDEX_ADDRESS: LazyLock<String> = LazyLock::new(|| {
            drop(dotenv());
            env::var("DOCUMENT_INDEX_ADDRESS")
                .unwrap_or_else(|_| "https://document-index.aleph-alpha.com".to_owned())
        });
        &DOCUMENT_INDEX_ADDRESS
    }

    // tests if the shutdown procedure is executed properly (not blocking)
    #[tokio::test]
    async fn shutdown() {
        let config = AppConfig {
            tcp_addr: "127.0.0.1:8888".parse().unwrap(),
            inference_addr: "https://api.aleph-alpha.com".to_owned(),
            operator_config: OperatorConfig::empty(),
            namespace_update_interval: Duration::from_secs(10),
            log_level: "info".to_owned(),
            open_telemetry_endpoint: None,
        };

        let shutdown_completed = super::run(config, ready(())).await.unwrap();

        // wasm runtime needs some time to shutdown (at least on Daniel's machine), so the time out
        // has been increased to 2sec
        let r = tokio::time::timeout(Duration::from_secs(2), shutdown_completed).await;
        assert_ok!(r);
    }
}
