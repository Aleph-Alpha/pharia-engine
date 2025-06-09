/// Expose the CSI (Cognitive System Interface which is offered to skills) via HTTP.
///
/// This allows users to test and debug Skills on their machine while still having access
/// to the full functionality the Kernel offers as a runtime environment.
mod v0_2;
mod v0_3;
mod v1;

use axum::{
    Json, Router,
    extract::{FromRef, State},
    http::StatusCode,
    routing::post,
};
use axum_extra::{
    TypedHeader,
    headers::{Authorization, authorization::Bearer},
};
use semver::VersionReq;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    csi::{CsiError, RawCsi},
    logging::TracingContext,
    skills::SupportedVersion,
};

pub fn http<T>() -> Router<T>
where
    T: CsiProvider + Clone + Send + Sync + 'static,
    T::Csi: RawCsi + Clone + Sync + Send + 'static,
{
    Router::new()
        .nest("/csi/v1", v1::http())
        // Legacy CSI route
        .route("/csi", post(http_csi_handle::<T::Csi>))
}

pub trait CsiProvider {
    type Csi;
    fn csi(&self) -> &Self::Csi;
}

/// Wrapper around Raw Csi API for the csi shell. We use this strict alias to enable extracting a
/// reference from the [`AppState`] using a [`FromRef`] implementation.
pub struct CsiState<M>(pub M);

impl<T> FromRef<T> for CsiState<T::Csi>
where
    T: CsiProvider,
    T::Csi: Clone,
{
    fn from_ref(app_state: &T) -> CsiState<T::Csi> {
        CsiState(app_state.csi().clone())
    }
}

async fn http_csi_handle<C>(
    State(CsiState(csi)): State<CsiState<C>>,
    bearer: TypedHeader<Authorization<Bearer>>,
    Json(args): Json<VersionedCsiRequest>,
) -> (StatusCode, Json<Value>)
where
    C: RawCsi + Clone + Sync,
{
    let drivers = csi;
    let tracing_context = TracingContext::current();
    let result = match args {
        VersionedCsiRequest::V0_2(request) => {
            request
                .respond(&drivers, bearer.token().to_owned(), tracing_context)
                .await
        }
        VersionedCsiRequest::V0_3(request) => {
            request
                .respond(&drivers, bearer.token().to_owned(), tracing_context)
                .await
        }
        VersionedCsiRequest::Unknown(request) => Err(request.into()),
    };
    match result {
        Ok(result) => (StatusCode::OK, Json(result)),
        Err(e) => (e.status_code(), Json(json!(e.to_string()))),
    }
}

/// This represents the versioned interactions with the CSI.
/// The members of this enum provide the glue code to translate between a function
/// defined in a versioned WIT world and the `Csi` trait.
/// By introducing this abstraction, we can expose a versioned interface of the CSI over http.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case", tag = "version")]
enum VersionedCsiRequest {
    #[serde(rename = "0.3")]
    V0_3(v0_3::CsiRequest),
    #[serde(rename = "0.2")]
    V0_2(v0_2::CsiRequest),
    #[serde(untagged)]
    Unknown(UnknownCsiRequest),
}

#[derive(Debug, thiserror::Error)]
enum CsiShellError {
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
    #[error(
        "The CSI function {0} is not supported by this Kernel installation yet. Try updating your Kernel version or downgrading your SDK."
    )]
    UnknownFunction(String),
    #[error(
        "The specified CSI version is not supported by this Kernel installation yet. Try updating your Kernel version or downgrading your SDK."
    )]
    NotSupported,
    #[error("This CSI version is no longer supported by the Kernel. Try upgrading your SDK.")]
    NoLongerSupported,
    #[error("A valid CSI version is required. Try upgrading your SDK.")]
    InvalidVersion,
}

impl CsiShellError {
    fn from_csi_error(csi_error: CsiError) -> Self {
        match csi_error {
            error @ (CsiError::Inference(_) | CsiError::Any(_)) => {
                CsiShellError::Internal(error.into())
            }
        }
    }

    fn status_code(&self) -> StatusCode {
        match self {
            CsiShellError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            // We use `BAD_REQUEST` (400) for validation error as it is more commonly used.
            // `UNPROCESSABLE_ENTITY` (422) is an alternative, but it may surprise users as it is less commonly
            // known
            _ => StatusCode::BAD_REQUEST,
        }
    }
}

impl From<CsiShellError> for (StatusCode, Json<Value>) {
    fn from(e: CsiShellError) -> Self {
        (e.status_code(), Json(json!(e.to_string())))
    }
}

#[derive(Deserialize)]
struct UnknownCsiRequest {
    version: Option<String>,
}

