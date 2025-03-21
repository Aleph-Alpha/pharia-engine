/// Expose the CSI (Cognitive System Interface which is offered to skills) via HTTP.
///
/// This allows users to test and debug Skills on their machine while still having access
/// to the full functionality the Kernel offers as a runtime environment.
mod v0_2;
mod v0_3;
mod v1;

use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use axum_extra::{
    TypedHeader,
    headers::{Authorization, authorization::Bearer},
};
use semver::VersionReq;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    csi::Csi,
    shell::{AppState, CsiState},
    skill_runtime::SkillRuntimeApi,
    skill_store::SkillStoreApi,
    skills::SupportedVersion,
};

pub fn http<C, R, S>() -> Router<AppState<C, R, S>>
where
    C: Csi + Clone + Sync + Send + 'static,
    R: SkillRuntimeApi + Clone + Send + Sync + 'static,
    S: SkillStoreApi + Clone + Send + Sync + 'static,
{
    Router::new()
        .nest("/csi/v1", v1::http())
        .route("/csi", post(http_csi_handle::<C>))
}

async fn http_csi_handle<C>(
    State(CsiState(csi)): State<CsiState<C>>,
    bearer: TypedHeader<Authorization<Bearer>>,
    Json(args): Json<VersionedCsiRequest>,
) -> (StatusCode, Json<Value>)
where
    C: Csi + Clone + Sync,
{
    let drivers = csi;
    let result = match args {
        VersionedCsiRequest::V0_2(request) => {
            request.respond(&drivers, bearer.token().to_owned()).await
        }
        VersionedCsiRequest::V0_3(request) => {
            request.respond(&drivers, bearer.token().to_owned()).await
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
/// defined in a versioned WIT world and the `CsiForSkills` trait.
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
}
