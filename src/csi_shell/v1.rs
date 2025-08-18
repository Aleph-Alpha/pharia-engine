use std::convert::Infallible;

use async_stream::try_stream;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response, Sse, sse::Event},
    routing::post,
};
use derive_more::{From, Into};
use futures::Stream;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    authorization::Authentication,
    chunking,
    csi::RawCsi,
    csi_shell::{CsiProvider, CsiState},
    inference, language_selection,
    logging::TracingContext,
    namespace_watcher::Namespace,
    search, tool,
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

// Questionable wether these transformations for CsiShellError should live here, since only one
// implementation can exist. Which means either in the Future we need different version of the
// `CsiShellError` or we will move it out of a version specific module.

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

pub fn http<T>() -> Router<T>
where
    T: CsiProvider + Clone + Send + Sync + 'static,
    T::Csi: RawCsi + Clone + Sync + Send + 'static,
{
    Router::new()
        .route("/chat", post(chat))
        .route("/chat_stream", post(chat_stream))
        .route("/chunk", post(chunk))
        .route("/chunk_with_offsets", post(chunk_with_offsets))
        .route("/complete", post(complete))
        .route("/completion_stream", post(completion_stream))
        .route("/documents", post(documents))
        .route("/document_metadata", post(document_metadata))
        .route("/explain", post(explain))
        .route("/invoke_tool", post(invoke_tool))
        .route("/list_tools", post(list_tools))
        .route("/search", post(search))
        .route("/select_language", post(select_language))
}

#[derive(Deserialize)]
struct Argument {
    name: String,
    value: Value,
}

impl From<Argument> for tool::Argument {
    fn from(value: Argument) -> Self {
        Self {
            name: value.name,
            value: value.value.to_string().into_bytes(),
        }
    }
}

#[derive(Deserialize)]
struct InvokeRequest {
    name: String,
    arguments: Vec<Argument>,
}

