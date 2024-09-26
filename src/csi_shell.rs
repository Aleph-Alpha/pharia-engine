use std::{future::Future, iter::once, net::SocketAddr};

use anyhow::Context;
use axum::{
    extract::{MatchedPath, State},
    http::{header::AUTHORIZATION, Request, StatusCode},
    routing::post,
    Json, Router,
};
use axum_extra::{
    headers::{authorization::Bearer, Authorization},
    TypedHeader,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::{net::TcpListener, task::JoinHandle};
use tower::ServiceBuilder;
use tower_http::{
    compression::CompressionLayer,
    decompression::DecompressionLayer,
    sensitive_headers::SetSensitiveRequestHeadersLayer,
    trace::{DefaultOnRequest, DefaultOnResponse, TraceLayer},
};
use tracing::{error, info, info_span, Level};

use crate::{
    csi::{chunking, Csi, CsiDrivers},
    inference, language_selection,
};

pub struct CsiShell {
    handle: JoinHandle<()>,
}

impl CsiShell {
    /// Start a shell listening to incoming requests at the given address. Successful construction
    /// implies that the listener is bound to the endpoint.
    pub async fn new(
        addr: impl Into<SocketAddr>,
        drivers: CsiDrivers,
        shutdown_signal: impl Future<Output = ()> + Send + 'static,
    ) -> Result<Self, anyhow::Error> {
        let addr = addr.into();
        // It is important to construct the listener outside of the `spawn` invocation. We need to
        // guarantee the listener is already bound to the port, once `Self` is constructed.
        let listener = TcpListener::bind(addr).await.context(format!(
            "Could not bind a tcp listener for CSI shell to '{addr}'"
        ))?;
        info!("Listening on: {addr}");
        let handle = tokio::spawn(async {
            let res = axum::serve(listener, http(drivers))
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

pub fn http(drivers: CsiDrivers) -> Router {
    Router::new()
        .route("/csi", post(http_csi_handle))
        .with_state(drivers)
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

pub async fn http_csi_handle(
    State(drivers): State<CsiDrivers>,
    bearer: TypedHeader<Authorization<Bearer>>,
    Json(args): Json<VersionedCsiRequest>,
) -> (StatusCode, Json<Value>) {
    let result = match args {
        VersionedCsiRequest::V0_2(request) => match request {
            V0_2CsiRequest::Complete(completion_request) => drivers
                .complete_text(bearer.token().to_owned(), completion_request.into())
                .await
                .map(|r| json!(Completion::from(r))),
            V0_2CsiRequest::Chunk(chunk_request) => drivers
                .chunk(bearer.token().to_owned(), chunk_request.into())
                .await
                .map(|r| json!(r)),
            V0_2CsiRequest::SelectLanguage(select_language_request) => drivers
                .select_language(select_language_request.into())
                .await
                .map(|r| json!(r.map(Language::from))),
            V0_2CsiRequest::CompleteAll(complete_all_request) => drivers
                .complete_all(
                    bearer.token().to_owned(),
                    complete_all_request
                        .requests
                        .into_iter()
                        .map(inference::CompletionRequest::from)
                        .collect(),
                )
                .await
                .map(|v| json!(v.into_iter().map(Completion::from).collect::<Vec<_>>())),
        },
    };
    match result {
        Ok(result) => (StatusCode::OK, Json(result)),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!(e.to_string())),
        ),
    }
}

/// This structs allows us to represent versioned interactions with the CSI.
/// The members of this enum provide the glue code to translate between a function
/// defined in a versioned wit world and the `CsiForSkills` trait.
/// By introducing this abstraction, we can expose a versioned interface of the CSI over http.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "version")]
pub enum VersionedCsiRequest {
    #[serde(rename = "0.2")]
    V0_2(V0_2CsiRequest),
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "function")]
pub enum V0_2CsiRequest {
    Complete(CompletionRequest),
    Chunk(ChunkRequest),
    SelectLanguage(SelectLanguageRequest),
    CompleteAll(CompleteAllRequest),
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CompletionRequest {
    pub model: String,
    pub prompt: String,
    pub params: CompletionParams,
}

impl From<CompletionRequest> for inference::CompletionRequest {
    fn from(value: CompletionRequest) -> Self {
        let CompletionRequest {
            model,
            prompt,
            params,
        } = value;

        Self {
            prompt,
            model,
            params: params.into(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CompletionParams {
    pub max_tokens: Option<u32>,
    pub temperature: Option<f64>,
    pub top_k: Option<u32>,
    pub top_p: Option<f64>,
    pub stop: Vec<String>,
}

impl From<CompletionParams> for inference::CompletionParams {
    fn from(value: CompletionParams) -> Self {
        let CompletionParams {
            max_tokens,
            temperature,
            top_k,
            top_p,
            stop,
        } = value;

        Self {
            max_tokens,
            temperature,
            top_k,
            top_p,
            stop,
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ContentFilter,
}

impl From<inference::FinishReason> for FinishReason {
    fn from(value: inference::FinishReason) -> Self {
        match value {
            inference::FinishReason::Stop => FinishReason::Stop,
            inference::FinishReason::Length => FinishReason::Length,
            inference::FinishReason::ContentFilter => FinishReason::ContentFilter,
        }
    }
}

#[derive(Deserialize, Serialize)]
pub struct Completion {
    pub text: String,
    pub finish_reason: FinishReason,
}

impl From<inference::Completion> for Completion {
    fn from(value: inference::Completion) -> Self {
        let inference::Completion {
            text,
            finish_reason,
        } = value;
        Self {
            text,
            finish_reason: finish_reason.into(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ChunkRequest {
    pub text: String,
    pub params: ChunkParams,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ChunkParams {
    pub model: String,
    pub max_tokens: u32,
}

impl From<ChunkRequest> for chunking::ChunkRequest {
    fn from(value: ChunkRequest) -> Self {
        let ChunkRequest {
            text,
            params: ChunkParams { model, max_tokens },
        } = value;

        Self {
            text,
            model,
            max_tokens,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SelectLanguageRequest {
    pub text: String,
    pub languages: Vec<Language>,
}

impl From<SelectLanguageRequest> for language_selection::SelectLanguageRequest {
    fn from(value: SelectLanguageRequest) -> Self {
        let SelectLanguageRequest { text, languages } = value;
        Self {
            text,
            languages: languages
                .into_iter()
                .map(language_selection::Language::from)
                .collect(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    /// english
    Eng,
    /// german
    Deu,
}

impl From<Language> for language_selection::Language {
    fn from(value: Language) -> Self {
        match value {
            Language::Eng => language_selection::Language::Eng,
            Language::Deu => language_selection::Language::Deu,
        }
    }
}
impl From<language_selection::Language> for Language {
    fn from(value: language_selection::Language) -> Self {
        match value {
            language_selection::Language::Eng => Language::Eng,
            language_selection::Language::Deu => Language::Deu,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CompleteAllRequest {
    pub requests: Vec<CompletionRequest>,
}

#[cfg(test)]
mod tests {
    use inference::tests::InferenceStub;
    use serde_json::json;

    use crate::csi::tests::dummy_csi_apis;

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
            "version": "0.2",
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
        let prompt = "Say hello to Homer";
        let completion_request = CompletionRequest {
            model: "llama-3.1-8b-instruct".to_owned(),
            prompt: prompt.to_owned(),
            params: CompletionParams {
                max_tokens: Some(128),
                temperature: None,
                top_k: None,
                top_p: None,
                stop: vec![],
            },
        };
        let request = VersionedCsiRequest::V0_2(V0_2CsiRequest::Complete(completion_request));

        // When
        let api_token = "dummy auth token";
        let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
        auth_value.set_sensitive(true);
        let inference_stub = InferenceStub::new(|r| Ok(inference::Completion::from_text(r.prompt)));
        let csi_apis = CsiDrivers {
            inference: inference_stub.api(),
            ..dummy_csi_apis()
        };
        let http = http(csi_apis);

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
        let completion = serde_json::from_slice::<Completion>(&body).unwrap();
        assert_eq!(completion.text, prompt);
    }
}
