use std::convert::Infallible;

use async_stream::try_stream;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response, Sse, sse::Event},
    routing::post,
};
use axum_extra::{
    TypedHeader,
    headers::{Authorization, authorization::Bearer},
};
use derive_more::{From, Into};
use futures::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    chunking,
    csi::Csi,
    inference, language_selection, search,
    shell::{AppState, CsiState},
    skill_runtime::SkillRuntimeApi,
    skill_store::SkillStoreApi,
};

// Make our own error that wraps `anyhow::Error`.
struct CsiShellError(anyhow::Error);

// Tell axum how to convert `AppError` into a response.
impl IntoResponse for CsiShellError {
    fn into_response(self) -> Response {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Something went wrong: {}", self.0),
        )
            .into_response()
    }
}

// This enables using `?` on functions that return `Result<_, anyhow::Error>` to turn them into
// `Result<_, CsiShellError>`. That way you don't need to do that manually.
impl<E> From<E> for CsiShellError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self(err.into())
    }
}

pub fn http<C, R, S>() -> Router<AppState<C, R, S>>
where
    C: Csi + Clone + Sync + Send + 'static,
    R: SkillRuntimeApi + Clone + Send + Sync + 'static,
    S: SkillStoreApi + Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/chat_stream", post(chat_stream))
        .route("/completion_stream", post(completion_stream))
        .route("/explain", post(explain))
}

async fn explain<C>(
    State(CsiState(csi)): State<CsiState<C>>,
    bearer: TypedHeader<Authorization<Bearer>>,
    Json(requests): Json<Vec<ExplainRequest>>,
) -> Result<Json<Vec<Vec<TextScore>>>, CsiShellError>
where
    C: Csi,
{
    let results = csi
        .explain(
            bearer.token().to_owned(),
            requests.into_iter().map(Into::into).collect(),
        )
        .await
        .map(|v| {
            v.into_iter()
                .map(|r| r.into_iter().map(Into::into).collect())
                .collect()
        })?;
    Ok(Json(results))
}

