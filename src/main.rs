use axum::{routing::get, Router};
use std::env;
use tokio::net::TcpListener;
use tokio::signal;

#[tokio::main]
async fn main() {
    // .env files are optional
    let _ = dotenvy::dotenv();

    let host = env::var("HOST").expect("HOST variable not set");
    let port = env::var("PORT").expect("PORT variable not set");
    let bind = format!("{host}:{port}");
    let listener = TcpListener::bind(bind)
        .await
        .expect("Could not bind server, please check host and port"); //todo:
                                                                      //error handling
    axum::serve(listener, http())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Could not start server!"); //todo: error handling
}

fn http() -> Router {
    Router::new().route("/", get(|| async { "Hello, world!" }))
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

    use super::*;
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use libc::{getpid, kill};
    use std::time::Duration;
    use tokio::time::timeout;
    use tokio_test::assert_ok;
    use tower::ServiceExt;

    #[tokio::test]
    async fn shutdown_on_sigint() {
        let shutdown = shutdown_signal();
        tokio::spawn(async move {
            unsafe {
                assert_eq!(kill(getpid(), libc::SIGINT), 0);
            }
        });
        assert_ok!(timeout(Duration::from_millis(10), shutdown).await);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shutdown_on_sigterm() {
        let shutdown = shutdown_signal();
        tokio::spawn(async {
            unsafe {
                assert_eq!(kill(getpid(), libc::SIGTERM), 0);
            }
        });
        assert_ok!(timeout(Duration::from_millis(10), shutdown).await);
    }

    #[tokio::test]
    async fn hello_world() {
        let http = http();
        let resp = http
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"Hello, world!");
    }
}
