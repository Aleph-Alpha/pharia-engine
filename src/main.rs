mod config;
mod inference;
mod shell;
mod skills;
use std::future::Future;

use crate::inference::Inference;
use crate::skills::SkillExecutor;
use config::AppConfig;
use tokio::signal;

#[tokio::main]
async fn main() {
    let app_config = AppConfig::from_env();

    run(app_config, shutdown_signal()).await
}

async fn run(app_config: AppConfig, shutdown_signal: impl Future<Output = ()> + Send + 'static) {
    let inference = Inference::new();
    let inference_api = inference.api();

    let skill_executor = SkillExecutor::new(inference_api);
    let skill_executor_api = skill_executor.api();

    if let Err(e) = shell::run(app_config.tcp_addr, skill_executor_api, shutdown_signal).await {
        eprintln!("Could not boot shell: {}", e);
    }

    skill_executor.shutdown().await;
    inference.shutdown().await;
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
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
        };

        let r = tokio::time::timeout(Duration::from_secs(1), super::run(config, ready(()))).await;
        assert_ok!(r)
    }
}
