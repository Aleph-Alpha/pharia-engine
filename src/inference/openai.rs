use async_openai::{
    self, Client,
    config::OpenAIConfig,
    error::{ApiError, OpenAIError},
    types::{
        ChatChoiceLogprobs, ChatCompletionRequestMessage, ChatCompletionResponseMessage,
        ChatCompletionStreamOptions, ChatCompletionTokenLogprob, CompletionUsage,
        CreateChatCompletionRequest, CreateChatCompletionResponse,
        CreateChatCompletionStreamResponse, FinishReason, TopLogprobs,
    },
};
use futures::StreamExt;
use tokio::sync::mpsc;

use crate::{
    inference,
    inference::{InferenceError, client::InferenceClient},
    logging::TracingContext,
};

pub struct OpenAiClient {
    client: Client<OpenAIConfig>,
}

impl OpenAiClient {
    pub fn new(host: impl Into<String>, api_key: impl Into<String>) -> Self {
        let config = OpenAIConfig::new()
            .with_api_key(api_key)
            .with_api_base(host);
        Self {
            client: Client::with_config(config),
        }
    }

    const NOT_SUPPORTED: &str = "Inference backend is currently configured to an OpenAI compatible \
    one. For this, we only support chat requests. For other request types, please ask your operator \
    to configure the inference backend to use the Aleph Alpha inference backend.";
}

impl TryFrom<inference::Message> for ChatCompletionRequestMessage {
    type Error = InferenceError;

    fn try_from(message: inference::Message) -> Result<Self, Self::Error> {
        let inference::Message { role, content } = message;
        match role.as_str() {
            "user" => Ok(ChatCompletionRequestMessage::User(content.into())),
            "assistant" => Ok(ChatCompletionRequestMessage::Assistant(content.into())),
            "system" => Ok(ChatCompletionRequestMessage::System(content.into())),
            // we currently do not support tool messages, as we have no concept of the tool id yet,
            // which we would need to provide here
            _ => Err(InferenceError::ToolCallNotSupported(format!(
                "Invalid role: {role}"
            ))),
        }
    }
}

/// Not supporting tool calls yet.
impl TryFrom<ChatCompletionResponseMessage> for inference::Message {
    type Error = InferenceError;

    fn try_from(message: ChatCompletionResponseMessage) -> Result<Self, Self::Error> {
        message
            .content
            .map(|content| inference::Message {
                role: message.role.to_string(),
                content,
            })
            .ok_or_else(|| {
                // The content will be None if there is a tool call field.
                InferenceError::ToolCallNotSupported("Invalid message without content".to_string())
            })
    }
}

impl From<CompletionUsage> for inference::TokenUsage {
    fn from(usage: CompletionUsage) -> Self {
        // OpenAI provides more fields like `prompt_tokens_details` and `completion_tokens_details`.
        inference::TokenUsage {
            prompt: usage.prompt_tokens,
            completion: usage.completion_tokens,
        }
    }
}

impl From<TopLogprobs> for inference::Logprob {
    fn from(logprobs: TopLogprobs) -> Self {
        inference::Logprob {
            logprob: f64::from(logprobs.logprob),
            // We also have a bytes field, but it is optional.
            token: logprobs.token.into_bytes(),
        }
    }
}

impl From<ChatCompletionTokenLogprob> for inference::Distribution {
    fn from(logprob: ChatCompletionTokenLogprob) -> Self {
        inference::Distribution {
            sampled: inference::Logprob {
                logprob: f64::from(logprob.logprob),
                token: logprob.token.into_bytes(),
            },
            top: logprob.top_logprobs.into_iter().map(Into::into).collect(),
        }
    }
}

/// While async-openai represents no logprobs as an Option in the type system, we use an empty
/// vector to represent this. Also, we have no concept of logprobs.refusal yet. If logprobs.content
/// is None, we also map this to an empty vector.
fn map_logprobs(logprobs: Option<ChatChoiceLogprobs>) -> Vec<inference::Distribution> {
    if let Some(logprobs) = logprobs {
        if let Some(content) = logprobs.content {
            content.into_iter().map(Into::into).collect()
        } else {
            vec![]
        }
    } else {
        vec![]
    }
}

