use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    chunking::{ChunkParams, ChunkRequest},
    csi::Csi,
    csi_shell::CsiShellError,
    inference::{ChatParams, ChatRequest, CompletionParams, CompletionRequest, Logprobs, Message},
    language_selection::{Language, SelectLanguageRequest},
    search::{DocumentPath, SearchRequest},
};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "function")]
pub enum V0_2CsiRequest {
    Complete(V0_2CompletionRequest),
    Chunk(V0_2ChunkRequest),
    SelectLanguage(V0_2SelectLanguageRequest),
    CompleteAll {
        requests: Vec<V0_2CompletionRequest>,
    },
    Search(SearchRequest),
    Chat(V0_2ChatRequest),
    Documents {
        requests: Vec<DocumentPath>,
    },
    DocumentMetadata(DocumentMetadataRequest),
    #[serde(untagged)]
    Unknown {
        function: Option<String>,
    },
}

impl V0_2CsiRequest {
    pub async fn act<C>(self, drivers: &C, auth: String) -> Result<Value, CsiShellError>
    where
        C: Csi + Sync,
    {
        let result = match self {
            V0_2CsiRequest::Complete(completion_request) => drivers
                .complete(auth, vec![completion_request.into()])
                .await
                .map(|r| json!(r.first().unwrap()))?,
            V0_2CsiRequest::Chunk(chunk_request) => drivers
                .chunk(auth, vec![chunk_request.into()])
                .await
                .map(|r| json!(r.first().unwrap()))?,
            V0_2CsiRequest::SelectLanguage(select_language_request) => drivers
                .select_language(vec![select_language_request.into()])
                .await
                .map(|r| json!(r.first().unwrap()))?,
            V0_2CsiRequest::CompleteAll { requests } => drivers
                .complete(auth, requests.into_iter().map(Into::into).collect())
                .await
                .map(|v| json!(v))?,
            V0_2CsiRequest::Search(search_request) => drivers
                .search(auth, vec![search_request])
                .await
                .map(|v| json!(v.first().unwrap()))?,
            V0_2CsiRequest::Chat(chat_request) => drivers
                .chat(auth, vec![chat_request.into()])
                .await
                .map(|v| json!(v.first().unwrap()))?,
            V0_2CsiRequest::Documents { requests } => {
                drivers.documents(auth, requests).await.map(|r| json!(r))?
            }
            V0_2CsiRequest::DocumentMetadata(document_metadata_request) => drivers
                .document_metadata(auth, vec![document_metadata_request.document_path])
                .await
                .map(|r| json!(r.first().unwrap()))?,
            V0_2CsiRequest::Unknown { function } => {
                return Err(CsiShellError::UnknownFunction(
                    function.unwrap_or_else(|| "specified".to_owned()),
                ));
            }
        };
        Ok(result)
    }
}

/// Retrieve the metadata of a document
#[derive(Debug, Serialize, Deserialize)]
pub struct DocumentMetadataRequest {
    /// Which Document
    pub document_path: DocumentPath,
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
            frequency_penalty: None,
            presence_penalty: None,
            logprobs: Logprobs::No,
        }
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct V0_2ChunkRequest {
    pub text: String,
    pub params: V0_2ChunkParams,
}

impl From<V0_2ChunkRequest> for ChunkRequest {
    fn from(value: V0_2ChunkRequest) -> Self {
        let V0_2ChunkRequest { text, params } = value;
        Self {
            text,
            params: params.into(),
        }
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct V0_2ChunkParams {
    pub model: String,
    pub max_tokens: u32,
}

impl From<V0_2ChunkParams> for ChunkParams {
    fn from(value: V0_2ChunkParams) -> Self {
        let V0_2ChunkParams { model, max_tokens } = value;
        Self { model, max_tokens }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct V0_2SelectLanguageRequest {
    pub text: String,
    pub languages: Vec<V0_2Language>,
}

impl From<V0_2SelectLanguageRequest> for SelectLanguageRequest {
    fn from(value: V0_2SelectLanguageRequest) -> Self {
        let V0_2SelectLanguageRequest { text, languages } = value;
        Self {
            text,
            languages: languages.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum V0_2Language {
    /// English
    Eng,
    /// German
    Deu,
}

impl From<V0_2Language> for Language {
    fn from(value: V0_2Language) -> Self {
        match value {
            V0_2Language::Eng => Self::Eng,
            V0_2Language::Deu => Self::Deu,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct V0_2ChatRequest {
    pub model: String,
    pub messages: Vec<V0_2Message>,
    pub params: V0_2ChatParams,
}

impl From<V0_2ChatRequest> for ChatRequest {
    fn from(value: V0_2ChatRequest) -> Self {
        let V0_2ChatRequest {
            model,
            messages,
            params,
        } = value;
        Self {
            model,
            messages: messages.into_iter().map(Into::into).collect(),
            params: params.into(),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
pub struct V0_2ChatParams {
    pub max_tokens: Option<u32>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
}

impl From<V0_2ChatParams> for ChatParams {
    fn from(value: V0_2ChatParams) -> Self {
        let V0_2ChatParams {
            max_tokens,
            temperature,
            top_p,
        } = value;
        Self {
            max_tokens,
            temperature,
            top_p,
            ..Default::default()
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct V0_2Message {
    pub role: String,
    pub content: String,
}

impl From<V0_2Message> for Message {
    fn from(value: V0_2Message) -> Self {
        let V0_2Message { role, content } = value;
        Self {
            role: role.to_lowercase(),
            content,
        }
    }
}

#[cfg(test)]
pub mod tests {
    use serde_json::json;

    use super::*;
    pub use crate::csi_shell::VersionedCsiRequest;

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
            Ok(VersionedCsiRequest::V0_2(V0_2CsiRequest::Chunk(chunk_request))) if chunk_request.text == "Hello"
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
            Ok(VersionedCsiRequest::V0_2(V0_2CsiRequest::SelectLanguage(select_language_request))) if select_language_request.text == "Hello"
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
            Ok(VersionedCsiRequest::V0_2(V0_2CsiRequest::CompleteAll { requests })) if requests.len() == 2
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
            Ok(VersionedCsiRequest::V0_2(V0_2CsiRequest::Search(search_request))) if search_request.query == "Hello"
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
            Ok(VersionedCsiRequest::V0_2(V0_2CsiRequest::Chat(chat_request))) if chat_request.model == "pharia-1-llm-7b-control"
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
            Ok(VersionedCsiRequest::V0_2(V0_2CsiRequest::Documents { requests })) if requests.len() == 2
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
            Ok(VersionedCsiRequest::V0_2(V0_2CsiRequest::DocumentMetadata(
                _
            )))
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
            Ok(VersionedCsiRequest::V0_2(V0_2CsiRequest::Unknown { function: Some(function) })) if function == "not-implemented"
        ));
    }
}
