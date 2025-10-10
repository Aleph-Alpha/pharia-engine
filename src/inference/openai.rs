use async_openai::{
    self, Client,
    config::OpenAIConfig,
    error::{ApiError, OpenAIError},
    types::{
        ChatChoiceLogprobs, ChatCompletionMessageToolCall, ChatCompletionMessageToolCallChunk,
        ChatCompletionNamedToolChoice, ChatCompletionRequestAssistantMessage,
        ChatCompletionRequestMessage, ChatCompletionRequestToolMessage,
        ChatCompletionStreamOptions, ChatCompletionTokenLogprob, ChatCompletionTool,
        ChatCompletionToolChoiceOption, ChatCompletionToolType, CompletionUsage,
        CreateChatCompletionRequest, FinishReason, FunctionCall, FunctionCallStream, FunctionName,
        FunctionObject, ReasoningEffort, ResponseFormat, ResponseFormatJsonSchema, TopLogprobs,
    },
};
use futures::StreamExt;
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::{
    authorization::Authentication,
    inference::{
        self, InferenceError,
        client::{InferenceClient, validate_chat_event, validate_chat_response},
    },
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
        match message {
            inference::Message::Assistant(inference::AssistantMessage {
                content,
                tool_calls,
            }) => Ok(ChatCompletionRequestMessage::Assistant(
                ChatCompletionRequestAssistantMessage {
                    content: content.map(Into::into),
                    tool_calls: tool_calls
                        .map(|tool_calls| tool_calls.into_iter().map(Into::into).collect()),
                    refusal: None,
                    name: None,
                    audio: None,
                    #[allow(deprecated)]
                    function_call: None,
                },
            )),
            inference::Message::Tool(inference::ToolMessage {
                content,
                tool_call_id,
            }) => Ok(ChatCompletionRequestMessage::Tool(
                ChatCompletionRequestToolMessage {
                    content: content.into(),
                    tool_call_id,
                },
            )),
            // We did support upper case role names in the past (at least the AlephAlpha inference
            // accepted it), so we need to continue to support them by matching on the lowercase
            // version of the role name.
            inference::Message::Other { role, content } => match role.to_lowercase().as_str() {
                "system" => Ok(ChatCompletionRequestMessage::System(content.into())),
                "developer" => Ok(ChatCompletionRequestMessage::Developer(content.into())),
                "user" => Ok(ChatCompletionRequestMessage::User(content.into())),
                _ => Err(InferenceError::RoleNotSupported(role)),
            },
        }
    }
}

impl From<inference::ToolCall> for ChatCompletionMessageToolCall {
    fn from(tool_call: inference::ToolCall) -> Self {
        let inference::ToolCall {
            id,
            name,
            arguments,
        } = tool_call;
        ChatCompletionMessageToolCall {
            id,
            function: FunctionCall { name, arguments },
            r#type: ChatCompletionToolType::Function,
        }
    }
}

impl From<ChatCompletionMessageToolCall> for inference::ToolCall {
    fn from(tool_call: ChatCompletionMessageToolCall) -> Self {
        let ChatCompletionMessageToolCall {
            id,
            r#type: _,
            function: FunctionCall { name, arguments },
        } = tool_call;
        inference::ToolCall {
            id,
            name,
            arguments,
        }
    }
}