impl TryFrom<&inference::ChatRequest> for CreateChatCompletionRequest {
    type Error = InferenceError;

    #[allow(clippy::cast_possible_truncation)]
    fn try_from(request: &inference::ChatRequest) -> Result<Self, Self::Error> {
        let inference::ChatRequest {
            model,
            messages,
            params,
        } = request;
        let inference::ChatParams {
            max_tokens,
            temperature,
            top_p,
            frequency_penalty,
            presence_penalty,
            logprobs,
        } = params;
        let messages = messages
            .iter()
            .map(|m| ChatCompletionRequestMessage::try_from(m.clone()))
            .collect::<Result<Vec<_>, _>>()?;
        let request = CreateChatCompletionRequest {
            model: model.clone(),
            messages,
            logprobs: if let inference::Logprobs::No = logprobs {
                None
            } else {
                Some(true)
            },
            top_logprobs: match logprobs {
                inference::Logprobs::Top(k) => Some(*k),
                inference::Logprobs::Sampled => Some(1),
                inference::Logprobs::No => None,
            },
            temperature: temperature.map(|t| t as f32),
            top_p: top_p.map(|p| p as f32),
            frequency_penalty: frequency_penalty.map(|p| p as f32),
            presence_penalty: presence_penalty.map(|p| p as f32),
            // `max_tokens` is deprecated in favor of `max_completion_tokens`
            // An upper bound for the number of tokens that can be generated for a completion, including visible output tokens and reasoning tokens.
            max_completion_tokens: *max_tokens,
            ..Default::default()
        };
        Ok(request)
    }
}

impl TryFrom<FinishReason> for inference::FinishReason {
    type Error = InferenceError;

    fn try_from(finish_reason: FinishReason) -> Result<Self, Self::Error> {
        match finish_reason {
            FinishReason::Stop => Ok(inference::FinishReason::Stop),
            FinishReason::Length => Ok(inference::FinishReason::Length),
            FinishReason::ContentFilter => Ok(inference::FinishReason::ContentFilter),
            FinishReason::FunctionCall | FinishReason::ToolCalls => {
                Err(InferenceError::ToolCallNotSupported(format!(
                    "Unsupported finish reason: {finish_reason:?}",
                )))
            }
        }
    }
}

impl TryFrom<CreateChatCompletionResponse> for inference::ChatResponse {
    type Error = InferenceError;

    fn try_from(response: CreateChatCompletionResponse) -> Result<Self, Self::Error> {
        let first_choice = response.choices.into_iter().next().ok_or_else(|| {
            InferenceError::Other(anyhow::anyhow!(
                "Expected at least one choice in the chat completion response."
            ))
        })?;

        // It is not quite clear why finish reason is represented as an Option. The OpenAI API docs
        // specify it to always exist:
        // https://platform.openai.com/docs/api-reference/chat/object#chat/object-choices
        let finish_reason = first_choice
            .finish_reason
            .ok_or_else(|| {
                anyhow::anyhow!("Expected chat completion response to have a finish reason.")
            })?
            .try_into()?;

        // It is not quite clear why usage is represented as an Option. The OpenAI API docs specify
        // it to always exist:
        // https://platform.openai.com/docs/api-reference/chat/object#chat/object-usage
        let usage = response
            .usage
            .ok_or_else(|| anyhow::anyhow!("Expected chat completion response to have a usage."))?
            .into();

        let message = inference::Message::try_from(first_choice.message.clone())?;
        let response = inference::ChatResponse {
            message,
            finish_reason,
            logprobs: map_logprobs(first_choice.logprobs),
            usage,
        };
        Ok(response)
    }
}

impl From<OpenAIError> for InferenceError {
    fn from(error: OpenAIError) -> Self {
        match error {
            OpenAIError::ApiError(ApiError {
                code: Some(code), ..
            }) if code == "invalid_api_key" => InferenceError::Unauthorized,
            OpenAIError::ApiError(ApiError {
                code: Some(code), ..
            }) if code == "model_not_found" => InferenceError::ModelNotFound,
            _ => InferenceError::Other(anyhow::anyhow!(
                "Error while calling OpenAI chat completion API: {:?}",
                error
            )),
        }
    }
}

