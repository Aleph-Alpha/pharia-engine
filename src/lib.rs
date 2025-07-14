mod authorization;
mod chunking;
mod config;
mod csi;
mod csi_shell;
mod feature_set;
mod hardcoded_skills;
mod http;
mod inference;
mod language_selection;
mod logging;
mod mcp;
mod metrics;
mod namespace_watcher;
mod registries;
mod search;
mod shell;
mod skill;
mod skill_cache;
mod skill_driver;
mod skill_loader;
mod skill_runtime;
mod skill_store;
mod tokenizers;
mod tool;
mod wasm;

use std::sync::Arc;

use anyhow::{Context, Error};
use authorization::Authorization;
use futures::Future;
use namespace_watcher::{NamespaceDescriptionLoaders, NamespaceWatcher};
use search::Search;
use shell::Shell;
use skill_loader::SkillLoader;
use skill_store::SkillStore;
use tokenizers::Tokenizers;
use tool::ToolRuntime;
use tracing::error;
use wasm::Engine;

use crate::{csi::CsiDrivers, mcp::Mcp, shell::ShellState};

use self::{inference::Inference, skill_runtime::SkillRuntime};

pub use config::{AppConfig, OtelConfig};
pub use feature_set::FeatureSet;
pub use logging::{initialize_tracing, tracer_provider};
pub use metrics::initialize_metrics;
pub use namespace_watcher::NamespaceConfigs;

/// Wasmtime + Cranelift do a lot of small allocations for compilation,
/// which can lead to heap fragmentation. Jemalloc does better at handling this,
/// so we use this instead to make it easier to give the memory back to the OS.
///
/// This is in the lib.rs not main.rs so that we can use it for all of our tests,
/// to make sure that we test it as thoroughly as possible.
#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

pub struct Kernel {
    inference: Inference,
    tokenizers: Tokenizers,
    search: Search,
    skill_loader: SkillLoader,
    skill_store: SkillStore,
    tool: ToolRuntime,
    mcp: Mcp,
    skill_runtime: SkillRuntime,
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
            NamespaceDescriptionLoaders::new(app_config.namespaces().clone())
                .context("Unable to read the configuration for namespaces")
                .inspect_err(|e| error!(target: "pharia-kernel::initialization", "{e:#}"))?,
        );
        let engine = Arc::new(
            Engine::new(app_config.engine_config())
                .context("engine creation failed")
                .inspect_err(|e| error!(target: "pharia-kernel::initialization", "{e:#}"))?,
        );

        // Boot up the drivers which power the CSI.
        let tokenizers = Tokenizers::new(app_config.inference_url().to_owned()).unwrap();
        let inference = Inference::new(app_config.inference_url().to_owned());
        let search = Search::new(app_config.document_index_url().to_owned());
        let tool = ToolRuntime::new();
        let mcp = Mcp::from_subscriber(tool.api());
        let csi_drivers = CsiDrivers {
            inference: inference.api(),
            search: search.api(),
            tokenizers: tokenizers.api(),
            tool: tool.api(),
        };

        let registry_config = app_config.namespaces().registry_config();
        let skill_loader = SkillLoader::from_config(engine, registry_config);
        let skill_store = SkillStore::new(
            skill_loader.api(),
            app_config.namespace_update_interval(),
            app_config.desired_skill_cache_memory_usage(),
        );

        // Boot up the runtime we need to execute Skills
        let skill_runtime = SkillRuntime::new(csi_drivers.clone(), skill_store.api());

        let mut namespace_watcher = NamespaceWatcher::with_config(
            skill_store.api(),
            tool.api(),
            mcp.api(),
            loaders,
            app_config.namespace_update_interval(),
        );

        // Wait for the first pass of the configuration so that the configured skills are loaded
        namespace_watcher.wait_for_ready().await;

        let authorization = Authorization::new(app_config.authorization_url().to_owned());

        let app_state = ShellState::new(
            skill_runtime.api(),
            skill_store.api(),
            authorization.api(),
            mcp.api(),
            csi_drivers.clone(),
        );

        let shell = match Shell::new(
            app_config.pharia_ai_feature_set(),
            app_config.kernel_address(),
            app_state,
            shutdown_signal,
        )
        .await
        {
            Ok(shell) => shell,
            Err(e) => {
                // In case construction of shell goes wrong (e.g. we can not bind the port) we
                // shut down all the other actors we created so far, so they do not live on in
                // detached threads.
                //
                // We shutdown actors that consume other actors first, so they never try to
                // talk to an actor that is gone. For example: The namespace watcher talks
                // to the tool and skill store actor, so we first shutdown the namespace watcher.
                authorization.wait_for_shutdown().await;
                namespace_watcher.wait_for_shutdown().await;
                skill_runtime.wait_for_shutdown().await;
                mcp.wait_for_shutdown().await;
                tool.wait_for_shutdown().await;
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
            tool,
            mcp,
            skill_runtime,
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
        self.skill_runtime.wait_for_shutdown().await;
        self.mcp.wait_for_shutdown().await;
        self.tool.wait_for_shutdown().await;
        self.skill_store.wait_for_shutdown().await;
        self.skill_loader.wait_for_shutdown().await;
        self.tokenizers.wait_for_shutdown().await;
        self.inference.wait_for_shutdown().await;
        self.search.wait_for_shutdown().await;
    }
}

