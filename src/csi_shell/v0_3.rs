//! CSI Shell version 0.3
//!
//! This module introduces serializable/user-facing structs which are very similar to our "internal" representations.
//! While this module may appear to contain a lot of boilerplate code, there is a good reason to not serialize our "internal" representations:
//! It allows us to keep our external interface stable while updating our "internal" representations.
//! Imagine we introduce a new version (0.4) with breaking changes in the api (e.g. a new field in `CompletionParams`).
//! If we simply serialized the internal representation, we would break clients going against the 0.3 version of the CSI shell.
use derive_more::{From, Into};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    authorization::Authentication, chunking, csi::RawCsi, csi_shell::CsiShellError, inference,
    language_selection, logging::TracingContext, search,
};

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", tag = "function")]
pub enum CsiRequest {
    Explain {
        requests: Vec<ExplainRequest>,
    },
    Chat {
        requests: Vec<ChatRequest>,
    },
    Chunk {
        requests: Vec<ChunkRequest>,
    },
    ChunkWithOffsets {
        requests: Vec<ChunkWithOffsetRequest>,
    },
    Complete {
        requests: Vec<CompletionRequest>,
    },
    Documents {
        requests: Vec<DocumentPath>,
    },
    DocumentMetadata {
        requests: Vec<DocumentPath>,
    },
    Search {
        requests: Vec<SearchRequest>,
    },
    SelectLanguage {
        requests: Vec<SelectLanguageRequest>,
    },
    #[serde(untagged)]
    Unknown {
        function: Option<String>,
    },
}

impl CsiRequest {
    #[allow(clippy::too_many_lines)]
    pub async fn respond<C>(
        self,
        drivers: &C,
        auth: Authentication,
        tracing_context: TracingContext,
    ) -> Result<Value, CsiShellError>
    where
        C: RawCsi + Sync,
    {
        let response = match self {
            CsiRequest::Explain { requests } => {
                let explanations = drivers
                    .explain(
                        auth,
                        tracing_context,
                        requests.into_iter().map(Into::into).collect(),
                    )
                    .await
                    .map_err(CsiShellError::from_csi_error)?;
                CsiResponse::Explain(
                    explanations
                        .into_iter()
                        .map(|r| r.into_iter().map(Into::into).collect())
                        .collect(),
                )
            }
            CsiRequest::Chat { requests } => drivers
                .chat(
                    auth,
                    tracing_context,
                    requests.into_iter().map(Into::into).collect(),
                )
                .await
                .map(|v| CsiResponse::Chat(v.into_iter().map(Into::into).collect()))?,
            CsiRequest::Chunk { requests } => drivers
                .chunk(
                    auth,
                    tracing_context,
                    requests.into_iter().map(Into::into).collect(),
                )
                .await
                .map(|c| {
                    CsiResponse::Chunk(
                        c.into_iter()
                            .map(|c| c.into_iter().map(|c| c.text).collect())
                            .collect(),
                    )
                })?,
            CsiRequest::ChunkWithOffsets { requests } => drivers
                .chunk(
                    auth,
                    tracing_context,
                    requests.into_iter().map(Into::into).collect(),
                )
                .await
                .map(|c| {
                    CsiResponse::ChunkWithOffsets(
                        c.into_iter()
                            .map(|c| c.into_iter().map(Into::into).collect())
                            .collect(),
                    )
                })?,
            CsiRequest::Complete { requests } => drivers
                .complete(
                    auth,
                    tracing_context,
                    requests.into_iter().map(Into::into).collect(),
                )
                .await
                .map(|v| CsiResponse::Complete(v.into_iter().map(Into::into).collect()))?,
            CsiRequest::Documents { requests } => drivers
                .documents(
                    auth,
                    tracing_context,
                    requests.into_iter().map(Into::into).collect(),
                )
                .await
                .map(|r| CsiResponse::Documents(r.into_iter().map(Into::into).collect()))?,
            CsiRequest::DocumentMetadata { requests } => drivers
                .document_metadata(
                    auth,
                    tracing_context,
                    requests.into_iter().map(Into::into).collect(),
                )
                .await
                .map(CsiResponse::DocumentMetadata)?,
            CsiRequest::Search { requests } => drivers
                .search(
                    auth,
                    tracing_context,
                    requests.into_iter().map(Into::into).collect(),
                )
                .await
                .map(|v| {
                    CsiResponse::SearchResult(
                        v.into_iter()
                            .map(|r| r.into_iter().map(Into::into).collect())
                            .collect(),
                    )
                })?,
            CsiRequest::SelectLanguage { requests } => drivers
                .select_language(
                    requests.into_iter().map(Into::into).collect(),
                    tracing_context,
                )
                .await
                .map(|r| {
                    CsiResponse::SelectLanguage(r.into_iter().map(|m| m.map(Into::into)).collect())
                })?,
            CsiRequest::Unknown { function } => {
                return Err(CsiShellError::UnknownFunction(
                    function.unwrap_or_else(|| "specified".to_owned()),
                ));
            }
        };
        Ok(json!(response))
    }
}