impl TryFrom<CreateChatCompletionStreamResponse> for inference::ChatEvent {
    type Error = InferenceError;

    fn try_from(event: CreateChatCompletionStreamResponse) -> Result<Self, Self::Error> {
        if let Some(usage) = event.usage {
            let usage = usage.into();
            return Ok(inference::ChatEvent::Usage { usage });
        }

        let first_choice = event.choices.into_iter().next().ok_or_else(|| {
            anyhow::anyhow!("Expected at least one choice in the chat completion stream response.")
        })?;
        if let Some(finish_reason) = first_choice.finish_reason {
            let finish_reason = finish_reason.try_into()?;
            return Ok(inference::ChatEvent::MessageEnd { finish_reason });
        }
        if let Some(role) = first_choice.delta.role {
            return Ok(inference::ChatEvent::MessageBegin {
                role: role.to_string(),
            });
        }
        if let Some(content) = first_choice.delta.content {
            let logprobs = map_logprobs(first_choice.logprobs);
            return Ok(inference::ChatEvent::MessageAppend { content, logprobs });
        }
        Err(InferenceError::ToolCallNotSupported(
            "Invalid message without content".to_string(),
        ))
    }
}

impl InferenceClient for OpenAiClient {
    async fn chat(
        &self,
        request: &inference::ChatRequest,
        _api_token: String,
        _tracing_context: &TracingContext,
    ) -> Result<inference::ChatResponse, InferenceError> {
        let request = request.try_into()?;
        let response = self.client.chat().create(request).await?;
        let response = inference::ChatResponse::try_from(response)?;
        Ok(response)
    }

    async fn stream_chat(
        &self,
        request: &inference::ChatRequest,
        _api_token: String,
        _tracing_context: &TracingContext,
        send: mpsc::Sender<inference::ChatEvent>,
    ) -> Result<(), InferenceError> {
        let mut request: CreateChatCompletionRequest = request.try_into()?;
        request.stream_options = Some(ChatCompletionStreamOptions {
            include_usage: true,
        });
        let mut stream = self.client.chat().create_stream(request).await?;

        while let Some(event) = stream.next().await {
            let event = inference::ChatEvent::try_from(event?)?;
            drop(send.send(event).await);
        }
        Ok(())
    }

    async fn complete(
        &self,
        _request: &inference::CompletionRequest,
        _api_token: String,
        _tracing_context: &TracingContext,
    ) -> Result<inference::Completion, InferenceError> {
        Err(InferenceError::Other(anyhow::anyhow!(Self::NOT_SUPPORTED)))
    }

    async fn stream_completion(
        &self,
        _request: &inference::CompletionRequest,
        _api_token: String,
        _tracing_context: &TracingContext,
        _send: mpsc::Sender<inference::CompletionEvent>,
    ) -> Result<(), InferenceError> {
        Err(InferenceError::Other(anyhow::anyhow!(Self::NOT_SUPPORTED)))
    }

    async fn explain(
        &self,
        _request: &inference::ExplanationRequest,
        _api_token: String,
        _tracing_context: &TracingContext,
    ) -> Result<inference::Explanation, InferenceError> {
        Err(InferenceError::Other(anyhow::anyhow!(Self::NOT_SUPPORTED)))
    }
}

#[cfg(test)]
mod tests {
    use super::OpenAiClient;
    use tokio::sync::mpsc;

    use crate::{
        inference::{
            ChatEvent, ChatParams, ChatRequest, InferenceError, Logprobs, Message,
            client::InferenceClient,
        },
        logging::TracingContext,
        tests::{openai_inference_url, openai_token},
    };

    #[tokio::test]
    async fn chat_message_conversion() {
        // Given an inference client
        let api_token = openai_token().to_owned();
        let host = openai_inference_url().to_owned();
        let client = OpenAiClient::new(host, api_token);

        let result = <OpenAiClient as InferenceClient>::chat(
            &client,
            &ChatRequest {
                model: "gpt-4o-mini".to_owned(),
                messages: vec![Message::new("user", "An apple a day")],
                params: ChatParams::default(),
            },
            "dummy-token".to_owned(),
            &TracingContext::dummy(),
        )
        .await
        .unwrap();

        // Then a chat response is returned
        assert!(!result.message.content.is_empty());
    }