impl From<InvokeRequest> for tool::InvokeRequest {
    fn from(value: InvokeRequest) -> Self {
        Self {
            name: value.name,
            arguments: value.arguments.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Deserialize)]
struct InvokeRequests {
    namespace: Namespace,
    requests: Vec<InvokeRequest>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ToolModality {
    Text { text: String },
}

impl From<tool::Modality> for ToolModality {
    fn from(value: tool::Modality) -> Self {
        match value {
            tool::Modality::Text { text } => ToolModality::Text { text },
        }
    }
}

async fn invoke_tool<C>(
    State(CsiState(csi)): State<CsiState<C>>,
    Json(requests): Json<InvokeRequests>,
) -> Json<Vec<Value>>
where
    C: RawCsi,
{
    let tracing_context = TracingContext::current();
    let results = csi
        .invoke_tool(
            requests.namespace,
            tracing_context,
            requests.requests.into_iter().map(Into::into).collect(),
        )
        .await;

    // Even a tool error means returning a 200, as the error is part of the json response.
    let results = results
        .into_iter()
        .map(|result| match result {
            Ok(result) => json!(
                result
                    .into_modalities()
                    .map(Into::into)
                    .collect::<Vec<ToolModality>>()
            ),
            Err(err) => json!(err.to_string()),
        })
        .collect();
    Json(results)
}

#[derive(Serialize)]
struct ToolDescription {
    name: String,
    description: String,
    input_schema: Value,
}

impl From<tool::ToolDescription> for ToolDescription {
    fn from(value: tool::ToolDescription) -> Self {
        let tool::ToolDescription {
            name,
            description,
            input_schema,
        } = value;
        Self {
            name,
            description,
            input_schema,
        }
    }
}

#[derive(Deserialize)]
struct ListToolsRequest {
    namespace: Namespace,
}

async fn list_tools<C>(
    State(CsiState(csi)): State<CsiState<C>>,
    Json(request): Json<ListToolsRequest>,
) -> Result<Json<Vec<ToolDescription>>, CsiShellError>
where
    C: RawCsi,
{
    let tracing_context = TracingContext::current();
    let results = csi
        .list_tools(request.namespace, tracing_context)
        .await
        .map(|v| v.into_iter().map(Into::into).collect())?;
    Ok(Json(results))
}

async fn select_language<C>(
    State(CsiState(csi)): State<CsiState<C>>,
    Json(requests): Json<Vec<SelectLanguageRequest>>,
) -> Result<Json<Vec<Option<Language>>>, CsiShellError>
where
    C: RawCsi + Sync,
{
    let tracing_context = TracingContext::current();
    let results = csi
        .select_language(
            requests.into_iter().map(Into::into).collect(),
            tracing_context,
        )
        .await
        .map(|v| v.into_iter().map(|m| m.map(Into::into)).collect())?;
    Ok(Json(results))
}

async fn search<C>(
    State(CsiState(csi)): State<CsiState<C>>,
    auth: Authentication,
    Json(requests): Json<Vec<SearchRequest>>,
) -> Result<Json<Vec<Vec<SearchResult>>>, CsiShellError>
where
    C: RawCsi,
{
    let tracing_context = TracingContext::current();
    let results = csi
        .search(
            auth,
            tracing_context,
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

async fn documents<C>(
    State(CsiState(csi)): State<CsiState<C>>,
    auth: Authentication,
    Json(requests): Json<Vec<DocumentPath>>,
) -> Result<Json<Vec<Document>>, CsiShellError>
where
    C: RawCsi,
{
    let tracing_context = TracingContext::current();
    let results = csi
        .documents(
            auth,
            tracing_context,
            requests.into_iter().map(Into::into).collect(),
        )
        .await
        .map(|v| v.into_iter().map(Into::into).collect())?;
    Ok(Json(results))
}

async fn document_metadata<C>(
    State(CsiState(csi)): State<CsiState<C>>,
    auth: Authentication,
    Json(requests): Json<Vec<DocumentPath>>,
) -> Result<Json<Vec<Option<Value>>>, CsiShellError>
where
    C: RawCsi,
{
    let tracing_context = TracingContext::current();
    let results = csi
        .document_metadata(
            auth,
            tracing_context,
            requests.into_iter().map(Into::into).collect(),
        )
        .await?;
    Ok(Json(results))
}

async fn chat<C>(
    State(CsiState(csi)): State<CsiState<C>>,
    auth: Authentication,
    Json(requests): Json<Vec<ChatRequest>>,
) -> Result<Json<Vec<ChatResponse>>, CsiShellError>
where
    C: RawCsi,
{
    let tracing_context = TracingContext::current();
    let requests = requests
        .into_iter()
        .map(TryInto::try_into)
        .collect::<Result<Vec<_>, _>>()?;
    let results = csi
        .chat(auth, tracing_context, requests)
        .await
        .map(|v| v.into_iter().map(Into::into).collect())?;
    Ok(Json(results))
}

async fn chunk<C>(
    State(CsiState(csi)): State<CsiState<C>>,
    auth: Authentication,
    Json(requests): Json<Vec<ChunkRequest>>,
) -> Result<Json<Vec<Vec<String>>>, CsiShellError>
where
    C: RawCsi,
{
    let tracing_context = TracingContext::current();
    let results = csi
        .chunk(
            auth,
            tracing_context,
            requests.into_iter().map(Into::into).collect(),
        )
        .await
        .map(|v| {
            v.into_iter()
                .map(|c| c.into_iter().map(|c| c.text).collect())
                .collect()
        })?;
    Ok(Json(results))
}

async fn chunk_with_offsets<C>(
    State(CsiState(csi)): State<CsiState<C>>,
    auth: Authentication,
    Json(requests): Json<Vec<ChunkWithOffsetRequest>>,
) -> Result<Json<Vec<Vec<ChunkWithOffset>>>, CsiShellError>
where
    C: RawCsi,
{
    let tracing_context = TracingContext::current();
    let results = csi
        .chunk(
            auth,
            tracing_context,
            requests.into_iter().map(Into::into).collect(),
        )
        .await
        .map(|v| {
            v.into_iter()
                .map(|c| c.into_iter().map(Into::into).collect())
                .collect()
        })?;
    Ok(Json(results))
}

async fn explain<C>(
    State(CsiState(csi)): State<CsiState<C>>,
    auth: Authentication,
    Json(requests): Json<Vec<ExplainRequest>>,
) -> Result<Json<Vec<Vec<TextScore>>>, CsiShellError>
where
    C: RawCsi,
{
    let tracing_context = TracingContext::current();
    let results = csi
        .explain(
            auth,
            tracing_context,
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

async fn complete<C>(
    State(CsiState(csi)): State<CsiState<C>>,
    auth: Authentication,
    Json(requests): Json<Vec<CompletionRequest>>,
) -> Result<Json<Vec<Completion>>, CsiShellError>
where
    C: RawCsi,
{
    let tracing_context = TracingContext::current();
    let results = csi
        .complete(
            auth,
            tracing_context,
            requests.into_iter().map(Into::into).collect(),
        )
        .await
        .map(|v| v.into_iter().map(Into::into).collect())?;
    Ok(Json(results))
}

async fn completion_stream<C>(
    State(CsiState(csi)): State<CsiState<C>>,
    auth: Authentication,
    Json(request): Json<CompletionRequest>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>>
where
    C: RawCsi + Clone + Sync,
{
    let tracing_context = TracingContext::current();
    let mut recv = csi
        .completion_stream(auth, tracing_context, request.into())
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
                .event("append")
                .json_data(CompletionAppendEvent {
                    text,
                    logprobs: logprobs.into_iter().map(Into::into).collect(),
                })
                .expect("`json_data` must only be called once."),
            inference::CompletionEvent::End { finish_reason } => Event::default()
                .event("end")
                .json_data(CompletionEndEvent {
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
struct CompletionAppendEvent {
    text: String,
    logprobs: Vec<Distribution>,
}

#[derive(Serialize)]
struct CompletionEndEvent {
    finish_reason: FinishReason,
}

#[derive(Serialize)]
struct CompletionUsageEvent {
    usage: TokenUsage,
}

async fn chat_stream<C>(
    State(CsiState(csi)): State<CsiState<C>>,
    auth: Authentication,
    Json(request): Json<ChatRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, CsiShellError>
where
    C: RawCsi + Clone + Sync,
{
    let tracing_context = TracingContext::current();
    let request = request.try_into()?;
    let mut recv = csi.chat_stream(auth, tracing_context, request).await;

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

    Ok(Sse::new(stream))
}

impl From<inference::ChatEvent> for Event {
    fn from(event: inference::ChatEvent) -> Self {
        match event {
            inference::ChatEvent::MessageBegin { role } => Event::default()
                .event("message_begin")
                .json_data(ChatMessageStartEvent { role })
                .expect("`json_data` must only be called once."),
            inference::ChatEvent::MessageAppend { content, logprobs } => Event::default()
                .event("message_append")
                .json_data(ChatMessageAppendEvent {
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
            inference::ChatEvent::ToolCall(tool_calls) => Event::default()
                .event("tool_call")
                .json_data(ToolCallEvent {
                    tool_calls: tool_calls.into_iter().map(Into::into).collect(),
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
struct ChatMessageAppendEvent {
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

#[derive(Serialize)]
struct ToolCallChunk {
    index: u32,
    id: Option<String>,
    name: Option<String>,
    arguments: Option<String>,
}

impl From<inference::ToolCallChunk> for ToolCallChunk {
    fn from(value: inference::ToolCallChunk) -> Self {
        let inference::ToolCallChunk {
            index,
            id,
            name,
            arguments,
        } = value;
        ToolCallChunk {
            index,
            id,
            name,
            arguments,
        }
    }
}

#[derive(Serialize)]
struct ToolCallEvent {
    tool_calls: Vec<ToolCallChunk>,
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
    #[serde(default)]
    echo: bool,
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
            echo,
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
            echo,
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
struct Function {
    name: String,
    description: Option<String>,
    parameters: Option<Value>,
    strict: Option<bool>,
}

impl From<Function> for inference::Function {
    fn from(value: Function) -> Self {
        let Function {
            name,
            description,
            parameters,
            strict,
        } = value;
        inference::Function {
            name,
            description,
            parameters: parameters.map(|p| serde_json::to_vec(&p).unwrap()),
            strict,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ToolChoice {
    Auto,
    Required,
    None,
    Named(String),
}

impl From<ToolChoice> for inference::ToolChoice {
    fn from(value: ToolChoice) -> Self {
        match value {
            ToolChoice::Auto => inference::ToolChoice::Auto,
            ToolChoice::Required => inference::ToolChoice::Required,
            ToolChoice::None => inference::ToolChoice::None,
            ToolChoice::Named(name) => inference::ToolChoice::Named(name),
        }
    }
}

#[derive(Deserialize)]
struct JsonSchema {
    name: String,
    description: Option<String>,
    schema: Option<Value>,
    strict: Option<bool>,
}

impl From<JsonSchema> for inference::JsonSchema {
    fn from(value: JsonSchema) -> Self {
        let JsonSchema {
            name,
            description,
            schema,
            strict,
        } = value;
        inference::JsonSchema {
            name,
            description,
            schema: schema.map(|s| serde_json::to_vec(&s).unwrap()),
            strict,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ResponseFormat {
    Text,
    JsonObject,
    JsonSchema(JsonSchema),
}

impl From<ResponseFormat> for inference::ResponseFormat {
    fn from(value: ResponseFormat) -> Self {
        match value {
            ResponseFormat::Text => inference::ResponseFormat::Text,
            ResponseFormat::JsonObject => inference::ResponseFormat::JsonObject,
            ResponseFormat::JsonSchema(json_schema) => {
                inference::ResponseFormat::JsonSchema(json_schema.into())
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReasoningEffort {
    Low,
    Medium,
    High,
}

impl From<ReasoningEffort> for inference::ReasoningEffort {
    fn from(value: ReasoningEffort) -> Self {
        match value {
            ReasoningEffort::Low => inference::ReasoningEffort::Low,
            ReasoningEffort::Medium => inference::ReasoningEffort::Medium,
            ReasoningEffort::High => inference::ReasoningEffort::High,
        }
    }
}

#[derive(Deserialize)]
struct ChatParams {
    max_tokens: Option<u32>,
    max_completion_tokens: Option<u32>,
    temperature: Option<f64>,
    top_p: Option<f64>,
    frequency_penalty: Option<f64>,
    presence_penalty: Option<f64>,
    logprobs: Logprobs,
    tools: Option<Vec<Function>>,
    tool_choice: Option<ToolChoice>,
    parallel_tool_calls: Option<bool>,
    response_format: Option<ResponseFormat>,
    reasoning_effort: Option<ReasoningEffort>,
}

impl From<ChatParams> for inference::ChatParams {
    fn from(value: ChatParams) -> Self {
        let ChatParams {
            max_tokens,
            max_completion_tokens,
            temperature,
            top_p,
            frequency_penalty,
            presence_penalty,
            logprobs,
            tools,
            tool_choice,
            parallel_tool_calls,
            response_format,
            reasoning_effort,
        } = value;
        inference::ChatParams {
            max_tokens,
            max_completion_tokens,
            temperature,
            top_p,
            frequency_penalty,
            presence_penalty,
            logprobs: logprobs.into(),
            tools: tools.map(|t| t.into_iter().map(Into::into).collect()),
            tool_choice: tool_choice.map(Into::into),
            parallel_tool_calls,
            response_format: response_format.map(Into::into),
            reasoning_effort: reasoning_effort.map(Into::into),
        }
    }
}

#[derive(Deserialize)]
struct ChatRequest {
    model: String,
    messages: Vec<Message>,
    params: ChatParams,
}

impl TryFrom<ChatRequest> for inference::ChatRequest {
    type Error = anyhow::Error;

    fn try_from(value: ChatRequest) -> Result<Self, Self::Error> {
        let ChatRequest {
            model,
            messages,
            params,
        } = value;
        let messages = messages
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(inference::ChatRequest {
            model,
            messages,
            params: params.into(),
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum FinishReason {
    Stop,
    Length,
    ContentFilter,
    ToolCalls,
}

impl From<inference::FinishReason> for FinishReason {
    fn from(value: inference::FinishReason) -> Self {
        match value {
            inference::FinishReason::Stop => FinishReason::Stop,
            inference::FinishReason::Length => FinishReason::Length,
            inference::FinishReason::ContentFilter => FinishReason::ContentFilter,
            inference::FinishReason::ToolCalls => FinishReason::ToolCalls,
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
struct ToolCall {
    id: String,
    name: String,
    arguments: String,
}

impl From<inference::ToolCall> for ToolCall {
    fn from(value: inference::ToolCall) -> Self {
        let inference::ToolCall {
            id,
            name,
            arguments,
        } = value;
        ToolCall {
            id,
            name,
            arguments,
        }
    }
}

impl From<ToolCall> for inference::ToolCall {
    fn from(value: ToolCall) -> Self {
        let ToolCall {
            id,
            name,
            arguments,
        } = value;
        inference::ToolCall {
            id,
            name,
            arguments,
        }
    }
}

/// Representation of a Message for serialization/deserialization via the CSI shell.
///
/// While we could also go for an enum approach here to mirror the domain representation in
/// [`crate::inference::Message`], we currently do not see any benefit in doing so, and stick to
/// the one struct approach here.
#[derive(Serialize, Deserialize)]
struct Message {
    role: String,
    content: Option<String>,
    tool_call_id: Option<String>,
    tool_calls: Option<Vec<ToolCall>>,
}

impl From<inference::AssistantMessage> for Message {
    fn from(value: inference::AssistantMessage) -> Self {
        let inference::AssistantMessage {
            content,
            tool_calls,
        } = value;
        Message {
            role: inference::AssistantMessage::role().to_owned(),
            content,
            tool_call_id: None,
            tool_calls: tool_calls.map(|calls| calls.into_iter().map(Into::into).collect()),
        }
    }
}

impl TryFrom<Message> for inference::Message {
    type Error = anyhow::Error;

    fn try_from(value: Message) -> Result<Self, Self::Error> {
        let Message {
            role,
            content,
            tool_call_id,
            tool_calls,
        } = value;
        match role.as_str() {
            "assistant" => Ok(inference::Message::Assistant(inference::AssistantMessage {
                content,
                tool_calls: tool_calls.map(|calls| calls.into_iter().map(Into::into).collect()),
            })),
            "tool" => Ok(inference::Message::Tool(inference::ToolMessage {
                // We did previously accept tool call messages without a tool call id, as the
                // AlephAlpha inference backend does not require it. Therefore, we still accept it.
                content: content.ok_or(anyhow::anyhow!("Tool message must have content"))?,
                tool_call_id: tool_call_id.unwrap_or_default(),
            })),
            _ => Ok(inference::Message::Other {
                role,
                content: content
                    .ok_or(anyhow::anyhow!("Non-assistant message must have content"))?,
            }),
        }
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

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use mime::APPLICATION_JSON;
    use reqwest::{
        Method,
        header::{AUTHORIZATION, CONTENT_TYPE},
    };
    use serde_json::json;
    use tokio::sync::mpsc;

    use crate::{
        csi::tests::RawCsiDouble,
        inference::{ChatEvent, ChatRequest, InferenceError, ToolCallChunk},
        namespace_watcher::Namespace,
        tool::{Argument, InvokeRequest, ToolError, ToolOutput},
    };

    use super::*;
    use tower::util::ServiceExt;

    impl Argument {
        pub fn new(name: impl Into<String>, value: impl Into<Vec<u8>>) -> Self {
            Self {
                name: name.into(),
                value: value.into(),
            }
        }
    }

    #[tokio::test]
    async fn message_content_can_be_none() {
        // Given a csi mock that:
        // 1. asserts that incoming messages can be developer, user and tool messages
        // 2. returns a message with a tool call and no content
        #[derive(Clone)]
        struct RawCsiMock;
        impl RawCsiDouble for RawCsiMock {
            async fn chat(
                &self,
                _auth: Authentication,
                _tracing_context: TracingContext,
                requests: Vec<inference::ChatRequest>,
            ) -> anyhow::Result<Vec<inference::ChatResponse>> {
                let messages = requests[0].messages.clone();
                assert_eq!(messages.len(), 4);
                assert!(matches!(
                    &messages[0],
                    inference::Message::Other {
                        role,
                        ..
                    }
                if role == "developer"));
                assert!(matches!(
                    &messages[1],
                    inference::Message::Other { role, .. } if role == "user"
                ));
                assert!(matches!(messages[2], inference::Message::Assistant { .. }));
                assert!(matches!(
                    &messages[3],
                    inference::Message::Tool(inference::ToolMessage {
                        tool_call_id,
                        ..
                    }) if tool_call_id == "1"
                ));

                Ok(vec![inference::ChatResponse {
                    message: inference::AssistantMessage {
                        content: None,
                        tool_calls: None,
                    },
                    finish_reason: inference::FinishReason::Stop,
                    logprobs: vec![],
                    usage: inference::TokenUsage {
                        prompt: 0,
                        completion: 0,
                    },
                }])
            }
        }

        #[derive(Clone)]
        struct CsiProviderStub;
        impl CsiProvider for CsiProviderStub {
            type Csi = RawCsiMock;
            fn csi(&self) -> &Self::Csi {
                &RawCsiMock
            }
        }

        // When doing a chat request
        let body = json!([{
            "model": "dummy",
            "messages": [
                {"role": "developer", "content": "You are a helpful assistant."},
                {"role": "user", "content": "What is the result of 1 + 2?"},
                {"role": "assistant", "tool_calls": [{"id": "1", "name": "add", "arguments": "{\"a\": 1, \"b\": 2}"}]},
                {"role": "tool", "content": "Result of the tool call is 3", "tool_call_id": "1"}
            ],
            "params": {
                "logprobs": "no",
            }
        }]);
        let response = http()
            .with_state(CsiProviderStub)
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, "Bearer test")
                    .uri("/chat")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then the response is successful, and we get the two events from the mock
        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&body).unwrap();
        let expected_body = json!([{
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": null,
                "tool_call_id": null,
            },
            "finish_reason": "stop",
            "logprobs": [],
            "usage": {
                "prompt": 0,
                "completion": 0
            }
        }]);
        assert_eq!(body, expected_body);
    }

    #[tokio::test]
    async fn stream_tool_calling_via_csi() {
        // Given a csi mock that asserts that
        // 1. asserts that the tool related parameters are provided
        // 2. returns tool call events
        #[derive(Clone)]
        struct RawCsiMock;
        impl RawCsiDouble for RawCsiMock {
            async fn chat_stream(
                &self,
                _auth: Authentication,
                _tracing_context: TracingContext,
                request: ChatRequest,
            ) -> mpsc::Receiver<Result<ChatEvent, InferenceError>> {
                assert_eq!(request.params.tools.as_ref().unwrap().len(), 1);
                assert_eq!(
                    request.params.tool_choice.as_ref().unwrap(),
                    &inference::ToolChoice::Named("add".to_string())
                );
                assert!(request.params.parallel_tool_calls.unwrap());
                assert_eq!(
                    request.params.response_format.as_ref().unwrap(),
                    &inference::ResponseFormat::JsonSchema(inference::JsonSchema {
                        name: "add".to_string(),
                        description: None,
                        schema: None,
                        strict: None,
                    })
                );

                let (tx, rx) = mpsc::channel(1);
                let events = vec![
                    ChatEvent::MessageBegin {
                        role: "tool".to_string(),
                    },
                    ChatEvent::ToolCall(vec![ToolCallChunk {
                        index: 0,
                        id: Some("1".to_string()),
                        name: Some("add".to_string()),
                        arguments: None,
                    }]),
                ];
                tokio::spawn(async move {
                    for event in events {
                        tx.send(Ok(event)).await.unwrap();
                    }
                });
                rx
            }
        }

        #[derive(Clone)]
        struct CsiProviderStub;
        impl CsiProvider for CsiProviderStub {
            type Csi = RawCsiMock;
            fn csi(&self) -> &Self::Csi {
                &RawCsiMock
            }
        }

        // When doing a chat request with all the tool related parameters
        let body = json!({
            "model": "dummy",
            "messages": [{"role": "user", "content": "Hello, world!"}],
            "params": {
                "logprobs": "no",
                "tools": [{"name": "add", "description": "Add two numbers", "parameters": {"type": "object", "properties": {"a": {"type": "number"}, "b": {"type": "number"}}, "required": ["a", "b"]}}],
                "tool_choice": {
                    "named": "add"
                },
                "parallel_tool_calls": true,
                "response_format": {
                    "json_schema": {
                        "name": "add",
                    }
                }
            }
        });
        let response = http()
            .with_state(CsiProviderStub)
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, "Bearer test")
                    .uri("/chat_stream")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then the response is successful, and we get the two events from the mock
        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body = String::from_utf8(body.to_vec()).unwrap();
        let expected_body = "\
            event: message_begin\n\
            data: {\"role\":\"tool\"}\n\n\
            event: tool_call\n\
            data: {\"tool_calls\":[{\"index\":0,\"id\":\"1\",\"name\":\"add\",\"arguments\":null}]}\n\n\
            ";

        assert_eq!(body, expected_body);
    }

    #[tokio::test]
    async fn tool_invokation_error_is_returned_as_string() {
        // Given a csi that always returns an error
        #[derive(Clone)]
        struct ToolSaboteur;
        impl RawCsiDouble for ToolSaboteur {
            async fn invoke_tool(
                &self,
                _namespace: Namespace,
                _tracing_context: TracingContext,
                _requests: Vec<InvokeRequest>,
            ) -> Vec<Result<ToolOutput, ToolError>> {
                vec![Err(ToolError::ToolExecution("Out of cheese!".to_string()))]
            }
        }

        #[derive(Clone)]
        struct CsiProviderStub;
        impl CsiProvider for CsiProviderStub {
            type Csi = ToolSaboteur;
            fn csi(&self) -> &Self::Csi {
                &ToolSaboteur
            }
        }

        // When we send a request to the invoke_tool endpoint
        let body = json!({
            "namespace": "dummy",
            "requests": []
        });
        let response = http()
            .with_state(CsiProviderStub)
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, "Bearer test")
                    .uri("/invoke_tool")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then the response is successful, but it contains the error
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(body, "[\"Out of cheese!\"]");
    }

    #[tokio::test]
    async fn csi_shell_can_list_tools() {
        // Given a csi mock that always returns a single tool for the 42 namespace
        #[derive(Clone)]
        struct RawCsiMock;

        impl RawCsiDouble for RawCsiMock {
            async fn list_tools(
                &self,
                namespace: Namespace,
                _tracing_context: TracingContext,
            ) -> anyhow::Result<Vec<tool::ToolDescription>> {
                assert_eq!(namespace.as_str(), "42");
                Ok(vec![tool::ToolDescription {
                    name: "add".to_string(),
                    description: "Add two numbers".to_string(),
                    input_schema: json!({
                        "type": "object",
                        "properties": {
                            "a": {"type": "number"},
                            "b": {"type": "number"},
                        },
                        "required": ["a", "b"],
                    }),
                }])
            }
        }

        #[derive(Clone)]
        struct CsiProviderStub;
        impl CsiProvider for CsiProviderStub {
            type Csi = RawCsiMock;
            fn csi(&self) -> &Self::Csi {
                &RawCsiMock
            }
        }

        // When we send a request to the list_tools endpoint
        let body = json!({
            "namespace": "42",
        });
        let response = http()
            .with_state(CsiProviderStub)
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, "Bearer test")
                    .uri("/list_tools")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then the response is successful and contains the expected tool
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(
            body,
            "[{\"name\":\"add\",\"description\":\"Add two numbers\",\"input_schema\":{\"properties\":{\"a\":{\"type\":\"number\"},\"b\":{\"type\":\"number\"}},\"required\":[\"a\",\"b\"],\"type\":\"object\"}}]"
        );
    }

    #[tokio::test]
    async fn tool_invokation_via_csi_shell() {
        // Given a csi mock that asserts on the input and returns a fixed output
        #[derive(Clone)]
        struct RawCsiMock;
        impl RawCsiDouble for RawCsiMock {
            async fn invoke_tool(
                &self,
                _namespace: Namespace,
                _tracing_context: TracingContext,
                mut requests: Vec<InvokeRequest>,
            ) -> Vec<Result<ToolOutput, ToolError>> {
                let InvokeRequest { name, arguments } = requests.remove(0);
                let expected_args = vec![Argument::new("a", "1"), Argument::new("b", "2")];
                assert_eq!(arguments, expected_args);
                assert_eq!(name, "add");
                vec![Ok(ToolOutput::from_text("3"))]
            }
        }

        #[derive(Clone)]
        struct CsiProviderStub;
        impl CsiProvider for CsiProviderStub {
            type Csi = RawCsiMock;
            fn csi(&self) -> &Self::Csi {
                &RawCsiMock
            }
        }

        // When we send a request to the invoke_tool endpoint
        let body = json!({
            "namespace": "test",
            "requests": [{
                "name": "add",
                "arguments": [
                    {
                        "name": "a",
                        "value": 1
                    },
                    {
                        "name": "b",
                        "value": 2
                    }
                ]
            }]
        });
        let response = http()
            .with_state(CsiProviderStub)
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, "Bearer test")
                    .uri("/invoke_tool")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then the response is successful and contains the expected tool output
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(body, "[[{\"text\":\"3\",\"type\":\"text\"}]]");
    }

    #[tokio::test]
    async fn select_language_endpoint_works() {
        // Given a csi that always returns en
        #[derive(Clone)]
        struct RawCsiStub;
        impl RawCsiDouble for RawCsiStub {
            async fn select_language(
                &self,
                _requests: Vec<language_selection::SelectLanguageRequest>,
                _tracing_context: TracingContext,
            ) -> anyhow::Result<Vec<Option<language_selection::Language>>> {
                Ok(vec![Some(language_selection::Language::new(
                    "en".to_string(),
                ))])
            }
        }

        #[derive(Clone)]
        struct CsiProviderStub;
        impl CsiProvider for CsiProviderStub {
            type Csi = RawCsiStub;
            fn csi(&self) -> &Self::Csi {
                &RawCsiStub
            }
        }

        // When we send a request to the select_language endpoint
        let body = json!([{
            "text": "Hello world",
            "languages": ["en", "es"]
        }]);
        let response = http()
            .with_state(CsiProviderStub)
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, "Bearer test")
                    .uri("/select_language")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then the response is successful and contains the expected language
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"[\"en\"]");
    }
}