impl From<ChatCompletionResponseMessage> for inference::AssistantMessage {
    fn from(message: ChatCompletionResponseMessage) -> Self {
        let ChatCompletionResponseMessage {
            content,
            reasoning_content,
            tool_calls,
        } = message;
        inference::AssistantMessage {
            content,
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

impl inference::ChatRequest {
    pub fn as_openai_stream_request(&self) -> Result<CreateChatCompletionRequest, InferenceError> {
        let mut request = self.as_openai_request()?;
        request.stream = Some(true);
        request.stream_options = Some(ChatCompletionStreamOptions {
            include_usage: true,
        });
        Ok(request)
    }

    /// Convert a [`inference::ChatRequest`] into an
    /// [`async_openai::types::CreateChatCompletionRequest`].
    ///
    /// `OpenAI` deprecated the `max_tokens` parameter in favor of `max_completion_tokens`. Our
    /// inference backend does not support the `max_completion_tokens` yet. While initially we
    /// thought we could simply use the deprecated `max_tokens` parameter, it turns out that using
    /// it for reasoning models leads to an error. Therefore, we branch on the inference backend.
    #[allow(clippy::cast_possible_truncation)]
    pub fn as_openai_request(&self) -> Result<CreateChatCompletionRequest, InferenceError> {
        let inference::ChatRequest {
            model,
            messages,
            params,
        } = self;
        let inference::ChatParams {
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
            #[allow(deprecated)]
            max_tokens: *max_tokens,
            max_completion_tokens: *max_completion_tokens,
            tools,
            tool_choice: tool_choice.as_ref().map(Into::into),
            parallel_tool_calls: *parallel_tool_calls,
            response_format: response_format
                .as_ref()
                .map(TryInto::try_into)
                .transpose()?,
            reasoning_effort: reasoning_effort.as_ref().map(Into::into),
            ..Default::default()
        };
        Ok(request)
    }
}

impl From<&inference::ReasoningEffort> for ReasoningEffort {
    fn from(reasoning_effort: &inference::ReasoningEffort) -> Self {
        match reasoning_effort {
            inference::ReasoningEffort::Minimal => ReasoningEffort::Minimal,
            inference::ReasoningEffort::Low => ReasoningEffort::Low,
            inference::ReasoningEffort::Medium => ReasoningEffort::Medium,
            inference::ReasoningEffort::High => ReasoningEffort::High,
        }
    }
}

impl TryFrom<&inference::ResponseFormat> for ResponseFormat {
    type Error = InferenceError;

    fn try_from(response_format: &inference::ResponseFormat) -> Result<Self, Self::Error> {
        match response_format {
            inference::ResponseFormat::Text => Ok(ResponseFormat::Text),
            inference::ResponseFormat::JsonObject => Ok(ResponseFormat::JsonObject),
            inference::ResponseFormat::JsonSchema(json_schema) => Ok(ResponseFormat::JsonSchema {
                json_schema: json_schema.try_into()?,
            }),
        }
    }
}

impl TryFrom<&inference::JsonSchema> for ResponseFormatJsonSchema {
    type Error = InferenceError;

    fn try_from(json_schema: &inference::JsonSchema) -> Result<Self, Self::Error> {
        let inference::JsonSchema {
            name,
            description,
            schema,
            strict,
        } = json_schema;
        let schema = schema
            .as_ref()
            .map(|s| serde_json::from_slice(s))
            .transpose()
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(ResponseFormatJsonSchema {
            name: name.clone(),
            description: description.clone(),
            schema,
            strict: *strict,
        })
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

/// A chat completion message generated by the model.
#[derive(Deserialize, Clone)]
pub struct ChatCompletionResponseMessage {
    /// The contents of the message.
    content: Option<String>,
    /// The reasoning content generated by the model.
    reasoning_content: Option<String>,
    /// The tool calls generated by the model, such as function calls.
    tool_calls: Option<Vec<async_openai::types::ChatCompletionMessageToolCall>>,
}

impl From<ChatCompletionResponseMessage> for inference::AssistantMessageV2 {
    fn from(message: ChatCompletionResponseMessage) -> Self {
        let ChatCompletionResponseMessage {
            content,
            reasoning_content,
            tool_calls,
        } = message;
        inference::AssistantMessageV2 {
            content,
            reasoning_content,
            tool_calls: tool_calls.map(|calls| calls.into_iter().map(Into::into).collect()),
        }
    }
}

#[derive(Deserialize)]
pub struct ChatChoice {
    message: ChatCompletionResponseMessage,
    /// The reason the model stopped generating tokens. This will be `stop` if the model hit a
    /// natural stop point or a provided stop sequence, `length` if the maximum number of
    /// tokens specified in the request was reached, `content_filter` if content was omitted
    /// due to a flag from our content filters, `tool_calls` if the model called a tool, or
    /// `function_call` (deprecated) if the model called a function.
    finish_reason: Option<async_openai::types::FinishReason>,
    /// Log probability information for the choice.
    logprobs: Option<async_openai::types::ChatChoiceLogprobs>,
}

#[derive(Deserialize)]
pub struct ChatResponseReasoningContent {
    /// A list of chat completion choices. Can be more than one if `n` is greater than 1.
    choices: Vec<ChatChoice>,
    usage: Option<async_openai::types::CompletionUsage>,
}

impl TryFrom<ChatResponseReasoningContent> for inference::ChatResponse {
    type Error = InferenceError;

    fn try_from(response: ChatResponseReasoningContent) -> Result<Self, Self::Error> {
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

        let message = inference::AssistantMessage::from(first_choice.message.clone());
        let response = inference::ChatResponse {
            message,
            finish_reason,
            logprobs: map_logprobs(first_choice.logprobs),
            usage,
        };
        Ok(response)
    }
}

impl TryFrom<ChatResponseReasoningContent> for inference::ChatResponseV2 {
    type Error = InferenceError;

    fn try_from(response: ChatResponseReasoningContent) -> Result<Self, Self::Error> {
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

        let message = inference::AssistantMessageV2::from(first_choice.message.clone());
        let response = inference::ChatResponseV2 {
            message,
            finish_reason,
            logprobs: map_logprobs(first_choice.logprobs),
            usage,
        };
        Ok(response)
    }
}

impl TryFrom<async_openai::types::CreateChatCompletionResponse> for inference::ChatResponse {
    type Error = InferenceError;
    fn try_from(
        response: async_openai::types::CreateChatCompletionResponse,
    ) -> Result<Self, Self::Error> {
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

        let message = inference::AssistantMessage::from(first_choice.message.clone());
        let response = inference::ChatResponse {
            message,
            finish_reason,
            logprobs: map_logprobs(first_choice.logprobs),
            usage,
        };
        Ok(response)
    }
}

impl From<async_openai::types::ChatCompletionResponseMessage> for inference::AssistantMessage {
    fn from(message: async_openai::types::ChatCompletionResponseMessage) -> Self {
        let async_openai::types::ChatCompletionResponseMessage {
            content,
            tool_calls,
            ..
        } = message;
        Self {
            content,
            tool_calls: tool_calls.map(|calls| calls.into_iter().map(Into::into).collect()),
        }
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
                "Error while calling OpenAI chat completion API: {:#}",
                error
            )),
        }
    }
}

/// A chat completion delta generated by streamed model responses.
#[derive(Deserialize)]
pub struct ChatCompletionStreamResponseDelta {
    /// The contents of the chunk message.
    content: Option<String>,
    tool_calls: Option<Vec<async_openai::types::ChatCompletionMessageToolCallChunk>>,
    /// The role of the author of this message.
    role: Option<async_openai::types::Role>,
}

#[derive(Deserialize)]
pub struct ChatChoiceStream {
    delta: ChatCompletionStreamResponseDelta,
    /// The reason the model stopped generating tokens. This will be
    /// `stop` if the model hit a natural stop point or a provided
    /// stop sequence,
    ///
    /// `length` if the maximum number of tokens specified in the
    /// request was reached,
    /// `content_filter` if content was omitted due to a flag from our
    /// content filters,
    /// `tool_calls` if the model called a tool, or `function_call`
    /// (deprecated) if the model called a function.
    finish_reason: Option<async_openai::types::FinishReason>,
    /// Log probability information for the choice.
    logprobs: Option<async_openai::types::ChatChoiceLogprobs>,
}

#[derive(Deserialize)]
/// Represents a streamed chunk of a chat completion response returned by model, based on the
/// provided input.
pub struct ChatStreamWithReasoning {
    /// A list of chat completion choices. Can contain more than one elements if `n` is greater
    /// than 1. Can also be empty for the last chunk if you set `stream_options: {"include_usage":
    /// true}`.
    choices: Vec<ChatChoiceStream>,

    /// An optional field that will only be present when you set `stream_options: {"include_usage":
    /// true}` in your request. When present, it contains a null value except for the last
    /// chunk which contains the token usage statistics for the entire request.
    usage: Option<async_openai::types::CompletionUsage>,
}

impl inference::ChatEvent {
    /// Convert a [`ChatStreamWithReasoning`] into a vector of [`inference::ChatEvent`].
    ///
    /// While it might seem natural to have a 1-1 mapping between events from the inference backend
    /// and events in our domain, we have chosen a domain model in which `MessageBegin` (containing
    /// the role) and `MessageAppend`/`ToolCall` (containing the content) are separate events.
    /// However, at least for tool calls coming from `OpenAI`, there is no guarantee for the event
    /// containing the role to be distinct from the event containing the tool call.
    /// Therefore, a single inference backend event can turn itself into multiple events in our
    /// domain model.
    pub fn from_stream(event: ChatStreamWithReasoning) -> Vec<Self> {
        // In case we receive a usage, there is no choices, and no other events.
        if let Some(usage) = event.usage {
            let usage = usage.into();
            return vec![inference::ChatEvent::Usage { usage }];
        }

        let Some(first_choice) = event.choices.into_iter().next() else {
            // GitHub models returns an empty choices array on the first event. To be compatible,
            // we do not raise an error but simply wait for the next event.
            return vec![];
        };

        let mut events = if let Some(role) = first_choice.delta.role {
            vec![inference::ChatEvent::MessageBegin {
                role: role.to_string(),
            }]
        } else {
            vec![]
        };

        // If the message has a main part, it is either a content chunk or a tool call.
        // We filter for empty content, as message containing the role often has an empty content.
        if let Some(content) = first_choice.delta.content
            && !content.is_empty()
        {
            let logprobs = map_logprobs(first_choice.logprobs);
            events.push(inference::ChatEvent::MessageAppend { content, logprobs });
        } else if let Some(tool_call) = first_choice.delta.tool_calls {
            events.push(inference::ChatEvent::ToolCall(
                tool_call.into_iter().map(Into::into).collect(),
            ));
        }

        if let Some(finish_reason) = first_choice.finish_reason {
            let finish_reason = finish_reason.into();
            events.push(inference::ChatEvent::MessageEnd { finish_reason });
        }
        events
    }
}

impl From<ChatCompletionMessageToolCallChunk> for inference::ToolCallChunk {
    fn from(chunk: ChatCompletionMessageToolCallChunk) -> Self {
        let ChatCompletionMessageToolCallChunk {
            index,
            id,
            function,
            r#type: _,
        } = chunk;
        if let Some(FunctionCallStream { name, arguments }) = function {
            inference::ToolCallChunk {
                index,
                id,
                name,
                arguments,
            }
        } else {
            inference::ToolCallChunk {
                index,
                id,
                name: None,
                arguments: None,
            }
        }
    }
}

impl InferenceClient for OpenAiClient {
    async fn chat(
        &self,
        request: &inference::ChatRequest,
        _auth: Authentication,
        _tracing_context: &TracingContext,
    ) -> Result<inference::ChatResponse, InferenceError> {
        let openai_request = request.as_openai_request()?;
        let response = self.client.chat().create(openai_request).await?;
        let response = inference::ChatResponse::try_from(response)?;
        validate_chat_response(request, &response)?;
        Ok(response)
    }
    async fn chat_v2(
        &self,
        request: &inference::ChatRequest,
        _auth: Authentication,
        _tracing_context: &TracingContext,
    ) -> Result<inference::ChatResponseV2, InferenceError> {
        let openai_request = request.as_openai_request()?;
        let response: ChatResponseReasoningContent =
            self.client.chat().create_byot(openai_request).await?;
        let response = inference::ChatResponseV2::try_from(response)?;
        Ok(response)
    }

    async fn stream_chat_v2(
        &self,
        request: &inference::ChatRequest,
        _auth: Authentication,
        _tracing_context: &TracingContext,
        send: mpsc::Sender<inference::ChatEvent>,
    ) -> Result<(), InferenceError> {
        let openai_request = request.as_openai_stream_request()?;
        let mut stream = self
            .client
            .chat()
            .create_stream_byot(openai_request)
            .await?;

        while let Some(event) = stream.next().await {
            let events = inference::ChatEvent::from_stream(event?);
            for event in events {
                validate_chat_event(request, &event)?;
                drop(send.send(event).await);
            }
        }
        Ok(())
    }

    async fn stream_chat(
        &self,
        request: &inference::ChatRequest,
        _auth: Authentication,
        _tracing_context: &TracingContext,
        send: mpsc::Sender<inference::ChatEvent>,
    ) -> Result<(), InferenceError> {
        let openai_request = request.as_openai_stream_request()?;
        let mut stream = self
            .client
            .chat()
            .create_stream_byot(openai_request)
            .await?;

        while let Some(event) = stream.next().await {
            let events = inference::ChatEvent::from_stream(event?);
            for event in events {
                validate_chat_event(request, &event)?;
                drop(send.send(event).await);
            }
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
    use super::*;
    use async_openai::types::{
        ChatCompletionMessageToolCall, ChatCompletionToolType, FunctionCall,
    };
    use tokio::sync::mpsc;

    use crate::{
        authorization::Authentication,
        inference::{
            self, ChatEvent, ChatParams, ChatRequest, Function, InferenceError, JsonSchema,
            Logprobs, Message, ReasoningEffort, ResponseFormat, ToolChoice,
            client::InferenceClient,
        },
        logging::TracingContext,
        tests::{openai_inference_url, openai_token},
    };

    const NON_REASONING_MODEL: &str = "gpt-4o-mini";
    const REASONING_MODEL: &str = "o4-mini";

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

        let message = ChatCompletionResponseMessage {
            tool_calls: Some(vec![tool_call]),
            content: None,
            reasoning_content: None,
        };

        // When converting to an inference assistant message
        let message = inference::AssistantMessage::from(message);

        // Then the tool call is available
        assert_eq!(message.tool_calls.unwrap().len(), 1);
    }

    #[cfg_attr(feature = "test_no_openai", ignore = "OpenAI tests disabled")]
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

        let result = <OpenAiClient as InferenceClient>::chat_v2(
            &client,
            &ChatRequest {
                model: NON_REASONING_MODEL.to_owned(),
                messages: vec![Message::user("When is order 123456 delivered?")],
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

    #[cfg_attr(feature = "test_no_openai", ignore = "OpenAI tests disabled")]
    #[tokio::test]
    async fn chat_message_conversion() {
        // Given an inference client
        let api_token = openai_token().to_owned();
        let host = openai_inference_url().to_owned();
        let client = OpenAiClient::new(host, api_token);

        let result = <OpenAiClient as InferenceClient>::chat_v2(
            &client,
            &ChatRequest {
                model: NON_REASONING_MODEL.to_owned(),
                messages: vec![Message::user("An apple a day")],
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

    #[cfg_attr(feature = "test_no_openai", ignore = "OpenAI tests disabled")]
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
        let result = <OpenAiClient as InferenceClient>::chat_v2(
            &client,
            &ChatRequest {
                model: NON_REASONING_MODEL.to_owned(),
                messages: vec![Message::user("An apple a day")],
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

    #[cfg_attr(feature = "test_no_openai", ignore = "OpenAI tests disabled")]
    #[tokio::test]
    async fn model_not_found() {
        // Given
        let api_token = openai_token().to_owned();
        let host = openai_inference_url().to_owned();
        let client = OpenAiClient::new(host, api_token);

        // When doing a chat request against a model that does not exist
        let result = <OpenAiClient as InferenceClient>::chat_v2(
            &client,
            &ChatRequest {
                model: "gpt-4o-mini-non-existent".to_owned(),
                messages: vec![Message::user("An apple a day")],
                params: ChatParams::default(),
            },
            Authentication::none(),
            &TracingContext::dummy(),
        )
        .await;

        // Then
        assert!(matches!(result, Err(InferenceError::ModelNotFound)));
    }

    #[cfg_attr(feature = "test_no_openai", ignore = "OpenAI tests disabled")]
    #[tokio::test]
    async fn bad_token_gives_unauthorized() {
        // Given a client with a bad token
        let api_token = "bad-token".to_owned();
        let host = openai_inference_url().to_owned();
        let client = OpenAiClient::new(host, api_token);

        // When doing a chat request
        let result = <OpenAiClient as InferenceClient>::chat_v2(
            &client,
            &ChatRequest {
                model: NON_REASONING_MODEL.to_owned(),
                messages: vec![Message::user("An apple a day")],
                params: ChatParams::default(),
            },
            Authentication::none(),
            &TracingContext::dummy(),
        )
        .await;

        // Then
        assert!(matches!(result, Err(InferenceError::Unauthorized)));
    }

    #[cfg_attr(feature = "test_no_openai", ignore = "OpenAI tests disabled")]
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
            <OpenAiClient as InferenceClient>::stream_chat_v2(
                &client,
                &ChatRequest {
                    model: NON_REASONING_MODEL.to_owned(),
                    messages: vec![Message::user("An apple a day")],
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
        assert_eq!(events.len(), 4);
        assert!(matches!(events[0], ChatEvent::MessageBegin { .. }));
        assert!(matches!(events[1], ChatEvent::MessageAppend { .. }));
        assert!(matches!(events[2], ChatEvent::MessageEnd { .. }));
        assert!(matches!(events[3], ChatEvent::Usage { .. }));
    }

    #[cfg_attr(feature = "test_no_openai", ignore = "OpenAI tests disabled")]
    #[tokio::test]
    async fn forced_tool_call() {
        // Given a message history that would not require a tool call
        let api_token = openai_token().to_owned();
        let host = openai_inference_url().to_owned();
        let client = OpenAiClient::new(host, api_token);
        let message = Message::user("What is the weather in Berlin?");

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
        let result = <OpenAiClient as InferenceClient>::chat_v2(
            &client,
            &ChatRequest {
                model: NON_REASONING_MODEL.to_owned(),
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

    #[cfg_attr(feature = "test_no_openai", ignore = "OpenAI tests disabled")]
    #[tokio::test]
    async fn streaming_tool_call() {
        // Given a message history that would lead to a tool call
        let api_token = openai_token().to_owned();
        let host = openai_inference_url().to_owned();
        let client = OpenAiClient::new(host, api_token);
        let message = Message::user("When will order 123456 be delivered?");

        // When streaming a tool call
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
        let params = ChatParams {
            tools: Some(vec![function]),
            ..Default::default()
        };

        let (send, mut recv) = mpsc::channel(1);
        tokio::spawn(async move {
            <OpenAiClient as InferenceClient>::stream_chat_v2(
                &client,
                &ChatRequest {
                    model: NON_REASONING_MODEL.to_owned(),
                    messages: vec![message],
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

        // Then the model calls the tool
        assert!(matches!(events[0], ChatEvent::MessageBegin { .. }));

        // And the first event contains the index, name and id
        if let ChatEvent::ToolCall(tool_calls) = &events[1] {
            assert_eq!(tool_calls.len(), 1);
            assert_eq!(tool_calls[0].index, 0);
            assert_eq!(tool_calls[0].name.as_ref().unwrap(), "get_delivery_date");
            assert!(!tool_calls[0].id.as_ref().unwrap().is_empty());
        } else {
            panic!("Expected a tool call event");
        }

        // And the second event contains the index and parts of the arguments
        if let ChatEvent::ToolCall(tool_calls) = &events[2] {
            assert_eq!(tool_calls.len(), 1);
            assert_eq!(tool_calls[0].index, 0);
            assert!(!tool_calls[0].arguments.as_ref().unwrap().is_empty());
        } else {
            panic!("Expected a tool call event");
        }

        assert!(matches!(
            events[events.len() - 2],
            ChatEvent::MessageEnd { .. }
        ));
        assert!(matches!(events[events.len() - 1], ChatEvent::Usage { .. }));
    }

    #[cfg_attr(feature = "test_no_openai", ignore = "OpenAI tests disabled")]
    #[tokio::test]
    async fn json_schema_response_format() {
        // Given a message history that would lead to a text response
        let api_token = openai_token().to_owned();
        let host = openai_inference_url().to_owned();
        let client = OpenAiClient::new(host, api_token);
        let message = Message::user("Hi, how are you?");

        // When forcing a json schema response format
        let response_format = ResponseFormat::JsonSchema(JsonSchema {
            name: "delivery_date".to_owned(),
            description: Some("Get the delivery date for a given order".to_owned()),
            schema: Some(
                serde_json::to_vec(&serde_json::json!({
                    "type": "object",
                    "properties": {
                        "order_id": { "type": "string" }
                    }
                }))
                .unwrap(),
            ),
            strict: None,
        });

        let params = ChatParams {
            response_format: Some(response_format),
            temperature: Some(0.0),
            ..Default::default()
        };

        let result = <OpenAiClient as InferenceClient>::chat_v2(
            &client,
            &ChatRequest {
                model: NON_REASONING_MODEL.to_owned(),
                messages: vec![message],
                params,
            },
            Authentication::none(),
            &TracingContext::dummy(),
        )
        .await
        .unwrap();

        // Then the model returns a json object
        let content = result.message.content.unwrap();
        let json: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(json["order_id"].is_string());
    }

    #[cfg_attr(feature = "test_no_openai", ignore = "OpenAI tests disabled")]
    #[tokio::test]
    async fn reasoning_model_expects_max_completion_tokens() {
        // Given a message history that would lead to a text response
        let api_token = openai_token().to_owned();
        let host = openai_inference_url().to_owned();
        let client = OpenAiClient::new(host, api_token);
        let message = Message::user("Hi, how are you?");

        let params = ChatParams {
            max_completion_tokens: Some(10),
            reasoning_effort: Some(ReasoningEffort::Low),
            ..Default::default()
        };

        let result = <OpenAiClient as InferenceClient>::chat_v2(
            &client,
            &ChatRequest {
                model: REASONING_MODEL.to_owned(),
                messages: vec![message],
                params,
            },
            Authentication::none(),
            &TracingContext::dummy(),
        )
        .await;

        // Then the model returns a text response
        assert!(result.is_ok());
    }
}
