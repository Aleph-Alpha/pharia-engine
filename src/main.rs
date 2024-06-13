mod inference;

use axum::extract::{Json, State};
use axum::routing::post;
use axum::{routing::get, Router};
use axum_extra::headers::authorization::Bearer;
use axum_extra::headers::Authorization;
use axum_extra::TypedHeader;
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

    inference.shutdown().await;
}

fn http(inference_api: InferenceApi) -> Router {
    Router::new()
        .route("/complete_text", post(complete_text))
        .with_state(inference_api)
        .route("/", get(|| async { "Hello, world!" }))
}

async fn complete_text(
    State(mut inference_api): State<InferenceApi>,
    bearer: TypedHeader<Authorization<Bearer>>,
    Json(args): Json<CompleteTextParameters>,
) -> String {
    inference_api
        .complete_text(args, bearer.token().to_owned())
        .await
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
        http::{self, header, Request},
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
    async fn api_token_missing() {
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
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            String::from_utf8(body.to_vec()).unwrap(),
            "Header of type `authorization` was missing".to_owned()
        );
    }

    #[cfg_attr(not(feature = "test_inference"), ignore)]
    #[tokio::test]
    async fn complete_text() {
        let api_token = std::env::var("AA_API_TOKEN").expect("AA_API_TOKEN variable not set");
        let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
        auth_value.set_sensitive(true);

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
                    .header(header::AUTHORIZATION, auth_value)
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