async fn completion_stream<C>(
    State(CsiState(csi)): State<CsiState<C>>,
    bearer: TypedHeader<Authorization<Bearer>>,
    Json(request): Json<CompletionRequest>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>>
where
    C: Csi + Clone + Sync,
{
    let mut recv = csi
        .completion_stream(bearer.token().to_owned(), request.into())
        .await;

    let stream = try_stream! {
        while let Some(result) = recv.recv().await {
            yield match result {
                Ok(event) => event.into(),
                Err(err) => Event::default()
                    .event("error")
                    .json_data(SseErrorEvent { message: err.to_string() })
                    .expect("`json_data` must only be called once."),
            }
        }
    };

    Sse::new(stream)
}

impl From<inference::CompletionEvent> for Event {
    fn from(event: inference::CompletionEvent) -> Self {
        match event {
            inference::CompletionEvent::Append { text, logprobs } => Event::default()
                .event("delta")
                .json_data(CompletionDeltaEvent {
                    text,
                    logprobs: logprobs.into_iter().map(Into::into).collect(),
                })
                .expect("`json_data` must only be called once."),
            inference::CompletionEvent::End { finish_reason } => Event::default()
                .event("finished")
                .json_data(CompletionFinishedEvent {
                    finish_reason: finish_reason.into(),
                })
                .expect("`json_data` must only be called once."),
            inference::CompletionEvent::Usage { usage } => Event::default()
                .event("usage")
                .json_data(CompletionUsageEvent {
                    usage: usage.into(),
                })
                .expect("`json_data` must only be called once."),
        }
    }
}

#[derive(Serialize)]
struct SseErrorEvent {
    message: String,
}

#[derive(Serialize)]
struct CompletionDeltaEvent {
    text: String,
    logprobs: Vec<Distribution>,
}

#[derive(Serialize)]
struct CompletionFinishedEvent {
    finish_reason: FinishReason,
}

#[derive(Serialize)]
struct CompletionUsageEvent {
    usage: TokenUsage,
}

async fn chat_stream<C>(
    State(CsiState(csi)): State<CsiState<C>>,
    bearer: TypedHeader<Authorization<Bearer>>,
    Json(request): Json<ChatRequest>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>>
where
    C: Csi + Clone + Sync,
{
    let mut recv = csi
        .chat_stream(bearer.token().to_owned(), request.into())
        .await;

    let stream = try_stream! {
        while let Some(result) = recv.recv().await {
            yield match result {
                Ok(event) => event.into(),
                Err(err) => Event::default()
                    .event("error")
                    .json_data(SseErrorEvent { message: err.to_string() })
                    .expect("`json_data` must only be called once."),
            }
        }
    };

    Sse::new(stream)
}

impl From<inference::ChatEvent> for Event {
    fn from(event: inference::ChatEvent) -> Self {
        match event {
            inference::ChatEvent::MessageBegin { role } => Event::default()
                .event("message_start")
                .json_data(ChatMessageStartEvent { role })
                .expect("`json_data` must only be called once."),
            inference::ChatEvent::MessageAppend { content, logprobs } => Event::default()
                .event("message_delta")
                .json_data(ChatMessageDeltaEvent {
                    content,
                    logprobs: logprobs.into_iter().map(Into::into).collect(),
                })
                .expect("`json_data` must only be called once."),
            inference::ChatEvent::MessageEnd { finish_reason } => Event::default()
                .event("message_end")
                .json_data(ChatMessageEndEvent {
                    finish_reason: finish_reason.into(),
                })
                .expect("`json_data` must only be called once."),
            inference::ChatEvent::Usage { usage } => Event::default()
                .event("usage")
                .json_data(ChatUsageEvent {
                    usage: usage.into(),
                })
                .expect("`json_data` must only be called once."),
        }
    }
}

#[derive(Serialize)]
struct ChatMessageStartEvent {
    role: String,
}

#[derive(Serialize)]
struct ChatMessageDeltaEvent {
    content: String,
    logprobs: Vec<Distribution>,
}

#[derive(Serialize)]
struct ChatMessageEndEvent {
    finish_reason: FinishReason,
}

#[derive(Serialize)]
struct ChatUsageEvent {
    usage: TokenUsage,
}

#[derive(Deserialize)]
struct CompletionRequest {
    prompt: String,
    model: String,
    params: CompletionParams,
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

#[derive(Deserialize)]
struct CompletionParams {
    return_special_tokens: bool,
    max_tokens: Option<u32>,
    temperature: Option<f64>,
    top_k: Option<u32>,
    top_p: Option<f64>,
    stop: Vec<String>,
    frequency_penalty: Option<f64>,
    presence_penalty: Option<f64>,
    logprobs: Logprobs,
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
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Logprobs {
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
struct ChatParams {
    max_tokens: Option<u32>,
    temperature: Option<f64>,
    top_p: Option<f64>,
    frequency_penalty: Option<f64>,
    presence_penalty: Option<f64>,
    logprobs: Logprobs,
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
struct ChatRequest {
    model: String,
    messages: Vec<Message>,
    params: ChatParams,
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

#[derive(Serialize, Deserialize)]
struct Message {
    role: String,
    content: String,
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
enum Granularity {
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
struct ExplainRequest {
    prompt: String,
    target: String,
    model: String,
    granularity: Granularity,
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
struct DocumentPath {
    namespace: String,
    collection: String,
    name: String,
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
struct IndexPath {
    namespace: String,
    collection: String,
    index: String,
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
struct SearchRequest {
    query: String,
    index_path: IndexPath,
    max_results: u32,
    min_score: Option<f64>,
    filters: Vec<Filter>,
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
    item: u32,
    position: u32,
}

impl From<search::TextCursor> for TextCursor {
    fn from(value: search::TextCursor) -> Self {
        let search::TextCursor { item, position } = value;
        TextCursor { item, position }
    }
}

#[derive(Serialize)]
struct SearchResult {
    document_path: DocumentPath,
    content: String,
    score: f64,
    start: TextCursor,
    end: TextCursor,
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
enum Filter {
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
enum FilterCondition {
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
struct MetadataFilter {
    field: String,
    #[serde(flatten)]
    condition: MetadataFilterCondition,
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
enum MetadataFilterCondition {
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
enum MetadataFieldValue {
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
struct ChunkParams {
    model: String,
    max_tokens: u32,
    overlap: u32,
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
struct ChunkRequest {
    text: String,
    params: ChunkParams,
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
struct ChunkWithOffsetRequest {
    text: String,
    params: ChunkParams,
    character_offsets: bool,
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
struct Language(String);

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
struct SelectLanguageRequest {
    text: String,
    languages: Vec<Language>,
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
