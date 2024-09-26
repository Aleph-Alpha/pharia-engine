use axum::{extract::State, http::StatusCode, Json};
use axum_extra::{
    headers::{authorization::Bearer, Authorization},
    TypedHeader,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    csi::{chunking, Csi, CsiDrivers},
    inference, language_selection,
};

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
    use serde_json::json;

    use super::*;

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
}
