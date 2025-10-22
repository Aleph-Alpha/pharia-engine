use axum::response::sse::Event;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{inference, tool};

#[derive(Deserialize)]
pub struct Argument {
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
pub struct Distribution {
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
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
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
pub struct TokenUsage {
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

#[derive(Serialize)]
pub struct SseErrorEvent {
    pub message: String,
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