    #[tokio::test]
    async fn top_logprobs_for_chat() {
        // Given an inference client
        let api_token = openai_token().to_owned();
        let host = openai_inference_url().to_owned();
        let client = OpenAiClient::new(host, api_token);

        let params = ChatParams {
            max_tokens: Some(1),
            logprobs: Logprobs::Top(2),
            temperature: Some(0.0),
            ..Default::default()
        };
        let result = <OpenAiClient as InferenceClient>::chat(
            &client,
            &ChatRequest {
                model: "gpt-4o-mini".to_owned(),
                messages: vec![Message::new("user", "An apple a day")],
                params,
            },
            "dummy-token".to_owned(),
            &TracingContext::dummy(),
        )
        .await
        .unwrap();

        // Then
        assert_eq!(result.logprobs.len(), 1);
        let top_logprobs = &result.logprobs[0].top;
        assert_eq!(top_logprobs.len(), 2);
        assert_eq!(str::from_utf8(&top_logprobs[0].token).unwrap(), "\"");
        assert_eq!(str::from_utf8(&top_logprobs[1].token).unwrap(), "The");
    }

    #[tokio::test]
    async fn model_not_found() {
        // Given
        let api_token = openai_token().to_owned();
        let host = openai_inference_url().to_owned();
        let client = OpenAiClient::new(host, api_token);

        // When doing a chat request against a model that does not exist
        let result = <OpenAiClient as InferenceClient>::chat(
            &client,
            &ChatRequest {
                model: "gpt-4o-mini-non-existent".to_owned(),
                messages: vec![Message::new("user", "An apple a day")],
                params: ChatParams::default(),
            },
            "dummy-token".to_owned(),
            &TracingContext::dummy(),
        )
        .await;

        // Then
        assert!(matches!(result, Err(InferenceError::ModelNotFound)));
    }

    #[tokio::test]
    async fn bad_token_gives_unauthorized() {
        // Given a client with a bad token
        let api_token = "bad-token".to_owned();
        let host = openai_inference_url().to_owned();
        let client = OpenAiClient::new(host, api_token);

        // When doing a chat request
        let result = <OpenAiClient as InferenceClient>::chat(
            &client,
            &ChatRequest {
                model: "gpt-4o-mini".to_owned(),
                messages: vec![Message::new("user", "An apple a day")],
                params: ChatParams::default(),
            },
            "dummy-token".to_owned(),
            &TracingContext::dummy(),
        )
        .await;

        // Then
        assert!(matches!(result, Err(InferenceError::Unauthorized)));
    }

    #[tokio::test]
    async fn chat_stream() {
        // Given
        let api_token = openai_token().to_owned();
        let host = openai_inference_url().to_owned();
        let client = OpenAiClient::new(host, api_token);

        // When
        let params = ChatParams {
            max_tokens: Some(1),
            temperature: Some(0.0),
            ..Default::default()
        };
        let (send, mut recv) = mpsc::channel(1);
        tokio::spawn(async move {
            <OpenAiClient as InferenceClient>::stream_chat(
                &client,
                &ChatRequest {
                    model: "gpt-4o-mini".to_owned(),
                    messages: vec![Message::new("user", "An apple a day")],
                    params,
                },
                "dummy-token".to_owned(),
                &TracingContext::dummy(),
                send,
            )
            .await
            .unwrap();
        });

        let mut events = vec![];
        while let Some(event) = recv.recv().await {
            events.push(event);
        }

        // Then
        assert_eq!(events.len(), 4);
        assert!(matches!(events[0], ChatEvent::MessageBegin { .. }));
        assert!(matches!(events[1], ChatEvent::MessageAppend { .. }));
        assert!(matches!(events[2], ChatEvent::MessageEnd { .. }));
        assert!(matches!(events[3], ChatEvent::Usage { .. }));
    }
}
