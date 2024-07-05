mod config;
mod inference;
mod registries;
mod shell;
mod skills;

use futures::Future;
use tracing::error;

use self::{
    inference::Inference,
    skills::{SkillExecutor, WasmRuntime},
};

pub use config::AppConfig;

pub async fn run(app_config: AppConfig, shutdown_signal: impl Future<Output = ()> + Send + 'static) {
    let inference = Inference::new(app_config.inference_addr);

    let runtime = WasmRuntime::new();
    let skill_executor = SkillExecutor::new(runtime, inference.api());
    let skill_executor_api = skill_executor.api();

    if let Err(e) = shell::run(app_config.tcp_addr, skill_executor_api, shutdown_signal).await {
        error!("Could not boot shell: {}", e);
    }

    skill_executor.shutdown().await;
    inference.shutdown().await;
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

        //wasm runtime needs some time to shutdown (at least on Daniel's maschine), so the time out
        //has been increased to 2sec
        let r = tokio::time::timeout(Duration::from_secs(2), super::run(config, ready(()))).await;
        assert_ok!(r);
    }
}