#[cfg(test)]
mod tests {
    use std::{env, future::ready, sync::LazyLock, time::Duration};

    use dotenvy::dotenv;
    use tokio_test::assert_ok;

    use super::*;

    /// API Token used by tests to authenticate requests.
    pub fn api_token() -> &'static str {
        static API_TOKEN: LazyLock<String> = LazyLock::new(|| {
            drop(dotenv());
            env::var("PHARIA_AI_TOKEN").expect("PHARIA_AI_TOKEN variable not set")
        });
        &API_TOKEN
    }

    /// Inference address used by tests.
    pub fn inference_url() -> &'static str {
        static INFERENCE_URL: LazyLock<String> = LazyLock::new(|| {
            drop(dotenv());
            env::var("INFERENCE_URL")
                .unwrap_or_else(|_| "https://inference-api.product.pharia.com".to_owned())
        });
        &INFERENCE_URL
    }

    /// Authorization address used by tests.
    pub fn authorization_url() -> &'static str {
        static AUTHORIZATION_URL: LazyLock<String> = LazyLock::new(|| {
            drop(dotenv());
            env::var("AUTHORIZATION_URL")
                .unwrap_or_else(|_| "https://pharia-iam.product.pharia.com".to_owned())
        });
        &AUTHORIZATION_URL
    }

    /// Inference address used by tests.
    pub fn document_index_url() -> &'static str {
        static DOCUMENT_INDEX_URL: LazyLock<String> = LazyLock::new(|| {
            drop(dotenv());
            env::var("DOCUMENT_INDEX_URL")
                .unwrap_or_else(|_| "https://document-index.product.pharia.com".to_owned())
        });
        &DOCUMENT_INDEX_URL
    }

    // tests if the shutdown procedure is executed properly (not blocking)
    #[tokio::test]
    async fn shutdown() {
        let config = AppConfig::default()
            .with_kernel_address("127.0.0.1:8888".parse().unwrap())
            .with_metrics_address("127.0.0.1:0".parse().unwrap());
        let kernel = Kernel::new(config, ready(())).await.unwrap();

        let shutdown_completed = kernel.wait_for_shutdown();

        // wasm runtime needs some time to shutdown (at least on Daniel's machine), so the time out
        // has been increased to 2sec
        let r = tokio::time::timeout(Duration::from_secs(2), shutdown_completed).await;
        assert_ok!(r);
    }
}
