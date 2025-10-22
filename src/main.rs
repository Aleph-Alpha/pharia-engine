use anyhow::Error;
use tokio::signal;

use pharia_engine::{AppConfig, Kernel, initialize_metrics, initialize_tracing};
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Error> {
    drop(dotenvy::dotenv());
    let app_config = AppConfig::new()?;
    let _otel_guard = initialize_tracing(app_config.as_otel_config())?;
    initialize_metrics(app_config.metrics_address())?;

    info!(
        target: "pharia-kernel::config",
        "Cache Configuration: Skill Cache = {} | Wasmtime Cache = {}",
        app_config.desired_skill_cache_memory_usage(),
        app_config.wasmtime_cache_size().unwrap_or_default()
    );

    let kernel = Kernel::new(app_config).await?;

    shutdown_signal().await;

    kernel.wait_for_shutdown().await;

    Ok(())
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
        () = ctrl_c => {},
        () = terminate => {},
    }
}
