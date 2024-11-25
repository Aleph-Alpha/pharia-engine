mod authorization;
mod config;
mod csi;
mod csi_shell;
mod inference;
mod language_selection;
mod logging;
mod metrics;
mod namespace_watcher;
mod registries;
mod search;
mod shell;
mod skill_loader;
mod skill_store;
mod skills;
mod tokenizers;

use std::sync::Arc;

use anyhow::{Context, Error};
use authorization::Authorization;
use csi::CsiDrivers;
use futures::Future;
use namespace_watcher::{NamespaceDescriptionLoaders, NamespaceWatcher};
use search::Search;
use shell::Shell;
use skill_loader::SkillLoader;
use skill_store::SkillStore;
use skills::Engine;
use tokenizers::Tokenizers;

use self::{inference::Inference, skills::SkillExecutor};

pub use config::AppConfig;
pub use inference::{Completion, FinishReason};
pub use logging::initialize_tracing;
pub use metrics::initialize_metrics;
pub use namespace_watcher::OperatorConfig;

pub struct Kernel {
    inference: Inference,
    tokenizers: Tokenizers,
    search: Search,
    skill_loader: SkillLoader,
    skill_store: SkillStore,
    skill_executor: SkillExecutor,
    namespace_watcher: NamespaceWatcher,
    authorization: Authorization,
    shell: Shell,
}

impl Kernel {
    /// Boots up all the actors making up the kernel. If completed the binding operation of the
    /// listener is completed, but no requests are actively handled yet.
    ///
    /// # Errors
    ///
    /// Fails if application can not read the namespace configuration. Errors in booting up the
    /// shell are only exposed once waiting for the shutdown
    pub async fn new(
        app_config: AppConfig,
        shutdown_signal: impl Future<Output = ()> + Send + 'static,
    ) -> Result<Self, Error> {
        let loaders = Box::new(
            NamespaceDescriptionLoaders::new(app_config.operator_config.clone())
                .context("Unable to read the configuration for namespaces")?,
        );
        let engine = Arc::new(
            Engine::new(app_config.use_pooling_allocator).context("engine creation failed")?,
        );

        // Boot up the drivers which power the CSI. Right now we only have inference.
        let tokenizers = Tokenizers::new(app_config.inference_addr.clone()).unwrap();
        let inference = Inference::new(app_config.inference_addr.clone());
        let search = Search::new(app_config.document_index_addr.clone());
        let csi_drivers = CsiDrivers {
            inference: inference.api(),
            search: search.api(),
            tokenizers: tokenizers.api(),
        };

        let registry_config = app_config.operator_config.registry_config();
        let skill_loader = SkillLoader::new(engine.clone(), registry_config);
        let skill_store = SkillStore::new(skill_loader.api(), app_config.namespace_update_interval);

        // Boot up runtime we need to execute Skills
        let skill_executor = SkillExecutor::new(engine, csi_drivers.clone(), skill_store.api());

        let mut namespace_watcher = NamespaceWatcher::with_config(
            skill_store.api(),
            loaders,
            app_config.namespace_update_interval,
        );

        // Wait for first pass of the configuration so that the configured skills are loaded
        namespace_watcher.wait_for_ready().await;

        let authorization = Authorization::new();

        let shell = match Shell::new(
            app_config.tcp_addr,
            skill_executor.api(),
            skill_store.api(),
            csi_drivers,
            shutdown_signal,
        )
        .await
        {
            Ok(shell) => shell,
            Err(e) => {
                // In case construction of shell goes wrong (e.g. we can not bind the port) we
                // shutdown all the other actors we created so far, so they do not live on in
                // detached threads.
                authorization.wait_for_shutdown().await;
                namespace_watcher.wait_for_shutdown().await;
                skill_executor.wait_for_shutdown().await;
                skill_store.wait_for_shutdown().await;
                skill_loader.wait_for_shutdown().await;
                tokenizers.wait_for_shutdown().await;
                inference.wait_for_shutdown().await;
                search.wait_for_shutdown().await;
                // Bubble up error, after resources have been freed
                return Err(e);
            }
        };

        Ok(Kernel {
            inference,
            tokenizers,
            search,
            skill_loader,
            skill_store,
            skill_executor,
            namespace_watcher,
            authorization,
            shell,
        })
    }

    /// Waits for the kernel to shut down and free all its resources. The shutdown begins once the
    /// shutdown signal passed in [`Kernel::new`] runs to completion
    pub async fn wait_for_shutdown(self) {
        // Shutdown everything we started. We reverse the order for the shutdown so all the required
        // actors are still answering for each component.
        self.shell.wait_for_shutdown().await;
        self.authorization.wait_for_shutdown().await;
        self.namespace_watcher.wait_for_shutdown().await;
        self.skill_executor.wait_for_shutdown().await;
        self.skill_store.wait_for_shutdown().await;
        self.skill_loader.wait_for_shutdown().await;
        self.tokenizers.wait_for_shutdown().await;
        self.inference.wait_for_shutdown().await;
        self.search.wait_for_shutdown().await;
    }
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::future::ready;
    use std::sync::LazyLock;
    use std::time::Duration;

    use dotenvy::dotenv;
    use namespace_watcher::OperatorConfig;
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
            metrics_addr: "127.0.0.1:0".parse().unwrap(),
            inference_addr: "https://api.aleph-alpha.com".to_owned(),
            document_index_addr: "https://document-index.aleph-alpha.com".to_owned(),
            operator_config: OperatorConfig::empty(),
            namespace_update_interval: Duration::from_secs(10),
            log_level: "info".to_owned(),
            open_telemetry_endpoint: None,
            use_pooling_allocator: false,
        };
        let kernel = Kernel::new(config, ready(())).await.unwrap();

        let shutdown_completed = kernel.wait_for_shutdown();

        // wasm runtime needs some time to shutdown (at least on Daniel's machine), so the time out
        // has been increased to 2sec
        let r = tokio::time::timeout(Duration::from_secs(2), shutdown_completed).await;
        assert_ok!(r);
    }
}
