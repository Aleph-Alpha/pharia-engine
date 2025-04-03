use anyhow::Error;
use tokio::signal;

use pharia_kernel::{AppConfig, Kernel, initialize_metrics, initialize_tracing};
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Error> {
    drop(dotenvy::dotenv());
    let app_config = AppConfig::new()?;
    initialize_tracing(&app_config)?;
    initialize_metrics(app_config.metrics_address())?;

    info!(
        "Memory request: {} | limit: {}",
        app_config
            .memory_request()
            .map_or_else(|| "None".to_owned(), |size| size.to_string()),
        app_config
            .memory_limit()
            .map_or_else(|| "None".to_owned(), |size| size.to_string())
    );
    let kernel = Kernel::new(app_config, shutdown_signal()).await?;
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
