use async_openai::{
    self, Client,
    config::OpenAIConfig,
    error::{ApiError, OpenAIError},
    types::{
        ChatChoiceLogprobs, ChatCompletionMessageToolCall, ChatCompletionNamedToolChoice,
        ChatCompletionRequestMessage, ChatCompletionResponseMessage, ChatCompletionStreamOptions,
        ChatCompletionTokenLogprob, ChatCompletionTool, ChatCompletionToolChoiceOption,
        ChatCompletionToolType, CompletionUsage, CreateChatCompletionRequest,
        CreateChatCompletionResponse, CreateChatCompletionStreamResponse, FinishReason,
        FunctionCall, FunctionName, FunctionObject, TopLogprobs,
    },
};
use futures::StreamExt;
use tokio::sync::mpsc;

use crate::{
    authorization::Authentication,
    inference::{self, InferenceError, client::InferenceClient},
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

impl From<ChatCompletionMessageToolCall> for inference::ToolCall {
    fn from(tool_call: ChatCompletionMessageToolCall) -> Self {
        let ChatCompletionMessageToolCall {
            id: _,
            r#type: _,
            function: FunctionCall { name, arguments },
        } = tool_call;
        inference::ToolCall { name, arguments }
    }
}

/// Not supporting tool calls yet.
impl From<ChatCompletionResponseMessage> for inference::ResponseMessage {
    fn from(message: ChatCompletionResponseMessage) -> Self {
        let ChatCompletionResponseMessage {
            role,
            content,
            tool_calls,
            ..
        } = message;
        inference::ResponseMessage {
            role: role.to_string(),
            content: content.map(|c| c.to_string()),
            tool_calls: tool_calls.map(|calls| calls.into_iter().map(Into::into).collect()),
        }
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
            tools,
            tool_choice,
        } = params;
        let messages = messages
            .iter()
            .map(|m| ChatCompletionRequestMessage::try_from(m.clone()))
            .collect::<Result<Vec<_>, _>>()?;
        let tools = tools
            .as_ref()
            .map(|t| {
                t.iter()
                    .map(TryInto::try_into)
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?;
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
            tools,
            tool_choice: tool_choice.as_ref().map(Into::into),
            ..Default::default()
        };
        Ok(request)
    }
}

impl From<&inference::ToolChoice> for ChatCompletionToolChoiceOption {
    fn from(tool_choice: &inference::ToolChoice) -> Self {
        match tool_choice {
            inference::ToolChoice::None => ChatCompletionToolChoiceOption::None,
            inference::ToolChoice::Auto => ChatCompletionToolChoiceOption::Auto,
            inference::ToolChoice::Required => ChatCompletionToolChoiceOption::Required,
            inference::ToolChoice::Named(name) => {
                ChatCompletionToolChoiceOption::Named(ChatCompletionNamedToolChoice {
                    function: FunctionName {
                        name: name.to_owned(),
                    },
                    r#type: ChatCompletionToolType::Function,
                })
            }
        }
    }
}

impl TryFrom<&inference::Function> for ChatCompletionTool {
    type Error = anyhow::Error;

    fn try_from(function: &inference::Function) -> Result<Self, Self::Error> {
        let inference::Function {
            name,
            description,
            parameters,
            strict,
        } = function;
        let parameters = parameters
            .as_ref()
            .map(|p| serde_json::from_slice(p))
            .transpose()?;
        let function = FunctionObject {
            name: name.clone(),
            description: description.clone(),
            parameters,
            strict: *strict,
        };
        Ok(ChatCompletionTool {
            function,
            ..Default::default()
        })
    }
}

impl From<FinishReason> for inference::FinishReason {
    fn from(finish_reason: FinishReason) -> Self {
        match finish_reason {
            FinishReason::Stop => inference::FinishReason::Stop,
            FinishReason::Length => inference::FinishReason::Length,
            FinishReason::ContentFilter => inference::FinishReason::ContentFilter,
            FinishReason::FunctionCall | FinishReason::ToolCalls => {
                inference::FinishReason::ToolCalls
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
            .into();

        // It is not quite clear why usage is represented as an Option. The OpenAI API docs specify
        // it to always exist:
        // https://platform.openai.com/docs/api-reference/chat/object#chat/object-usage
        let usage = response
            .usage
            .ok_or_else(|| anyhow::anyhow!("Expected chat completion response to have a usage."))?
            .into();

        let message = inference::ResponseMessage::from(first_choice.message.clone());
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
            let finish_reason = finish_reason.into();
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
        _auth: Authentication,
        _tracing_context: &TracingContext,
    ) -> Result<inference::ChatResponse, InferenceError> {
        let openai_request = request.try_into()?;
        let response = self.client.chat().create(openai_request).await?;
        let response = inference::ChatResponse::try_from(response)?;

        // We have an implicit assumption: If no tools are specified in the request, we expect a
        // content in the response. We also have consumers of this functions (old WIT worlds) that
        // never provide tools and always expect a content in the response. One option in Rust would
        // be to specify this in the type system. This could mean having two chat functions, one for
        // requests with tools and one for requests without tools. However, this would mean
        // introducing multiple chat functions in the CSI traits. For now, we believe the best way
        // forward is to check the condition here, and unwrap the content at the consumer side.
        if request.params.tools.is_none() && response.message.content.is_none() {
            return Err(InferenceError::EmptyContent);
        }
        Ok(response)
    }

    async fn stream_chat(
        &self,
        request: &inference::ChatRequest,
        _auth: Authentication,
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
        _auth: Authentication,
        _tracing_context: &TracingContext,
    ) -> Result<inference::Completion, InferenceError> {
        Err(InferenceError::Other(anyhow::anyhow!(Self::NOT_SUPPORTED)))
    }

    async fn stream_completion(
        &self,
        _request: &inference::CompletionRequest,
        _auth: Authentication,
        _tracing_context: &TracingContext,
        _send: mpsc::Sender<inference::CompletionEvent>,
    ) -> Result<(), InferenceError> {
        Err(InferenceError::Other(anyhow::anyhow!(Self::NOT_SUPPORTED)))
    }

    async fn explain(
        &self,
        _request: &inference::ExplanationRequest,
        _auth: Authentication,
        _tracing_context: &TracingContext,
    ) -> Result<inference::Explanation, InferenceError> {
        Err(InferenceError::Other(anyhow::anyhow!(Self::NOT_SUPPORTED)))
    }
}

#[cfg(test)]
mod tests {
    use super::OpenAiClient;
    use async_openai::types::{
        ChatCompletionMessageToolCall, ChatCompletionResponseMessage, ChatCompletionToolType,
        FunctionCall, Role,
    };
    use tokio::sync::mpsc;

    use crate::{
        authorization::Authentication,
        inference::{
            self, ChatEvent, ChatParams, ChatRequest, Function, InferenceError, Logprobs, Message,
            ToolChoice, client::InferenceClient,
        },
        logging::TracingContext,
        tests::{openai_inference_url, openai_token},
    };

    #[test]
    fn tool_call_is_mapped() {
        // Given an OpenAI message with a tool call
        let function_call = FunctionCall {
            name: "get_delivery_date".to_owned(),
            arguments: "{\"order_id\": \"123456\"}".to_owned(),
        };
        let tool_call = ChatCompletionMessageToolCall {
            id: "123456".to_owned(),
            function: function_call,
            r#type: ChatCompletionToolType::Function,
        };

        #[allow(deprecated)]
        let message = ChatCompletionResponseMessage {
            tool_calls: Some(vec![tool_call]),
            role: Role::Assistant,
            content: None,
            refusal: None,
            audio: None,
            function_call: None,
        };

        // When converting to an inference message
        let _message = inference::ResponseMessage::from(message);

        // Then the tool call is available
        // assert_eq!(message.tool_calls.len(), 1);
    }

    #[tokio::test]
    async fn chat_with_function_calling() {
        // Given an inference client
        let api_token = openai_token().to_owned();
        let host = openai_inference_url().to_owned();
        let client = OpenAiClient::new(host, api_token);

        let function = Function {
            name: "get_delivery_date".to_owned(),
            description: Some("Get the delivery date for a given order".to_owned()),
            parameters: Some(
                serde_json::to_vec(&serde_json::json!({
                    "type": "object",
                    "properties": {
                        "order_id": { "type": "string" }
                    }
                }))
                .unwrap(),
            ),
            strict: None,
        };

        let result = <OpenAiClient as InferenceClient>::chat(
            &client,
            &ChatRequest {
                model: "gpt-4o-mini".to_owned(),
                messages: vec![Message::new("user", "When is order 123456 delivered?")],
                params: ChatParams {
                    tools: Some(vec![function]),
                    ..Default::default()
                },
            },
            Authentication::none(),
            &TracingContext::dummy(),
        )
        .await
        .unwrap();

        // Then
        assert!(result.message.content.is_none());
        assert!(result.message.tool_calls.is_some());
    }

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
            Authentication::none(),
            &TracingContext::dummy(),
        )
        .await
        .unwrap();

        // Then a chat response is returned
        assert!(!result.message.content.unwrap().is_empty());
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
            Authentication::none(),
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
            Authentication::none(),
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
            Authentication::none(),
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
                Authentication::none(),
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
        // assert_eq!(events.len(), 4);
        assert!(matches!(events[0], ChatEvent::MessageBegin { .. }));
        assert!(matches!(events[1], ChatEvent::MessageAppend { .. }));
        assert!(matches!(events[2], ChatEvent::MessageEnd { .. }));
        assert!(matches!(events[3], ChatEvent::Usage { .. }));
    }

    #[tokio::test]
    async fn forced_tool_call() {
        // Given a message history that would not require a tool call
        let api_token = openai_token().to_owned();
        let host = openai_inference_url().to_owned();
        let client = OpenAiClient::new(host, api_token);
        let message = Message::new("user", "What is the weather in Berlin?");

        // When forcing a tool call
        let function = Function {
            name: "catch_fish".to_owned(),
            description: Some("Catch a fish (most likely a northern pike)".to_owned()),
            parameters: None,
            strict: None,
        };
        let params = ChatParams {
            tools: Some(vec![function]),
            tool_choice: Some(ToolChoice::Named("catch_fish".to_owned())),
            ..Default::default()
        };
        let result = <OpenAiClient as InferenceClient>::chat(
            &client,
            &ChatRequest {
                model: "gpt-4o-mini".to_owned(),
                messages: vec![message],
                params,
            },
            Authentication::none(),
            &TracingContext::dummy(),
        )
        .await
        .unwrap();

        // Then the model catches a fish
        let tool_calls = result.message.tool_calls.unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].name, "catch_fish");
    }
}
