use axum::{extract::State, http::StatusCode, Json};
use axum_extra::{
    headers::{authorization::Bearer, Authorization},
    TypedHeader,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    csi::{ChunkRequest, Csi},
    inference::{self, ChatRequest, CompletionRequest},
    language_selection::SelectLanguageRequest,
    search::{DocumentMetadataRequest, SearchRequest},
    shell::AppState,
};

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
                .complete_text(bearer.token().to_owned(), completion_request)
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
                        .map(inference::CompletionRequest::from)
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

/// This represents the versioned interactions with the CSI.
/// The members of this enum provide the glue code to translate between a function
/// defined in a versioned WIT world and the `CsiForSkills` trait.
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
    Search(SearchRequest),
    Chat(ChatRequest),
    DocumentMetadata(DocumentMetadataRequest),
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CompleteAllRequest {
    pub requests: Vec<CompletionRequest>,
}

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
}