impl From<UnknownCsiRequest> for CsiShellError {
    fn from(e: UnknownCsiRequest) -> Self {
        match e.version.map(|v| VersionReq::parse(&v)) {
            Some(Ok(req)) if req.comparators.len() == 1 => {
                let max_supported_version = SupportedVersion::latest_supported_version();
                let comp = req.comparators.first().unwrap();
                // Only applies to unknown versions. If we parse `1.x.x` as `1` then we are only doing a major version check and minor version only applies `0.x`
                if comp.major > max_supported_version.major
                    || (comp.major == max_supported_version.major
                        && comp.minor.is_some_and(|m| m > max_supported_version.minor))
                {
                    CsiShellError::NotSupported
                } else {
                    CsiShellError::NoLongerSupported
                }
            }
            // If the user passes in a random string, the parse will fail and we will end up down here
            Some(Ok(_) | Err(_)) | None => CsiShellError::InvalidVersion,
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt as _;
    use mime::TEXT_EVENT_STREAM;
    use reqwest::{
        Method,
        header::{AUTHORIZATION, CONTENT_TYPE},
    };
    use tokio::sync::mpsc;
    use tower::ServiceExt as _;

    use crate::{
        csi::tests::RawCsiDouble,
        inference::{
            Completion, CompletionEvent, CompletionRequest, FinishReason, InferenceError,
            TokenUsage,
        },
        shell::tests::dummy_auth_value,
    };

    use super::*;

    #[test]
    fn no_longer_supported_version() {
        let request = UnknownCsiRequest {
            version: Some("0.1.0".to_string()),
        };
        let error: CsiShellError = request.into();
        assert!(matches!(error, CsiShellError::NoLongerSupported));
    }

    #[test]
    fn not_supported_minor_version() {
        let request = UnknownCsiRequest {
            version: Some("0.9.0".to_string()),
        };
        let error: CsiShellError = request.into();
        assert!(matches!(error, CsiShellError::NotSupported));
    }

    #[test]
    fn not_supported_major_version() {
        let request = UnknownCsiRequest {
            version: Some("1.0.0".to_string()),
        };
        let error: CsiShellError = request.into();
        assert!(matches!(error, CsiShellError::NotSupported));
    }

    #[test]
    fn invalid_version() {
        let request = UnknownCsiRequest {
            version: Some("invalid".to_string()),
        };
        let error: CsiShellError = request.into();
        assert!(matches!(error, CsiShellError::InvalidVersion));
    }

    #[test]
    fn complete_request() {
        // Given a request in JSON format
        let request = json!({
            "version": "0.2",
            "function": "complete",
            "prompt": "Hello",
            "model": "pharia-1-llm-7b-control",
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

    #[test]
    fn chunk_request() {
        let request = json!({
            "version": "0.2",
            "function": "chunk",
            "text": "Hello",
            "params": {
                "model": "pharia-1-llm-7b-control",
                "max_tokens": 128
            }
        });

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(matches!(
            result,
            Ok(VersionedCsiRequest::V0_2(v0_2::CsiRequest::Chunk(chunk_request))) if chunk_request.text == "Hello"
        ));
    }

    #[test]
    fn select_language_request() {
        let request = json!({
            "version": "0.2",
            "function": "select_language",
            "text": "Hello",
            "languages": ["eng", "deu"]
        });

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(matches!(
            result,
            Ok(VersionedCsiRequest::V0_2(v0_2::CsiRequest::SelectLanguage(select_language_request))) if select_language_request.text == "Hello"
        ));
    }

    #[test]
    fn complete_all_request() {
        // Given a request in JSON format
        let request = json!({
            "version": "0.2",
            "function": "complete_all",
            "requests": [
                {
                    "prompt": "Hello",
                    "model": "pharia-1-llm-7b-control",
                    "params": {
                        "max_tokens": 128,
                        "temperature": null,
                        "top_k": null,
                        "top_p": null,
                        "stop": []
                    }
                },
                {
                    "prompt": "Hello",
                    "model": "pharia-1-llm-7b-control",
                    "params": {
                        "max_tokens": 128,
                        "temperature": null,
                        "top_k": null,
                        "top_p": null,
                        "stop": []
                    }
                }
            ]
        });

        // When it is deserialized into a `VersionedCsiRequest`
        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        // Then it should be deserialized successfully
        assert!(matches!(
            result,
            Ok(VersionedCsiRequest::V0_2(v0_2::CsiRequest::CompleteAll { requests })) if requests.len() == 2
        ));
    }

    #[test]
    fn search_request() {
        let request = json!({
            "version": "0.2",
            "function": "search",
            "query": "Hello",
            "index_path": {
                "namespace": "Kernel",
                "collection": "test",
                "index": "asym-64"
            },
            "max_results": 10,
            "min_score": null,
        });

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(matches!(
            result,
            Ok(VersionedCsiRequest::V0_2(v0_2::CsiRequest::Search(search_request))) if search_request.query == "Hello"
        ));
    }

    #[test]
    fn chat_request() {
        let request = json!({
            "version": "0.2",
            "function": "chat",
            "model": "pharia-1-llm-7b-control",
            "messages": [
                {
                    "role": "user",
                    "content": "Hello"
                }
            ],
            "params": {
                "max_tokens": 128,
                "temperature": null,
                "top_p": null
            }
        });

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(matches!(
            result,
            Ok(VersionedCsiRequest::V0_2(v0_2::CsiRequest::Chat(chat_request))) if chat_request.model == "pharia-1-llm-7b-control"
        ));
    }

    #[test]
    fn documents_request() {
        let request = json!({
            "version": "0.2",
            "function": "documents",
            "requests": [
                {
                    "namespace": "Kernel",
                    "collection": "test",
                    "name": "kernel-docs"
                },
                {
                    "namespace": "Kernel",
                    "collection": "test",
                    "name": "kernel-docs"
                }
            ]
        });

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(matches!(
            result,
            Ok(VersionedCsiRequest::V0_2(v0_2::CsiRequest::Documents { requests })) if requests.len() == 2
        ));
    }

    #[test]
    fn document_metadata_request() {
        // Given a request in JSON format
        let request = json!({
            "version": "0.2",
            "function": "document_metadata",
            "document_path": {
                "namespace": "Kernel",
                "collection": "test",
                "name": "kernel/docs"
            }
        });

        // When it is deserialized into a `VersionedCsiRequest`
        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        // Then it should be deserialized successfully
        assert!(matches!(
            result,
            Ok(VersionedCsiRequest::V0_2(v0_2::CsiRequest::DocumentMetadata { document_path })) if document_path.namespace == "Kernel" && document_path.collection == "test" && document_path.name == "kernel/docs"
        ));
    }

    #[test]
    fn unknown_request() {
        let request = json!({
            "version": "0.2",
            "function": "not-implemented"
        });

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(matches!(
            result,
            Ok(VersionedCsiRequest::V0_2(v0_2::CsiRequest::Unknown { function: Some(function) })) if function == "not-implemented"
        ));
    }

    #[tokio::test]
    async fn http_csi_handle_returns_completion() {
        // Given a versioned csi request
        let prompt = "Say hello to Homer";
        let body = json!({
            "version": "0.2",
            "function": "complete",
            "model": "pharia-1-llm-7b-control",
            "prompt": prompt,
            "params": {
                "max_tokens": 1,
                "temperature": null,
                "top_k": null,
                "top_p": null,
                "stop": [],
            },
        });

        // When
        #[derive(Clone)]
        struct CsiStub;
        impl RawCsiDouble for CsiStub {
            async fn complete(
                &self,
                _: String,
                _: TracingContext,
                request: Vec<CompletionRequest>,
            ) -> anyhow::Result<Vec<Completion>> {
                Ok(vec![Completion::from_text(request[0].prompt.clone())])
            }
        }
        let app_state = ProviderStub::new(CsiStub);
        let http = http().with_state(app_state);

        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, dummy_auth_value())
                    .uri("/csi")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_value["text"].as_str().unwrap(), prompt);
    }

    #[tokio::test]
    async fn http_csi_handle_returns_completion_stream() {
        // Given a versioned csi request and a stub csi that returns three events for completions
        #[derive(Clone)]
        struct RawCsiStub;

        impl RawCsiDouble for RawCsiStub {
            async fn completion_stream(
                &self,
                _auth: String,
                _tracing_context: TracingContext,
                _request: CompletionRequest,
            ) -> mpsc::Receiver<Result<CompletionEvent, InferenceError>> {
                let (sender, receiver) = mpsc::channel(1);
                tokio::spawn(async move {
                    let append = CompletionEvent::Append {
                        text: "Say hello to Homer".to_owned(),
                        logprobs: vec![],
                    };
                    let end = CompletionEvent::End {
                        finish_reason: FinishReason::Stop,
                    };
                    let usage = CompletionEvent::Usage {
                        usage: TokenUsage {
                            prompt: 0,
                            completion: 0,
                        },
                    };
                    sender.send(Ok(append)).await.unwrap();
                    sender.send(Ok(end)).await.unwrap();
                    sender.send(Ok(usage)).await.unwrap();
                });
                receiver
            }
        }

        let body = json!({
            "model": "pharia-1-llm-7b-control",
            "prompt": "Hi",
            "params": {
                "max_tokens": 1,
                "stop": [],
                "return_special_tokens": true,
                "logprobs": "no",
            },
        });

        // When
        let app_state = ProviderStub::new(RawCsiStub);
        let http = http().with_state(app_state);

        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, dummy_auth_value())
                    .uri("/csi/v1/completion_stream")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then we get the expected events
        assert_eq!(resp.status(), StatusCode::OK);
        let content_type = resp.headers().get(CONTENT_TYPE).unwrap();
        assert_eq!(content_type, TEXT_EVENT_STREAM.as_ref());

        let body_text = resp.into_body().collect().await.unwrap().to_bytes();
        let expected_body = "event: append\n\
            data: {\"text\":\"Say hello to Homer\",\"logprobs\":[]}\n\
            \n\
            event: end\n\
            data: {\"finish_reason\":\"stop\"}\n\
            \n\
            event: usage\n\
            data: {\"usage\":{\"prompt\":0,\"completion\":0}}\n\
            \n\
            ";
        assert_eq!(body_text, expected_body);
    }

    #[derive(Clone)]
    struct ProviderStub<T> {
        csi: T,
    }

    impl<T> ProviderStub<T> {
        fn new(csi: T) -> Self {
            Self { csi }
        }
    }

    impl<T> CsiProvider for ProviderStub<T> {
        type Csi = T;

        fn csi(&self) -> &T {
            &self.csi
        }
    }
}
