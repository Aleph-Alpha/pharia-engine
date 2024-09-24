use anyhow::Error;
use tokio::signal;

use pharia_kernel::{initialize_tracing, AppConfig, Kernel};

#[tokio::main]
async fn main() -> Result<(), Error> {
    drop(dotenvy::dotenv());
    let app_config = AppConfig::from_env()?;
    initialize_tracing(&app_config)?;

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
