mod inference;

use axum::extract::{Json, State};
use axum::routing::post;
use axum::{routing::get, Router};
use inference::{CompleteTextParameters, Inference, InferenceApi};
use std::env;
use tokio::net::TcpListener;
use tokio::signal;

#[tokio::main]
async fn main() {
    // .env files are optional
    let _ = dotenvy::dotenv();

    let inference = Inference::new();
    let inference_api = inference.api();

    let host = env::var("HOST").expect("HOST variable not set");
    let port = env::var("PORT").expect("PORT variable not set");
    let bind = format!("{host}:{port}");
    let listener = TcpListener::bind(bind)
        .await
        .expect("Could not bind server, please check host and port"); //todo:
                                                                      //error handling
    axum::serve(listener, http(inference_api))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Could not start server!"); //todo: error handling
}

fn http(inference_api: InferenceApi) -> Router {
    Router::new()
        .route("/complete_text", post(complete_text))
        .with_state(inference_api)
        .route("/", get(|| async { "Hello, world!" }))
}

async fn complete_text(
    State(mut inference_api): State<InferenceApi>,
    Json(args): Json<CompleteTextParameters>,
) -> String {
    inference_api.complete_text(args).await
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
    use axum::{
        body::Body,
        http::{self, Request},
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    #[tokio::test]
    async fn hello_world() {
        let http = http(Inference::new().api());
        let resp = http
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"Hello, world!");
    }

    #[tokio::test]
    async fn complete_text() {
        let http = http(Inference::new().api());
        let ctp = CompleteTextParameters {
            prompt: "An apple a day".to_owned(),
            model: "luminous-nextgen-7b".to_owned(),
            max_tokens: 10,
        };
        let resp = http
            .oneshot(
                Request::builder()
                    .method(http::Method::POST)
                    .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                    .uri("/complete_text")
                    .body(Body::from(serde_json::to_string(&ctp).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert!(&body[..].starts_with(b" keeps the doctor away"));
    }
}
