use anyhow::Error;
use tokio::signal;

use pharia_kernel::AppConfig;

#[tokio::main]
async fn main() -> Result<(), Error> {
    drop(dotenvy::dotenv());
    let app_config = AppConfig::from_env();

    pharia_kernel::run(app_config, shutdown_signal())
        .await? // We booted everything up and are operational
        .await; // We shut everything down, and released all resources.
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
