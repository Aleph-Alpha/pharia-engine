//! Helpers for tracing inference requests following OpenTelemetry `GenAI` semantic conventions.
//!
//! See: <https://opentelemetry.io/docs/specs/semconv/gen-ai/gen-ai-agent-spans/>

use crate::{
    inference::{AssistantMessage, CompletionRequest, Message},
    logging::TracingContext,
};
use tracing::{Level, field};

use super::actor::ChatRequest;

impl TracingContext {
    /// Creates a child tracing context for a chat request with `GenAI` semantic convention
    /// attributes.
    ///
    /// For OpenTelemetry spans, all attributes that are to be recorded need to be specified when
    /// creating the span.
    pub fn child_from_chat_request(self, request: &ChatRequest) -> Self {
        let span = tracing::span!(
            target: "pharia-kernel::inference",
            parent: self.span(),
            Level::INFO,
            "chat",
            "gen_ai.request.model" = request.model,
            "gen_ai.request.max_tokens" = request.params.max_tokens,
            "gen_ai.request.temperature" = request.params.temperature,
            "gen_ai.request.top_p" = request.params.top_p,
            "gen_ai.request.frequency_penalty" = request.params.frequency_penalty,
            "gen_ai.request.presence_penalty" = request.params.presence_penalty,
            "gen_ai.input.messages" = field::Empty,
            "gen_ai.output.messages" = field::Empty,
        );
        Self::new(span)
    }

    /// Capture the input messages of a chat request on the span.
    ///
    /// Ensure that the span has been created by calling [`Self::child_from_chat_request`] so that
    /// the attributes can be captured.
    pub fn capture_input_messages(&self, messages: &[Message]) {
        self.span().record(
            "gen_ai.input.messages",
            serde_json::to_string(
                &messages
                    .iter()
                    .map(Message::as_otel_message)
                    .collect::<Vec<_>>(),
            )
            .unwrap(),
        );
    }

    /// Capture the output message of a chat request on the span.
    ///
    /// Ensure that the span has been created by calling [`Self::child_from_chat_request`] so that
    /// the attributes can be captured.
    pub fn capture_output_message(&self, message: &AssistantMessage) {
        self.span().record(
            "gen_ai.output.messages",
            serde_json::to_string(&[message.as_otel_message()]).unwrap(),
        );
    }

    /// Creates a child tracing context for a completion request with `GenAI` semantic convention
    /// attributes.
    ///
    /// For OpenTelemetry spans, all attributes that are to be recorded need to be specified when
    /// creating the span.
    pub fn child_from_completion_request(self, request: &CompletionRequest) -> Self {
        let span = tracing::span!(
            target: "pharia-kernel::inference",
            parent: self.span(),
            Level::INFO,
            "text_completion",
            "gen_ai.request.model" = request.model,
            "gen_ai.request.max_tokens" = request.params.max_tokens,
            "gen_ai.request.temperature" = request.params.temperature,
            "gen_ai.request.top_p" = request.params.top_p,
            "gen_ai.request.frequency_penalty" = request.params.frequency_penalty,
            "gen_ai.request.presence_penalty" = request.params.presence_penalty,
        );
        Self::new(span)
    }
}
