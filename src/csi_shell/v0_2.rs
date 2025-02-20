/// CSI Shell version 0.2
///
/// See [v0_3.rs](v0_3.rs) for a more detailed explanation on the reasoning for introducing
/// serializable/user-facing structs in here and for not serializing our "internal" representations.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{chunking, csi::Csi, csi_shell::CsiShellError, inference, language_selection, search};

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", tag = "function")]
pub enum CsiRequest {
    Chat(ChatRequest),
    Chunk(ChunkRequest),
    Complete(CompletionRequest),
    CompleteAll { requests: Vec<CompletionRequest> },
    Documents { requests: Vec<DocumentPath> },
    DocumentMetadata { document_path: DocumentPath },
    Search(SearchRequest),
    SelectLanguage(SelectLanguageRequest),
}

impl CsiRequest {
    pub async fn act<C>(self, drivers: &C, auth: String) -> Result<Value, CsiShellError>
    where
        C: Csi + Sync,
    {
        let response = match self {
            CsiRequest::Chat(chat_request) => drivers
                .chat(auth, vec![chat_request.into()])
                .await
                .map(|mut r| CsiResponse::Chat(r.remove(0).into())),
            CsiRequest::Chunk(chunk_request) => drivers
                .chunk(auth, vec![chunk_request.into()])
                .await
                .map(|mut r| CsiResponse::Chunk(r.remove(0))),
            CsiRequest::Complete(completion_request) => drivers
                .complete(auth, vec![completion_request.into()])
                .await
                .map(|mut r| CsiResponse::Complete(r.remove(0).into())),
            CsiRequest::CompleteAll { requests } => drivers
                .complete(auth, requests.into_iter().map(Into::into).collect())
                .await
                .map(|r| CsiResponse::CompleteAll(r.into_iter().map(Into::into).collect())),
            CsiRequest::Documents { requests } => drivers
                .documents(auth, requests.into_iter().map(Into::into).collect())
                .await
                .map(|r| CsiResponse::Documents(r.into_iter().map(Into::into).collect())),
            CsiRequest::DocumentMetadata { document_path } => drivers
                .document_metadata(auth, vec![document_path.into()])
                .await
                .map(|mut r| CsiResponse::DocumentMetadata(r.remove(0))),
            CsiRequest::Search(search_request) => drivers
                .search(auth, vec![search_request.into()])
                .await
                .map(|mut r| {
                    CsiResponse::Search(r.remove(0).into_iter().map(Into::into).collect())
                }),
            CsiRequest::SelectLanguage(select_language_request) => drivers
                .select_language(vec![select_language_request.into()])
                .await
                .map(|mut r| {
                    r.remove(0)
                        .map(TryInto::try_into)
                        .transpose()
                        .map(CsiResponse::Language)
                })?,
        }?;
        Ok(json!(response))
    }
}

#[derive(Serialize)]
#[serde(untagged)]
enum CsiResponse {
    Chat(ChatResponse),
    Chunk(Vec<String>),
    Complete(Completion),
    CompleteAll(Vec<Completion>),
    Documents(Vec<Document>),
    DocumentMetadata(Option<Value>),
    Language(Option<Language>),
    Search(Vec<SearchResult>),
}

#[derive(Deserialize, Serialize)]
pub struct DocumentPath {
    pub namespace: String,
    pub collection: String,
    pub name: String,
}

impl From<DocumentPath> for search::DocumentPath {
    fn from(value: DocumentPath) -> Self {
        let DocumentPath {
            namespace,
            collection,
            name,
        } = value;
        Self {
            namespace,
            collection,
            name,
        }
    }
}