#[derive(Serialize)]
#[serde(untagged)]
enum CsiResponse {
    Explain(Vec<Vec<TextScore>>),
    Chat(Vec<ChatResponse>),
    Complete(Vec<Completion>),
    Chunk(Vec<Vec<String>>),
    ChunkWithOffsets(Vec<Vec<ChunkWithOffset>>),
    Documents(Vec<Document>),
    DocumentMetadata(Vec<Option<Value>>),
    SearchResult(Vec<Vec<SearchResult>>),
    SelectLanguage(Vec<Option<Language>>),
}

#[derive(Serialize)]
struct TextScore {
    start: u32,
    length: u32,
    score: f64,
}

impl From<inference::TextScore> for TextScore {
    fn from(value: inference::TextScore) -> Self {
        let inference::TextScore {
            start,
            length,
            score,
        } = value;
        TextScore {
            start,
            length,
            score,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Granularity {
    Auto,
    Word,
    Sentence,
    Paragraph,
}

impl From<Granularity> for inference::Granularity {
    fn from(value: Granularity) -> Self {
        match value {
            Granularity::Auto => inference::Granularity::Auto,
            Granularity::Word => inference::Granularity::Word,
            Granularity::Sentence => inference::Granularity::Sentence,
            Granularity::Paragraph => inference::Granularity::Paragraph,
        }
    }
}

#[derive(Deserialize)]
pub struct ExplainRequest {
    pub prompt: String,
    pub target: String,
    pub model: String,
    pub granularity: Granularity,
}

impl From<ExplainRequest> for inference::ExplanationRequest {
    fn from(value: ExplainRequest) -> Self {
        let ExplainRequest {
            prompt,
            target,
            model,
            granularity,
        } = value;
        inference::ExplanationRequest {
            prompt,
            target,
            model,
            granularity: granularity.into(),
        }
    }
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
        search::DocumentPath {
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
        DocumentPath {
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
        Document {
            path: path.into(),
            contents: contents.into_iter().map(Into::into).collect(),
            metadata,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

impl From<Message> for inference::Message {
    fn from(value: Message) -> Self {
        let Message { role, content } = value;
        inference::Message { role, content }
    }
}

impl From<inference::Message> for Message {
    fn from(value: inference::Message) -> Self {
        let inference::Message { role, content } = value;
        Message { role, content }
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
            inference::FinishReason::Stop => FinishReason::Stop,
            inference::FinishReason::Length => FinishReason::Length,
            inference::FinishReason::ContentFilter => FinishReason::ContentFilter,
        }
    }
}

#[derive(Serialize)]
struct TokenUsage {
    prompt: u32,
    completion: u32,
}

impl From<inference::TokenUsage> for TokenUsage {
    fn from(value: inference::TokenUsage) -> Self {
        let inference::TokenUsage { prompt, completion } = value;
        TokenUsage { prompt, completion }
    }
}

#[derive(Serialize)]
struct Logprob {
    token: Vec<u8>,
    logprob: f64,
}

impl From<inference::Logprob> for Logprob {
    fn from(value: inference::Logprob) -> Self {
        let inference::Logprob { token, logprob } = value;
        Logprob { token, logprob }
    }
}

#[derive(Serialize)]
struct Distribution {
    sampled: Logprob,
    top: Vec<Logprob>,
}

impl From<inference::Distribution> for Distribution {
    fn from(value: inference::Distribution) -> Self {
        let inference::Distribution { sampled, top } = value;
        Distribution {
            sampled: sampled.into(),
            top: top.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Serialize)]
struct ChatResponse {
    message: Message,
    finish_reason: FinishReason,
    logprobs: Vec<Distribution>,
    usage: TokenUsage,
}

impl From<inference::ChatResponse> for ChatResponse {
    fn from(value: inference::ChatResponse) -> Self {
        let inference::ChatResponse {
            message,
            finish_reason,
            logprobs,
            usage,
        } = value;
        ChatResponse {
            message: message.into(),
            finish_reason: finish_reason.into(),
            logprobs: logprobs.into_iter().map(Into::into).collect(),
            usage: usage.into(),
        }
    }
}

#[derive(Deserialize)]
pub struct ChatParams {
    pub max_tokens: Option<u32>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub frequency_penalty: Option<f64>,
    pub presence_penalty: Option<f64>,
    pub logprobs: Logprobs,
}

impl From<ChatParams> for inference::ChatParams {
    fn from(value: ChatParams) -> Self {
        let ChatParams {
            max_tokens,
            temperature,
            top_p,
            frequency_penalty,
            presence_penalty,
            logprobs,
        } = value;
        inference::ChatParams {
            max_tokens,
            temperature,
            top_p,
            frequency_penalty,
            presence_penalty,
            logprobs: logprobs.into(),
        }
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
        inference::ChatRequest {
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
        search::IndexPath {
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
    pub filters: Vec<Filter>,
}

impl From<SearchRequest> for search::SearchRequest {
    fn from(value: SearchRequest) -> Self {
        let SearchRequest {
            query,
            index_path,
            max_results,
            min_score,
            filters,
        } = value;
        search::SearchRequest {
            query,
            index_path: index_path.into(),
            max_results,
            min_score,
            filters: filters.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Serialize)]
struct TextCursor {
    pub item: u32,
    pub position: u32,
}

impl From<search::TextCursor> for TextCursor {
    fn from(value: search::TextCursor) -> Self {
        let search::TextCursor { item, position } = value;
        TextCursor { item, position }
    }
}

#[derive(Serialize)]
struct SearchResult {
    pub document_path: DocumentPath,
    pub content: String,
    pub score: f64,
    pub start: TextCursor,
    pub end: TextCursor,
}

impl From<search::SearchResult> for SearchResult {
    fn from(value: search::SearchResult) -> Self {
        let search::SearchResult {
            document_path,
            content,
            score,
            start,
            end,
        } = value;
        SearchResult {
            document_path: document_path.into(),
            content,
            score,
            start: start.into(),
            end: end.into(),
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Filter {
    Without(Vec<FilterCondition>),
    WithOneOf(Vec<FilterCondition>),
    With(Vec<FilterCondition>),
}

impl From<Filter> for search::Filter {
    fn from(value: Filter) -> Self {
        match value {
            Filter::Without(items) => {
                search::Filter::Without(items.into_iter().map(Into::into).collect())
            }
            Filter::WithOneOf(items) => {
                search::Filter::WithOneOf(items.into_iter().map(Into::into).collect())
            }
            Filter::With(items) => {
                search::Filter::With(items.into_iter().map(Into::into).collect())
            }
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
/// This representation diverges from the wit world 0.3 schema as it
/// has one more layer of nesting with the metadata filter and calls
/// the `with-all` variant simply `with`. While we want to keep the
/// http and wit representations as close as possible, this divergence
/// happened as a mistake and we do not update it to not break clients.
pub enum FilterCondition {
    Metadata(MetadataFilter),
}

impl From<FilterCondition> for search::FilterCondition {
    fn from(value: FilterCondition) -> Self {
        match value {
            FilterCondition::Metadata(metadata_filter) => {
                search::FilterCondition::Metadata(metadata_filter.into())
            }
        }
    }
}

#[derive(Deserialize, Debug)]
pub struct MetadataFilter {
    pub field: String,
    #[serde(flatten)]
    pub condition: MetadataFilterCondition,
}

impl From<MetadataFilter> for search::MetadataFilter {
    fn from(value: MetadataFilter) -> Self {
        let MetadataFilter { field, condition } = value;
        Self {
            field,
            condition: condition.into(),
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum MetadataFilterCondition {
    GreaterThan(f64),
    GreaterThanOrEqualTo(f64),
    LessThan(f64),
    LessThanOrEqualTo(f64),
    After(String),
    AtOrAfter(String),
    Before(String),
    AtOrBefore(String),
    EqualTo(MetadataFieldValue),
    IsNull(serde_bool::True),
}

impl From<MetadataFilterCondition> for search::MetadataFilterCondition {
    fn from(value: MetadataFilterCondition) -> Self {
        match value {
            MetadataFilterCondition::GreaterThan(v) => {
                search::MetadataFilterCondition::GreaterThan(v)
            }
            MetadataFilterCondition::GreaterThanOrEqualTo(v) => {
                search::MetadataFilterCondition::GreaterThanOrEqualTo(v)
            }
            MetadataFilterCondition::LessThan(v) => search::MetadataFilterCondition::LessThan(v),
            MetadataFilterCondition::LessThanOrEqualTo(v) => {
                search::MetadataFilterCondition::LessThanOrEqualTo(v)
            }
            MetadataFilterCondition::After(v) => search::MetadataFilterCondition::After(v),
            MetadataFilterCondition::AtOrAfter(v) => search::MetadataFilterCondition::AtOrAfter(v),
            MetadataFilterCondition::Before(v) => search::MetadataFilterCondition::Before(v),
            MetadataFilterCondition::AtOrBefore(v) => {
                search::MetadataFilterCondition::AtOrBefore(v)
            }
            MetadataFilterCondition::EqualTo(v) => {
                search::MetadataFilterCondition::EqualTo(v.into())
            }
            MetadataFilterCondition::IsNull(v) => search::MetadataFilterCondition::IsNull(v),
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(untagged)]
pub enum MetadataFieldValue {
    String(String),
    Integer(i64),
    Boolean(bool),
}

impl From<MetadataFieldValue> for search::MetadataFieldValue {
    fn from(value: MetadataFieldValue) -> Self {
        match value {
            MetadataFieldValue::String(v) => search::MetadataFieldValue::String(v),
            MetadataFieldValue::Integer(v) => search::MetadataFieldValue::Integer(v),
            MetadataFieldValue::Boolean(v) => search::MetadataFieldValue::Boolean(v),
        }
    }
}

#[derive(Deserialize)]
pub struct ChunkParams {
    pub model: String,
    pub max_tokens: u32,
    pub overlap: u32,
}

impl From<ChunkParams> for chunking::ChunkParams {
    fn from(value: ChunkParams) -> Self {
        let ChunkParams {
            model,
            max_tokens,
            overlap,
        } = value;
        chunking::ChunkParams {
            model,
            max_tokens,
            overlap,
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
        chunking::ChunkRequest {
            text,
            params: params.into(),
            character_offsets: false,
        }
    }
}

#[derive(Deserialize)]
pub struct ChunkWithOffsetRequest {
    pub text: String,
    pub params: ChunkParams,
    pub character_offsets: bool,
}

impl From<ChunkWithOffsetRequest> for chunking::ChunkRequest {
    fn from(value: ChunkWithOffsetRequest) -> Self {
        let ChunkWithOffsetRequest {
            text,
            params,
            character_offsets,
        } = value;
        chunking::ChunkRequest {
            text,
            params: params.into(),
            character_offsets,
        }
    }
}

#[derive(Serialize)]
struct ChunkWithOffset {
    text: String,
    byte_offset: u64,
    character_offset: Option<u64>,
}

impl From<chunking::Chunk> for ChunkWithOffset {
    fn from(value: chunking::Chunk) -> Self {
        let chunking::Chunk {
            text,
            byte_offset,
            character_offset,
        } = value;
        ChunkWithOffset {
            text,
            byte_offset,
            character_offset,
        }
    }
}

#[derive(Deserialize, Serialize, From, Into)]
#[serde(transparent)]
pub struct Language(String);

impl From<Language> for language_selection::Language {
    fn from(value: Language) -> Self {
        Self::new(value.0)
    }
}

impl From<language_selection::Language> for Language {
    fn from(value: language_selection::Language) -> Self {
        Self(value.into())
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
        language_selection::SelectLanguageRequest {
            text,
            languages: languages.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Logprobs {
    No,
    Sampled,
    Top(u8),
}

impl From<Logprobs> for inference::Logprobs {
    fn from(value: Logprobs) -> Self {
        match value {
            Logprobs::No => inference::Logprobs::No,
            Logprobs::Sampled => inference::Logprobs::Sampled,
            Logprobs::Top(n) => inference::Logprobs::Top(n),
        }
    }
}

#[derive(Deserialize)]
pub struct CompletionParams {
    pub return_special_tokens: bool,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f64>,
    pub top_k: Option<u32>,
    pub top_p: Option<f64>,
    pub stop: Vec<String>,
    pub frequency_penalty: Option<f64>,
    pub presence_penalty: Option<f64>,
    pub logprobs: Logprobs,
}

impl From<CompletionParams> for inference::CompletionParams {
    fn from(value: CompletionParams) -> Self {
        let CompletionParams {
            return_special_tokens,
            max_tokens,
            temperature,
            top_k,
            top_p,
            frequency_penalty,
            presence_penalty,
            logprobs,
            stop,
        } = value;
        inference::CompletionParams {
            return_special_tokens,
            max_tokens,
            temperature,
            top_k,
            top_p,
            stop,
            frequency_penalty,
            presence_penalty,
            logprobs: logprobs.into(),
            echo: false,
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
    fn from(value: CompletionRequest) -> Self {
        let CompletionRequest {
            prompt,
            model,
            params,
        } = value;
        inference::CompletionRequest {
            prompt,
            model,
            params: params.into(),
        }
    }
}

#[derive(Serialize)]
struct Completion {
    text: String,
    finish_reason: FinishReason,
    logprobs: Vec<Distribution>,
    usage: TokenUsage,
}

impl From<inference::Completion> for Completion {
    fn from(value: inference::Completion) -> Self {
        let inference::Completion {
            text,
            finish_reason,
            logprobs,
            usage,
        } = value;
        Completion {
            text,
            finish_reason: finish_reason.into(),
            logprobs: logprobs.into_iter().map(Into::into).collect(),
            usage: usage.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::csi_shell::VersionedCsiRequest;

    #[test]
    fn explain_request() {
        let request = json!({
            "version": "0.3",
            "function": "explain",
            "requests": [
                {
                    "prompt": "Hello",
                    "target": "World",
                    "model": "pharia-1-llm-7b-control",
                    "granularity": "word"
                }
            ]
        });

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(matches!(
            result,
            Ok(VersionedCsiRequest::V0_3(CsiRequest::Explain { requests })) if requests.len() == 1
        ));
    }

    #[test]
    fn explain_response() {
        let response = CsiResponse::Explain(vec![vec![
            inference::TextScore {
                start: 0,
                length: 5,
                score: 0.5,
            }
            .into(),
        ]]);
        let serialized = serde_json::to_value(response).unwrap();
        assert_eq!(
            serialized,
            json!([[{
                "start": 0,
                "length": 5,
                "score": 0.5
            }]])
        );
    }

    #[test]
    fn select_language_response() {
        let response = CsiResponse::SelectLanguage(vec![Some(
            language_selection::Language::new("eng".to_owned()).into(),
        )]);
        let serialized = serde_json::to_value(response).unwrap();
        assert_eq!(serialized, json!(["eng"]));
    }

    #[test]
    fn search_result_response() {
        let response = CsiResponse::SearchResult(vec![vec![
            search::SearchResult {
                document_path: search::DocumentPath {
                    namespace: "Kernel".to_string(),
                    collection: "test".to_string(),
                    name: "kernel-docs".to_string(),
                },
                content: "Hello".to_string(),
                score: 0.5,
                start: search::TextCursor {
                    item: 0,
                    position: 0,
                },
                end: search::TextCursor {
                    item: 0,
                    position: 0,
                },
            }
            .into(),
        ]]);

        let serialized = serde_json::to_value(response).unwrap();

        assert_eq!(
            serialized,
            json!([[{
                "document_path": {
                    "namespace": "Kernel",
                    "collection": "test",
                    "name": "kernel-docs"
                },
                "content": "Hello",
                "score": 0.5,
                "start": {
                    "item": 0,
                    "position": 0
                },
                "end": {
                    "item": 0,
                    "position": 0
                }
            }]])
        );
    }

    #[test]
    fn document_response() {
        let response = CsiResponse::Documents(vec![
            search::Document {
                path: search::DocumentPath {
                    namespace: "Kernel".to_string(),
                    collection: "test".to_string(),
                    name: "kernel-docs".to_string(),
                },
                contents: vec![search::Modality::Text {
                    text: "Hello".to_string(),
                }],
                metadata: Some(json!({ "created": "1970-07-01T14:10:11Z" })),
            }
            .into(),
        ]);

        let serialized = serde_json::to_value(response).unwrap();

        assert_eq!(
            serialized,
            json!([{
                "path": {
                    "namespace": "Kernel",
                    "collection": "test",
                    "name": "kernel-docs"
                },
                "contents": [
                    {
                        "modality": "text",
                        "text": "Hello"
                    }
                ],
                "metadata": {
                    "created": "1970-07-01T14:10:11Z"
                }
            }])
        );
    }

    #[test]
    fn complete_response() {
        let response = CsiResponse::Complete(vec![
            inference::Completion {
                text: "Hello".to_string(),
                finish_reason: inference::FinishReason::Stop,
                logprobs: vec![inference::Distribution {
                    sampled: inference::Logprob {
                        token: vec![],
                        logprob: 0.0,
                    },
                    top: vec![],
                }],
                usage: inference::TokenUsage {
                    prompt: 0,
                    completion: 0,
                },
            }
            .into(),
        ]);

        let serialized = serde_json::to_value(response).unwrap();

        assert_eq!(
            serialized,
            json!([{
                "text": "Hello",
                "finish_reason": "stop",
                "logprobs": [
                    {
                        "sampled": {
                            "token": [],
                            "logprob": 0.0
                        },
                        "top": []
                    }
                ],
                "usage": {
                    "prompt": 0,
                    "completion": 0
                }
            }])
        );
    }
    #[test]
    fn chat_response() {
        let response = CsiResponse::Chat(vec![
            inference::ChatResponse {
                message: inference::Message {
                    role: "user".to_string(),
                    content: "Hello".to_string(),
                },
                finish_reason: inference::FinishReason::Stop,
                logprobs: vec![inference::Distribution {
                    sampled: inference::Logprob {
                        token: vec![],
                        logprob: 0.0,
                    },
                    top: vec![],
                }],
                usage: inference::TokenUsage {
                    prompt: 0,
                    completion: 0,
                },
            }
            .into(),
        ]);

        let serialized = serde_json::to_value(response).unwrap();

        assert_eq!(
            serialized,
            json!([{
                "message": {
                    "role": "user",
                    "content": "Hello"
                },
                "finish_reason": "stop",
                "logprobs": [
                    {
                        "sampled": {
                            "token": [],
                            "logprob": 0.0
                        },
                        "top": []
                    }
                ],
                "usage": {
                    "prompt": 0,
                    "completion": 0
                }
            }])
        );
    }

    #[test]
    fn chunk_request() {
        let request = json!({
            "version": "0.3",
            "function": "chunk",
            "requests": [
                {
                    "text": "Hello",
                    "params": {
                        "model": "pharia-1-llm-7b-control",
                        "max_tokens": 128,
                        "overlap": 10
                    }
                },
                {
                    "text": "Hello",
                    "params": {
                        "model": "pharia-1-llm-7b-control",
                        "max_tokens": 128,
                        "overlap": 10
                    }
                }
            ]
        });

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(matches!(
            result,
            Ok(VersionedCsiRequest::V0_3(CsiRequest::Chunk { requests })) if requests.len() == 2
        ));
    }

    #[test]
    fn chunk_with_offset_request() {
        let request = json!({
            "version": "0.3",
            "function": "chunk_with_offsets",
            "requests": [
                {
                    "text": "Hello",
                    "params": {
                        "model": "pharia-1-llm-7b-control",
                        "max_tokens": 128,
                        "overlap": 10
                    },
                    "character_offsets": true
                },
                {
                    "text": "Hello",
                    "params": {
                        "model": "pharia-1-llm-7b-control",
                        "max_tokens": 128,
                        "overlap": 10
                    },
                    "character_offsets": false
                }
            ]
        });

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(matches!(
            result,
            Ok(VersionedCsiRequest::V0_3(CsiRequest::ChunkWithOffsets { requests })) if requests.len() == 2
        ));
    }

    #[test]
    fn chunk_with_offset_response() {
        let response = CsiResponse::ChunkWithOffsets(vec![vec![
            chunking::Chunk {
                text: "Hello".to_string(),
                byte_offset: 0,
                character_offset: Some(10),
            }
            .into(),
            chunking::Chunk {
                text: "Hello".to_string(),
                byte_offset: 5,
                character_offset: None,
            }
            .into(),
        ]]);

        let serialized = serde_json::to_value(response).unwrap();

        assert_eq!(
            serialized,
            json!([[{
                "text": "Hello",
                "byte_offset": 0,
                "character_offset": 10
            },{
                "text": "Hello",
                "byte_offset": 5,
                "character_offset": None::<u64>
            }]])
        );
    }

    #[test]
    fn select_language_request() {
        let request = json!({
            "version": "0.3",
            "function": "select_language",
            "requests": [
                {
                    "text": "Hello",
                    "languages": ["eng", "deu"]
                },
                {
                    "text": "Hello",
                    "languages": ["eng", "deu"]
                }
            ]
        });

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(matches!(
            result,
            Ok(VersionedCsiRequest::V0_3(CsiRequest::SelectLanguage { requests })) if requests.len() == 2
        ));
    }

    #[test]
    fn complete_request() {
        let request = json!({
            "version": "0.3",
            "function": "complete",
            "requests": [
                {
                    "prompt": "Hello",
                    "model": "pharia-1-llm-7b-control",
                    "params": {
                        "return_special_tokens": true,
                        "max_tokens": 128,
                        "temperature": null,
                        "top_k": null,
                        "top_p": null,
                        "stop": [],
                        "frequency_penalty": null,
                        "presence_penalty": null,
                        "logprobs": "no"
                    }
                },
                {
                    "prompt": "Hello",
                    "model": "pharia-1-llm-7b-control",
                    "params": {
                        "return_special_tokens": true,
                        "max_tokens": 128,
                        "temperature": null,
                        "top_k": null,
                        "top_p": null,
                        "stop": [],
                        "frequency_penalty": null,
                        "presence_penalty": null,
                        "logprobs": {
                            "top": 10
                        }
                    }
                }
            ]
        });

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(matches!(
            result,
            Ok(VersionedCsiRequest::V0_3(CsiRequest::Complete { requests })) if requests.len() == 2
        ));
    }

    #[test]
    fn search_request() {
        let request = json!({
            "version": "0.3",
            "function": "search",
            "requests": [
                {
                    "query": "Hello",
                    "index_path": {
                        "namespace": "Kernel",
                        "collection": "test",
                        "index": "asym-64"
                    },
                    "max_results": 10,
                    "min_score": null,
                    "filters": [
                        {
                            "with": [
                                {
                                    "metadata": {
                                        "field": "created",
                                        "after": "1970-07-01T14:10:11Z"
                                    }
                                }
                            ]
                        }
                    ]
                },
                {
                    "query": "Hello",
                    "index_path": {
                        "namespace": "Kernel",
                        "collection": "test",
                        "index": "asym-64"
                    },
                    "max_results": 10,
                    "min_score": null,
                    "filters": []
                }
            ]
        });

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(matches!(
            result,
            Ok(VersionedCsiRequest::V0_3(CsiRequest::Search { requests })) if requests.len() == 2
        ));
    }

    #[test]
    fn chat_request() {
        let request = json!({
            "version": "0.3",
            "function": "chat",
            "requests": [
                {
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
                        "top_k": null,
                        "top_p": null,
                        "stop": [],
                        "logprobs": "no"
                    },
                }
            ]
        });

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(matches!(
            result,
            Ok(VersionedCsiRequest::V0_3(CsiRequest::Chat { requests })) if requests.len() == 1
        ));
    }

    #[test]
    fn documents_request() {
        let request = json!({
            "version": "0.3",
            "function": "documents",
            "requests": [
                {
                    "namespace": "Kernel",
                    "collection": "test",
                    "name": "kernel-docs"
                }
            ]
        });

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(
            matches!(result, Ok(VersionedCsiRequest::V0_3(CsiRequest::Documents { requests })) if requests.len() == 1)
        );
    }

    #[test]
    fn document_metadata_request() {
        let request = json!({
            "version": "0.3",
            "function": "document_metadata",
            "requests": [
                {
                    "namespace": "Kernel",
                    "collection": "test",
                    "name": "asym-64"
                },
                {
                    "namespace": "Kernel",
                    "collection": "test",
                    "name": "asym-64"
                }
            ]
        });

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(
            matches!(result, Ok(VersionedCsiRequest::V0_3(CsiRequest::DocumentMetadata { requests })) if requests.len() == 2)
        );
    }

    #[test]
    fn unknown_request() {
        let request = json!({
            "version": "0.3",
            "function": "whatever",
        });

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(
            matches!(result, Ok(VersionedCsiRequest::V0_3(CsiRequest::Unknown { function: Some(function) })) if function == "whatever")
        );
    }
}
