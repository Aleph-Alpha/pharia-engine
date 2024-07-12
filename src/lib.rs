mod config;
mod inference;
mod registries;
mod shell;
mod skills;

use futures::Future;
use tracing::error;

use self::{inference::Inference, skills::SkillExecutor};

pub use config::AppConfig;

pub async fn run(
    app_config: AppConfig,
    shutdown_signal: impl Future<Output = ()> + Send + 'static,
) -> impl Future<Output = ()> {
    // Boot up the drivers which power the CSI. Right now we only have inference.
    let inference = Inference::new(app_config.inference_addr);

    // Boot up runtime we need to execute Skills
    let skill_executor = SkillExecutor::new(inference.api());
    let skill_executor_api = skill_executor.api();

    let shell_shutdown = shell::run(app_config.tcp_addr, skill_executor_api, shutdown_signal).await;

    async {
        // Make skills available via http interface. If we get the signal for shutdown the future
        // will complete.
        if let Err(e) = shell_shutdown.await {
            // We do **not** want to bubble up an error during shell initialization or execution. We
            // want to shutdown the other actors before finishing this function.
            error!("Could not boot shell: {e}");
        }

        // Shutdown everything we started. We reverse the order for the shutdown so all the required
        // actors are still answering for each component.
        skill_executor.wait_for_shutdown().await;
        inference.wait_for_shutdown().await;
    }
}

#[cfg(test)]
mod tests {

    use std::future::ready;
    use std::time::Duration;

    use tokio_test::assert_ok;

    use super::*;

    // tests if the shutdown procedure is executed properly (not blocking)
    #[tokio::test]
    async fn shutdown() {
        let config = AppConfig {
            tcp_addr: "127.0.0.1:8888".parse().unwrap(),
            inference_addr: "https://api.aleph-alpha.com".to_owned(),
        };

        //wasm runtime needs some time to shutdown (at least on Daniel's machine), so the time out
        //has been increased to 2sec
        let r =
            tokio::time::timeout(Duration::from_secs(2), super::run(config, ready(())).await).await;
        assert_ok!(r);
    }
}