impl From<search::DocumentPath> for DocumentPath {
    fn from(value: search::DocumentPath) -> Self {
        let search::DocumentPath {
            namespace,
            collection,
            name,
        } = value;
        Self {
            namespace,
            collection,
            name,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case", tag = "modality")]
enum Modality {
    Text { text: String },
    Image,
}

impl From<search::Modality> for Modality {
    fn from(value: search::Modality) -> Self {
        match value {
            search::Modality::Text { text } => Modality::Text { text },
            search::Modality::Image { bytes: _ } => Modality::Image,
        }
    }
}
#[derive(Serialize)]
struct Document {
    path: DocumentPath,
    contents: Vec<Modality>,
    metadata: Option<Value>,
}

impl From<search::Document> for Document {
    fn from(value: search::Document) -> Self {
        let search::Document {
            path,
            contents,
            metadata,
        } = value;
        Self {
            path: path.into(),
            contents: contents.into_iter().map(Into::into).collect(),
            metadata,
        }
    }
}
#[derive(Serialize)]
struct SearchResult {
    document_path: DocumentPath,
    content: String,
    score: f64,
}

impl From<search::SearchResult> for SearchResult {
    fn from(value: search::SearchResult) -> Self {
        let search::SearchResult {
            document_path,
            content,
            score,
            start: _,
            end: _,
        } = value;
        Self {
            document_path: document_path.into(),
            content,
            score,
        }
    }
}

#[derive(Deserialize)]
pub struct CompletionRequest {
    pub prompt: String,
    pub model: String,
    pub params: CompletionParams,
}

impl From<CompletionRequest> for inference::CompletionRequest {
    fn from(
        CompletionRequest {
            prompt,
            model,
            params,
        }: CompletionRequest,
    ) -> Self {
        Self {
            prompt,
            model,
            params: params.into(),
        }
    }
}

#[derive(Deserialize)]
pub struct CompletionParams {
    #[serde(default)]
    pub return_special_tokens: bool,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f64>,
    pub top_k: Option<u32>,
    pub top_p: Option<f64>,
    pub stop: Vec<String>,
}

impl From<CompletionParams> for inference::CompletionParams {
    fn from(
        CompletionParams {
            return_special_tokens,
            max_tokens,
            temperature,
            top_k,
            top_p,
            stop,
        }: CompletionParams,
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
            logprobs: inference::Logprobs::No,
        }
    }
}

#[derive(Serialize)]
struct Completion {
    pub text: String,
    pub finish_reason: FinishReason,
}

impl From<inference::Completion> for Completion {
    fn from(value: inference::Completion) -> Self {
        let inference::Completion {
            text,
            finish_reason,
            logprobs: _,
            usage: _,
        } = value;
        Self {
            text,
            finish_reason: finish_reason.into(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum FinishReason {
    Stop,
    Length,
    ContentFilter,
}

impl From<inference::FinishReason> for FinishReason {
    fn from(value: inference::FinishReason) -> Self {
        match value {
            inference::FinishReason::Stop => Self::Stop,
            inference::FinishReason::Length => Self::Length,
            inference::FinishReason::ContentFilter => Self::ContentFilter,
        }
    }
}

#[derive(Deserialize)]
pub struct ChunkRequest {
    pub text: String,
    pub params: ChunkParams,
}

impl From<ChunkRequest> for chunking::ChunkRequest {
    fn from(value: ChunkRequest) -> Self {
        let ChunkRequest { text, params } = value;
        Self {
            text,
            params: params.into(),
        }
    }
}

#[derive(Deserialize)]
pub struct ChunkParams {
    pub model: String,
    pub max_tokens: u32,
}

impl From<ChunkParams> for chunking::ChunkParams {
    fn from(value: ChunkParams) -> Self {
        let ChunkParams { model, max_tokens } = value;
        Self {
            model,
            max_tokens,
            overlap: 0,
        }
    }
}

#[derive(Deserialize)]
pub struct SelectLanguageRequest {
    pub text: String,
    pub languages: Vec<Language>,
}

impl From<SelectLanguageRequest> for language_selection::SelectLanguageRequest {
    fn from(value: SelectLanguageRequest) -> Self {
        let SelectLanguageRequest { text, languages } = value;
        Self {
            text,
            languages: languages.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    /// English
    Eng,
    /// German
    Deu,
}

impl From<Language> for language_selection::Language {
    fn from(value: Language) -> Self {
        match value {
            Language::Eng => Self::new("eng".to_owned()),
            Language::Deu => Self::new("deu".to_owned()),
        }
    }
}

impl TryFrom<language_selection::Language> for Language {
    type Error = anyhow::Error;

    fn try_from(value: language_selection::Language) -> Result<Self, Self::Error> {
        let language = match value {
            language_selection::Language(s) if s == "eng" => Self::Eng,
            language_selection::Language(s) if s == "deu" => Self::Deu,
            _ => {
                let err = anyhow::anyhow!("Unsupported language: {:?}", value);
                tracing::error!("{}", err);
                return Err(err);
            }
        };
        Ok(language)
    }
}

#[derive(Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub params: ChatParams,
}

impl From<ChatRequest> for inference::ChatRequest {
    fn from(value: ChatRequest) -> Self {
        let ChatRequest {
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

#[derive(Deserialize)]
pub struct IndexPath {
    pub namespace: String,
    pub collection: String,
    pub index: String,
}

impl From<IndexPath> for search::IndexPath {
    fn from(value: IndexPath) -> Self {
        let IndexPath {
            namespace,
            collection,
            index,
        } = value;
        Self {
            namespace,
            collection,
            index,
        }
    }
}

#[derive(Deserialize)]
pub struct SearchRequest {
    pub query: String,
    pub index_path: IndexPath,
    pub max_results: u32,
    pub min_score: Option<f64>,
}

impl From<SearchRequest> for search::SearchRequest {
    fn from(value: SearchRequest) -> Self {
        let SearchRequest {
            query,
            index_path,
            max_results,
            min_score,
        } = value;
        Self {
            query,
            index_path: index_path.into(),
            max_results,
            min_score,
            filters: Vec::new(),
        }
    }
}

#[derive(Deserialize)]
pub struct ChatParams {
    pub max_tokens: Option<u32>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
}

impl From<ChatParams> for inference::ChatParams {
    fn from(value: ChatParams) -> Self {
        let ChatParams {
            max_tokens,
            temperature,
            top_p,
        } = value;
        Self {
            max_tokens,
            temperature,
            top_p,
            frequency_penalty: None,
            presence_penalty: None,
            logprobs: inference::Logprobs::No,
        }
    }
}

#[derive(Deserialize, Serialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

impl From<Message> for inference::Message {
    fn from(value: Message) -> Self {
        let Message { role, content } = value;
        Self {
            role: role.to_lowercase(),
            content,
        }
    }
}

impl From<inference::Message> for Message {
    fn from(value: inference::Message) -> Self {
        let inference::Message { role, content } = value;
        Self { role, content }
    }
}

#[derive(Serialize)]
struct ChatResponse {
    message: Message,
    finish_reason: FinishReason,
}

impl From<inference::ChatResponse> for ChatResponse {
    fn from(value: inference::ChatResponse) -> Self {
        let inference::ChatResponse {
            message,
            finish_reason,
            logprobs: _,
            usage: _,
        } = value;
        Self {
            message: message.into(),
            finish_reason: finish_reason.into(),
        }
    }
}

#[cfg(test)]
pub mod tests {
    use serde_json::json;

    use super::*;
    pub use crate::csi_shell::VersionedCsiRequest;
    use crate::{inference, language_selection, search};

    #[test]
    fn document_metadata_response() {
        let response = CsiResponse::DocumentMetadata(Some(json!({
            "url": "http://example.de"
        })));

        let serialized = serde_json::to_value(response).unwrap();

        assert_eq!(
            serialized,
            json!({
                "url": "http://example.de"
            })
        );
    }

    #[test]
    fn documents_response() {
        let response = CsiResponse::Documents(vec![search::Document::dummy().into()]);

        let serialized = serde_json::to_value(response).unwrap();

        assert_eq!(
            serialized,
            json!([{
                "contents": [
                    {
                        "modality": "text",
                        "text": "Hello Homer"
                    }
                ],
                "metadata": {
                    "url": "http://example.de"
                },
                "path": {
                    "namespace": "Kernel",
                    "collection": "test",
                    "name": "kernel-docs"
                },
            }])
        );
    }
    #[test]
    fn chat_response() {
        let response = CsiResponse::Chat(
            inference::ChatResponse {
                message: inference::Message {
                    role: "user".to_string(),
                    content: "Hello".to_string(),
                },
                finish_reason: inference::FinishReason::Stop,
                logprobs: vec![],
                usage: inference::TokenUsage {
                    prompt: 0,
                    completion: 0,
                },
            }
            .into(),
        );

        let serialized = serde_json::to_value(response).unwrap();

        assert_eq!(
            serialized,
            json!({
                "message": {
                    "role": "user",
                    "content": "Hello"
                },
                "finish_reason": "stop",
            })
        );
    }
    #[test]
    fn search_response() {
        let response = CsiResponse::Search(vec![search::SearchResult {
            document_path: search::DocumentPath::new("Kernel", "test", "kernel-docs"),
            content: "Hello".to_string(),
            score: 0.5,
            start: search::TextCursor {
                item: 0,
                position: 0,
            },
            end: search::TextCursor {
                item: 0,
                position: 5,
            },
        }
        .into()]);

        let serialized = serde_json::to_value(response).unwrap();

        assert_eq!(
            serialized,
            json!([{
                "document_path": {
                    "namespace": "Kernel",
                    "collection": "test",
                    "name": "kernel-docs"
                },
                "content": "Hello",
                "score": 0.5,
            }])
        );
    }

    #[test]
    fn language_response() {
        let response = CsiResponse::Language(Some(
            language_selection::Language::new("eng".to_owned())
                .try_into()
                .unwrap(),
        ));

        let serialized = serde_json::to_value(response).unwrap();

        assert_eq!(serialized, json!("eng"));
    }

    #[test]
    fn complete_response() {
        let response = CsiResponse::Complete(
            inference::Completion {
                text: "Hello".to_string(),
                finish_reason: inference::FinishReason::Stop,
                logprobs: vec![],
                usage: inference::TokenUsage {
                    prompt: 0,
                    completion: 0,
                },
            }
            .into(),
        );

        let serialized = serde_json::to_value(response).unwrap();

        assert_eq!(
            serialized,
            json!({
                "text": "Hello",
                "finish_reason": "stop"
            })
        );
    }

    #[test]
    fn complete_all_response() {
        let response = CsiResponse::CompleteAll(vec![inference::Completion {
            text: "Hello".to_string(),
            finish_reason: inference::FinishReason::Stop,
            logprobs: vec![],
            usage: inference::TokenUsage {
                prompt: 0,
                completion: 0,
            },
        }
        .into()]);

        let serialized = serde_json::to_value(response).unwrap();

        assert_eq!(
            serialized,
            json!([
                {
                    "text": "Hello",
                    "finish_reason": "stop"
                }
            ])
        );
    }

    #[test]
    fn chunk_response() {
        let response = CsiResponse::Chunk(vec!["Hello".to_string()]);

        let serialized = serde_json::to_value(response).unwrap();

        assert_eq!(serialized, json!(["Hello"]));
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
            Ok(VersionedCsiRequest::V0_2(CsiRequest::Chunk(chunk_request))) if chunk_request.text == "Hello"
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
            Ok(VersionedCsiRequest::V0_2(CsiRequest::SelectLanguage(select_language_request))) if select_language_request.text == "Hello"
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
            Ok(VersionedCsiRequest::V0_2(CsiRequest::CompleteAll { requests })) if requests.len() == 2
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
            Ok(VersionedCsiRequest::V0_2(CsiRequest::Search(search_request))) if search_request.query == "Hello"
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
            Ok(VersionedCsiRequest::V0_2(CsiRequest::Chat(chat_request))) if chat_request.model == "pharia-1-llm-7b-control"
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
            Ok(VersionedCsiRequest::V0_2(CsiRequest::Documents { requests })) if requests.len() == 2
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
            Ok(VersionedCsiRequest::V0_2(CsiRequest::DocumentMetadata { document_path })) if document_path.namespace == "Kernel" && document_path.collection == "test" && document_path.name == "kernel/docs"
        ));
    }
}
