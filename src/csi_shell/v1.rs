use std::convert::Infallible;

use async_stream::try_stream;
use axum::{
    Json,
    extract::State,
    response::{Sse, sse::Event},
};
use axum_extra::{
    TypedHeader,
    headers::{Authorization, authorization::Bearer},
};
use futures::Stream;
use serde::{Deserialize, Serialize};

use crate::{csi::Csi, inference, shell::CsiState};

pub async fn completion_stream<C>(
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

pub async fn chat_stream<C>(
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
