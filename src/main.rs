use tokio::signal;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use pharia_kernel::AppConfig;

#[tokio::main]
async fn main() {
    let app_config = AppConfig::from_env();
    // Set up tracing subscriber that behaves like env_logger
    // Can switch to some other subscriber in the future
    tracing_subscriber::registry()
        .with(EnvFilter::from_env("PHARIA_KERNEL_LOG"))
        .with(tracing_subscriber::fmt::layer())
        .init();

    pharia_kernel::run(app_config, shutdown_signal())
        .await
        .await;
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
