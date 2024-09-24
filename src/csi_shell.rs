
use std::{future::Future, iter::once, net::SocketAddr};

use anyhow::Context;
use axum::{
    extract::MatchedPath,
    http::{header::AUTHORIZATION, Request, StatusCode},
    routing::post,
    Json, Router,
};
use axum_extra::{
    headers::{authorization::Bearer, Authorization},
    TypedHeader,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{net::TcpListener, task::JoinHandle};
use tower::ServiceBuilder;
use tower_http::{
    compression::CompressionLayer,
    decompression::DecompressionLayer,
    sensitive_headers::SetSensitiveRequestHeadersLayer,
    trace::{DefaultOnRequest, DefaultOnResponse, TraceLayer},
};
use tracing::{error, info, info_span, Level};

struct CsiShell {
    handle: JoinHandle<()>
}

impl CsiShell {
    /// Start a shell listening to incoming requests at the given address. Successful construction
    /// implies that the listener is bound to the endpoint.
    pub async fn new(
        addr: impl Into<SocketAddr>,
        shutdown_signal: impl Future<Output = ()> + Send + 'static,
    ) -> Result<Self, anyhow::Error> {
        let addr = addr.into();
        // It is important to construct the listener outside of the `spawn` invocation. We need to
        // guarantee the listener is already bound to the port, once `Self` is constructed.
        let listener = TcpListener::bind(addr)
            .await
            .context(format!("Could not bind a tcp listener for CSI shell to '{addr}'"))?;
        info!("Listening on: {addr}");
        let handle = tokio::spawn(async {
            let res = axum::serve(listener, http())
                .with_graceful_shutdown(shutdown_signal)
                .await;
            if let Err(e) = res {
                error!("Error terminating CSI shell: {e}");
            }
        });
        Ok(Self { handle })
    }

    pub async fn wait_for_shutdown(self) {
        self.handle.await.unwrap();
    }
}

pub fn http() -> Router {
    Router::new()
        .route("/csi", post(http_csi_handle))
        .layer(
            ServiceBuilder::new()
                // Mark the `Authorization` request header as sensitive so it doesn't show in logs
                .layer(SetSensitiveRequestHeadersLayer::new(once(AUTHORIZATION)))
                // High level logging of requests and responses
                .layer(
                    TraceLayer::new_for_http()
                        .make_span_with(|request: &Request<_>| {
                            // Log the matched route's path (with placeholders not filled in).
                            // Use request.uri() or OriginalUri if you want the real path.
                            let matched_path = request
                                .extensions()
                                .get::<MatchedPath>()
                                .map(MatchedPath::as_str);

                            info_span!(
                                "http_request",
                                method = ?request.method(),
                                matched_path,
                            )
                        })
                        .on_request(DefaultOnRequest::new().level(Level::INFO))
                        .on_response(DefaultOnResponse::new().level(Level::INFO)),
                )
                // Compress responses
                .layer(CompressionLayer::new())
                .layer(DecompressionLayer::new()),
        )
}

async fn http_csi_handle(
    bearer: TypedHeader<Authorization<Bearer>>,
    Json(args): Json<VersionedCsiRequest>,
) -> (StatusCode, Json<Value>) {
    (StatusCode::OK, Json(Value::String("dummy completion".to_owned())))
}    



/// This structs allows us to represent versioned interactions with the CSI.
/// The members of this enum provide the glue code to translate between a function
/// defined in a versioned wit world and the `CsiForSkills` trait.
/// By introducing this abstraction, we can expose a versioned interface of the CSI over http.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "version")]
pub enum VersionedCsiRequest {
    V0_2(V0_2CsiRequest),
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "function")]
pub enum V0_2CsiRequest {
    Complete(CompletionRequest),
}

#[derive(Deserialize, Serialize)]
pub struct CompletionRequest {
    pub model: String,
    pub prompt: String,
    pub params: CompletionParams,
}

#[derive(Deserialize, Serialize)]
pub struct CompletionParams {
    pub max_tokens: u32,
    pub temperature: Option<f32>,
    pub top_k: Option<u32>,
    pub top_p: Option<f32>,
    pub stop: Vec<String>,
}


#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use axum::{
        body::Body,
        http::{self, header, Request},
    };
    use http_body_util::BodyExt;
    use tower::util::ServiceExt;

    #[test]
    fn csi_v_2_request_is_deserialized() {
        // Given a request in JSON format
        let request = json!({
            "version": "v0_2",
            "function": "complete",
            "prompt": "Hello",
            "model": "llama-3.1-8b-instruct",
            "params": {
                "max_tokens": 128,
                "temperature": null,
                "top_k": null,
                "top_p": null,
                "stop": []
            }
        });

        // When it is deserialized into a `VersionedCsiRequest`
        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        // Then it should be deserialized successfully
        assert!(result.is_ok());
    }
 

    #[tokio::test]
    async fn http_csi_handle_returns_completion() {
        // Given a versioned csi request 
        let completion_request = CompletionRequest {
            model: "llama-3.1-8b-instruct".to_owned(),
            prompt: "Hello".to_owned(),
            params: CompletionParams {
                max_tokens: 128,
                temperature: None,
                top_k: None,
                top_p: None,
                stop: vec![],
            }
        };
        let request = VersionedCsiRequest::V0_2(V0_2CsiRequest::Complete(completion_request));

        // When
        let api_token = "dummy auth token";
        let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
        auth_value.set_sensitive(true);
        let http = http();

        let resp = http
            .oneshot(
                Request::builder()
                    .method(http::Method::POST)
                    .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                    .header(header::AUTHORIZATION, auth_value)
                    .uri("/csi")
                    .body(Body::from(serde_json::to_string(&request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let answer = serde_json::from_slice::<String>(&body).unwrap();
        assert_eq!(answer, "dummy completion");
    }
}