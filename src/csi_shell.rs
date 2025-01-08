use axum::{extract::State, http::StatusCode, Json};
use axum_extra::{
    headers::{authorization::Bearer, Authorization},
    TypedHeader,
};
use semver::VersionReq;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    csi::{ChunkRequest, Csi},
    inference::{ChatRequest, CompletionParams, CompletionRequest},
    language_selection::SelectLanguageRequest,
    search::{DocumentMetadataRequest, SearchRequest},
    shell::AppState,
    skills::SupportedVersion,
};

#[allow(clippy::too_many_lines)]
pub async fn http_csi_handle<C>(
    State(app_state): State<AppState<C>>,
    bearer: TypedHeader<Authorization<Bearer>>,
    Json(args): Json<VersionedCsiRequest>,
) -> (StatusCode, Json<Value>)
where
    C: Csi + Clone + Sync,
{
    let drivers = app_state.csi_drivers;
    let result = match args {
        VersionedCsiRequest::V0_2(request) => match request {
            V0_2CsiRequest::Complete(completion_request) => drivers
                .complete_text(bearer.token().to_owned(), completion_request.into())
                .await
                .map(|r| json!(r)),
            V0_2CsiRequest::Chunk(chunk_request) => drivers
                .chunk(bearer.token().to_owned(), chunk_request)
                .await
                .map(|r| json!(r)),
            V0_2CsiRequest::SelectLanguage(select_language_request) => drivers
                .select_language(select_language_request)
                .await
                .map(|r| json!(r)),
            V0_2CsiRequest::CompleteAll(complete_all_request) => drivers
                .complete_all(
                    bearer.token().to_owned(),
                    complete_all_request
                        .requests
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                )
                .await
                .map(|v| json!(v)),
            V0_2CsiRequest::Search(search_request) => drivers
                .search(bearer.token().to_owned(), search_request)
                .await
                .map(|v| json!(v)),
            V0_2CsiRequest::Chat(chat_request) => drivers
                .chat(bearer.token().to_owned(), chat_request)
                .await
                .map(|v| json!(v)),
            V0_2CsiRequest::DocumentMetadata(document_metadata_request) => drivers
                .document_metadata(
                    bearer.token().to_owned(),
                    document_metadata_request.document_path,
                )
                .await
                .map(|r| json!(r)),
            V0_2CsiRequest::Unknown { function } => {
                let msg = format!("The CSI function {} is not supported by this Kernel installation yet. Try updating your Kernel version or downgrading your SDK.", function.as_deref().unwrap_or("specified"));
                return (VALIDATION_ERROR_STATUS_CODE, Json(json!(msg)));
            }
        },
        VersionedCsiRequest::V0_3(request) => match request {
            V0_3CsiRequest::Complete(completion_request) => drivers
                .complete_text(bearer.token().to_owned(), completion_request.into())
                .await
                .map(|r| json!(r)),
            V0_3CsiRequest::Chunk(chunk_request) => drivers
                .chunk(bearer.token().to_owned(), chunk_request)
                .await
                .map(|r| json!(r)),
            V0_3CsiRequest::SelectLanguage(select_language_request) => drivers
                .select_language(select_language_request)
                .await
                .map(|r| json!(r)),
            V0_3CsiRequest::CompleteAll(complete_all_request) => drivers
                .complete_all(
                    bearer.token().to_owned(),
                    complete_all_request
                        .requests
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                )
                .await
                .map(|v| json!(v)),
            V0_3CsiRequest::Search(search_request) => drivers
                .search(bearer.token().to_owned(), search_request)
                .await
                .map(|v| json!(v)),
            V0_3CsiRequest::Chat(chat_request) => drivers
                .chat(bearer.token().to_owned(), chat_request)
                .await
                .map(|v| json!(v)),
            V0_3CsiRequest::DocumentMetadata(document_metadata_request) => drivers
                .document_metadata(
                    bearer.token().to_owned(),
                    document_metadata_request.document_path,
                )
                .await
                .map(|r| json!(r)),
            V0_3CsiRequest::Unknown { function } => {
                let msg = format!("The CSI function {} is not supported by this Kernel installation yet. Try updating your Kernel version or downgrading your SDK.", function.as_deref().unwrap_or("specified"));
                return (VALIDATION_ERROR_STATUS_CODE, Json(json!(msg)));
            }
        },
        VersionedCsiRequest::Unknown { version } => {
            let error = match version.map(|v| VersionReq::parse(&v)) {
                Some(Ok(req)) if req.comparators.len() == 1 => {
                    let max_supported_version = SupportedVersion::latest_supported_version();
                    let comp = req.comparators.first().unwrap();
                    // Only applies to unknown versions. If we parse `1.x.x` as `1` then we are only doing a major version check and minor version only applies `0.x`
                    if comp.major > max_supported_version.major
                        || (comp.major == max_supported_version.major
                            && comp.minor.is_some_and(|m| m > max_supported_version.minor))
                    {
                        "The specified CSI version is not supported by this Kernel installation yet. Try updating your Kernel version or downgrading your SDK."
                    } else {
                        "This CSI version is no longer supported by the Kernel. Try upgrading your SDK."
                    }
                }
                // If the user passes in a random string, the parse will fail and we will end up down here
                Some(Ok(_) | Err(_)) | None => {
                    "A valid CSI version is required. Try upgrading your SDK."
                }
            };
            return (VALIDATION_ERROR_STATUS_CODE, Json(json!(error)));
        }
    };
    match result {
        Ok(result) => (StatusCode::OK, Json(result)),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!(e.to_string())),
        ),
    }
}

/// This represents the versioned interactions with the CSI.
/// The members of this enum provide the glue code to translate between a function
/// defined in a versioned WIT world and the `CsiForSkills` trait.
/// By introducing this abstraction, we can expose a versioned interface of the CSI over http.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "version")]
pub enum VersionedCsiRequest {
    #[serde(rename = "0.3")]
    V0_3(V0_3CsiRequest),
    #[serde(rename = "0.2")]
    V0_2(V0_2CsiRequest),
    #[serde(untagged)]
    Unknown { version: Option<String> },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "function")]
pub enum V0_3CsiRequest {
    Complete(V0_2CompletionRequest),
    Chunk(ChunkRequest),
    SelectLanguage(SelectLanguageRequest),
    CompleteAll(CompleteAllRequest),
    Search(SearchRequest),
    Chat(ChatRequest),
    DocumentMetadata(DocumentMetadataRequest),
    #[serde(untagged)]
    Unknown {
        function: Option<String>,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "function")]
pub enum V0_2CsiRequest {
    Complete(V0_2CompletionRequest),
    Chunk(ChunkRequest),
    SelectLanguage(SelectLanguageRequest),
    CompleteAll(CompleteAllRequest),
    Search(SearchRequest),
    Chat(ChatRequest),
    DocumentMetadata(DocumentMetadataRequest),
    #[serde(untagged)]
    Unknown {
        function: Option<String>,
    },
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CompleteAllRequest {
    pub requests: Vec<V0_2CompletionRequest>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct V0_2CompletionRequest {
    pub prompt: String,
    pub model: String,
    pub params: V0_2CompletionParams,
}

impl From<V0_2CompletionRequest> for CompletionRequest {
    fn from(
        V0_2CompletionRequest {
            prompt,
            model,
            params,
        }: V0_2CompletionRequest,
    ) -> Self {
        Self {
            prompt,
            model,
            params: params.into(),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct V0_2CompletionParams {
    #[serde(default)]
    pub return_special_tokens: bool,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f64>,
    pub top_k: Option<u32>,
    pub top_p: Option<f64>,
    pub stop: Vec<String>,
}

impl From<V0_2CompletionParams> for CompletionParams {
    fn from(
        V0_2CompletionParams {
            return_special_tokens,
            max_tokens,
            temperature,
            top_k,
            top_p,
            stop,
        }: V0_2CompletionParams,
    ) -> Self {
        Self {
            return_special_tokens,
            max_tokens,
            temperature,
            top_k,
            top_p,
            stop,
        }
    }
}

/// We use `BAD_REQUEST` (400) for validation error as it is more commonly used.
/// `UNPROCESSABLE_ENTITY` (422) is an alternative, but it may surprise users as it is less commonly
/// known
const VALIDATION_ERROR_STATUS_CODE: StatusCode = StatusCode::BAD_REQUEST;

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn csi_v_2_request_is_deserialized() {
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
    fn csi_v_3_request_is_deserialized() {
        // Given a request in JSON format
        let request = json!({
            "version": "0.3",
            "function": "complete",
            "prompt": "Hello",
            "model": "pharia-1-llm-7b-control",
            "params": {
                "return_special_tokens": true,
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
}
