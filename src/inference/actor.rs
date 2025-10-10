use derive_more::{Constructor, Deref, Display, IntoIterator};
use futures::{StreamExt, stream::FuturesUnordered};
use serde::Serialize;
use std::{future::Future, pin::Pin, str::FromStr, sync::Arc};
use tokio::{
    select,
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use crate::{
    authorization::Authentication, context, inference::client::AlephAlphaClient,
    logging::TracingContext,
};
use tracing::{info, warn};

use super::{
    client::{InferenceClient, InferenceError},
    openai::OpenAiClient,
};

#[cfg(test)]
use double_trait::double;

/// Provider configuration for the inference backend
pub enum InferenceProvider<'a> {
    /// Use the Aleph Alpha inference API.
    AlephAlpha { url: &'a str },
    /// Use any OpenAI-compatible inference. Only chat requests are supported, completion,
    /// explanation and chunking requests are not.
    OpenAi { url: &'a str, token: &'a str },
    /// Kernel is running without any inference backend. Inference requests will lead to a runtime
    /// error.
    None,
}

/// Configuration for the inference backend that is used
pub struct InferenceConfig<'a> {
    /// The inference provider to use
    pub provider: InferenceProvider<'a>,
    /// Whether to capture `GenAI` content (prompts/completions) in traces
    pub gen_ai_content_capture: bool,
}

/// Handle to the inference actor. Spin this up in order to use the inference API.
pub struct Inference {
    send: mpsc::Sender<InferenceMsg>,
    handle: JoinHandle<()>,
}

impl Inference {
    /// Starts a new inference Actor. Calls to this method be balanced by calls to
    /// [`Self::shutdown`].
    pub fn new(config: InferenceConfig<'_>) -> Self {
        match config.provider {
            InferenceProvider::AlephAlpha { url } => {
                info!(
                    target: "pharia-kernel::inference",
                    "Using Aleph Alpha Inference at {}", url
                );
                let client = AlephAlphaClient::new(url);
                Self::with_client(client, config.gen_ai_content_capture)
            }
            InferenceProvider::OpenAi { url, token } => {
                info!(
                    target: "pharia-kernel::inference",
                    "Using OpenAI Inference API at {}", url
                );
                let client = OpenAiClient::new(url, token);
                Self::with_client(client, config.gen_ai_content_capture)
            }
            InferenceProvider::None => {
                warn!(
                    target: "pharia-kernel::inference",
                    "No inference configured, running without inference capabilities."
                );
                Self::with_client(InferenceNotConfigured, config.gen_ai_content_capture)
            }
        }
    }

    pub fn with_client(client: impl InferenceClient, gen_ai_content_capture: bool) -> Self {
        let (send, recv) = tokio::sync::mpsc::channel::<InferenceMsg>(1);
        let mut actor = InferenceActor::new(client, recv, gen_ai_content_capture);
        let handle = tokio::spawn(async move { actor.run().await });
        Inference { send, handle }
    }

    pub fn api(&self) -> InferenceSender {
        InferenceSender(self.send.clone())
    }

    /// Inference is going to shutdown, as soon as the last instance of [`InferenceApi`] is dropped.
    pub async fn wait_for_shutdown(self) {
        drop(self.send);
        self.handle.await.unwrap();
    }
}

#[cfg_attr(test, double(InferenceApiDouble))]
pub trait InferenceApi {
    fn explain(
        &self,
        request: ExplanationRequest,
        auth: Authentication,
        tracing_context: TracingContext,
    ) -> impl Future<Output = Result<Explanation, InferenceError>> + Send;

    fn complete(
        &self,
        request: CompletionRequest,
        auth: Authentication,
        tracing_context: TracingContext,
    ) -> impl Future<Output = Result<Completion, InferenceError>> + Send;

    fn completion_stream(
        &self,
        request: CompletionRequest,
        auth: Authentication,
        tracing_context: TracingContext,
    ) -> impl Future<Output = mpsc::Receiver<Result<CompletionEvent, InferenceError>>> + Send;

    fn chat(
        &self,
        request: ChatRequest,
        auth: Authentication,
        tracing_context: TracingContext,
    ) -> impl Future<Output = Result<ChatResponse, InferenceError>> + Send;

    fn chat_stream(
        &self,
        request: ChatRequest,
        auth: Authentication,
        tracing_context: TracingContext,
    ) -> impl Future<Output = mpsc::Receiver<Result<ChatEvent, InferenceError>>> + Send;
}

impl InferenceApi for InferenceSender {
    async fn explain(
        &self,
        request: ExplanationRequest,
        auth: Authentication,
        tracing_context: TracingContext,
    ) -> Result<Explanation, InferenceError> {
        let (send, recv) = oneshot::channel();
        let msg = InferenceMsg::Explain {
            request,
            send,
            auth,
            tracing_context,
        };
        self.0
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        let explanation = recv
            .await
            .expect("sender must be alive when awaiting for answers")?;
        Ok(explanation)
    }

    async fn complete(
        &self,
        request: CompletionRequest,
        auth: Authentication,
        tracing_context: TracingContext,
    ) -> Result<Completion, InferenceError> {
        let (send, recv) = oneshot::channel();
        let msg = InferenceMsg::Complete {
            request,
            send,
            auth,
            tracing_context,
        };
        self.0
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        let completion = recv
            .await
            .expect("sender must be alive when awaiting for answers")?;
        Ok(completion)
    }

    async fn completion_stream(
        &self,
        request: CompletionRequest,
        auth: Authentication,
        tracing_context: TracingContext,
    ) -> mpsc::Receiver<Result<CompletionEvent, InferenceError>> {
        let (send, recv) = mpsc::channel(1);
        let msg = InferenceMsg::CompletionStream {
            request,
            send,
            auth,
            tracing_context,
        };
        self.0
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv
    }

    async fn chat(
        &self,
        request: ChatRequest,
        auth: Authentication,
        tracing_context: TracingContext,
    ) -> Result<ChatResponse, InferenceError> {
        let (send, recv) = oneshot::channel();
        let msg = InferenceMsg::Chat {
            request,
            send,
            auth,
            tracing_context,
        };
        self.0
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        let chat_response = recv
            .await
            .expect("sender must be alive when awaiting for answers")?;
        Ok(chat_response)
    }

    async fn chat_stream(
        &self,
        request: ChatRequest,
        auth: Authentication,
        tracing_context: TracingContext,
    ) -> mpsc::Receiver<Result<ChatEvent, InferenceError>> {
        let (send, recv) = mpsc::channel(1);
        let msg = InferenceMsg::ChatStream {
            request,
            send,
            auth,
            tracing_context,
        };
        self.0
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv
    }
}

/// An inference client that always returns runtime errors.
///
/// With the goal of supporting not only the Aleph Alphe inference API, but also configurable
/// OpenAI-compatible inference APIs, we start with a configuration option of no inference backend
/// at all.
pub struct InferenceNotConfigured;

impl InferenceClient for InferenceNotConfigured {
    async fn complete(
        &self,
        _request: &CompletionRequest,
        _auth: Authentication,
        _tracing_context: &TracingContext,
    ) -> Result<Completion, InferenceError> {
        Err(InferenceError::NotConfigured)
    }

    async fn stream_completion(
        &self,
        _request: &CompletionRequest,
        _auth: Authentication,
        _tracing_context: &TracingContext,
        _send: mpsc::Sender<CompletionEvent>,
    ) -> Result<(), InferenceError> {
        Err(InferenceError::NotConfigured)
    }

    async fn chat_with_reasoning(
        &self,
        _request: &ChatRequest,
        _auth: Authentication,
        _tracing_context: &TracingContext,
    ) -> Result<ChatResponse, InferenceError> {
        Err(InferenceError::NotConfigured)
    }

    async fn stream_chat_with_reasoning(
        &self,
        _request: &ChatRequest,
        _auth: Authentication,
        _tracing_context: &TracingContext,
        _send: mpsc::Sender<ChatEvent>,
    ) -> Result<(), InferenceError> {
        Err(InferenceError::NotConfigured)
    }

    async fn explain(
        &self,
        _request: &ExplanationRequest,
        _auth: Authentication,
        _tracing_context: &TracingContext,
    ) -> Result<Explanation, InferenceError> {
        Err(InferenceError::NotConfigured)
    }
}

/// At which granularity should the target be explained in terms of the prompt.
/// If you choose, for example, [`Granularity::Sentence`] then we report the importance score of each
/// sentence in the prompt towards generating the target output.
/// The default is [`Granularity::Auto`] which means we will try to find the granularity that
/// brings you closest to around 30 explanations. For large prompts, this would likely
/// be sentences. For short prompts this might be individual words or even tokens.
#[derive(Display, PartialEq, Debug)]
pub enum Granularity {
    /// Let the system decide which granularity is most suitable for the given input.
    Auto,
    Word,
    Sentence,
    Paragraph,
}

pub struct ExplanationRequest {
    /// The prompt that typically was the input of a previous completion request
    pub prompt: String,
    /// The target string that should be explained. The influence of individual parts
    /// of the prompt for generating this target string will be indicated in the response.
    pub target: String,
    pub model: String,
    /// The granularity of the parts of the prompt for which a single
    /// score is computed.
    pub granularity: Granularity,
}

#[derive(Debug)]
pub struct TextScore {
    pub start: u32,
    pub length: u32,
    pub score: f64,
}

/// While `[aleph_alpha_client::ExplanationOutput]` contains multiple items for `Text`, `Image`, and `Target`,
/// we do not support multi-modal prompts and do not return any scores for `Image`.
/// As we also do not support target-granularity as part of the `[crate::ExplanationRequest]`, we will get
/// an empty vector in the target scores, and therefore can ignore these one as well.
/// Explanation then becomes a wrapper around the `TextScore` vector for the text item.
#[derive(Deref, Constructor, Debug, IntoIterator)]
pub struct Explanation(Vec<TextScore>);

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub enum Logprobs {
    /// Do not return any logprobs
    #[default]
    No,
    /// Return only the logprob of the tokens which have actually been sampled into the completion.
    Sampled,
    /// Request between 0 and 20 tokens
    Top(u8),
}

#[derive(Default, Debug, PartialEq, Clone)]
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
    pub echo: bool,
}

#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub prompt: String,
    pub model: String,
    pub params: CompletionParams,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub name: String,
    pub description: Option<String>,
    // While it would be nice to specify here already in the type system that this must be a JSON
    // value, this would require the possibility for a non-happy path when converting from the
    // bindings in the `wasm` module, which we do not have.
    pub parameters: Option<Vec<u8>>,
    pub strict: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReasoningEffort {
    Minimal,
    Low,
    Medium,
    High,
}

#[derive(Debug, Default, PartialEq)]
pub struct ChatParams {
    pub max_tokens: Option<u32>,
    pub max_completion_tokens: Option<u32>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub frequency_penalty: Option<f64>,
    pub presence_penalty: Option<f64>,
    pub logprobs: Logprobs,
    pub tools: Option<Vec<Function>>,
    pub tool_choice: Option<ToolChoice>,
    pub parallel_tool_calls: Option<bool>,
    pub response_format: Option<ResponseFormat>,
    pub reasoning_effort: Option<ReasoningEffort>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResponseFormat {
    Text,
    JsonObject,
    JsonSchema(JsonSchema),
}

#[derive(Debug, Clone, PartialEq)]
pub struct JsonSchema {
    pub name: String,
    pub description: Option<String>,
    pub schema: Option<Vec<u8>>,
    pub strict: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct AssistantMessage {
    pub content: Option<String>,
    pub reasoning_content: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
}

impl AssistantMessage {
    pub fn as_otel_message(&self) -> OTelMessage {
        OTelMessage {
            role: "assistant".to_owned(),
            content: self.content.clone().unwrap_or_default(),
        }
    }
}

impl AssistantMessage {
    pub fn role() -> &'static str {
        "assistant"
    }
}

/// Merge reasoning content with the assistant message content.
///
/// When switching to v2/completions, we will get a stronger typed response from inference that
/// distinguishes between content and reasoning content. However, older wit world do not know
/// about reasoning, so we need to merge the content and reasoning content into a single string.
///
/// A known limitation is that we take assumptions about the thinking tokens of the model (qwen syntax).
/// However, since reasoning models have not been available in the Aleph Alpha API for a long time, it
/// is reasonable to assume that the only reasoning model used inside skills is qwen (if even).
pub fn prepend_reasoning_content(content: String, reasoning_content: Option<String>) -> String {
    if let Some(reasoning_content) = reasoning_content {
        format!("<think>{reasoning_content}</think>{content}")
    } else {
        content
    }
}

#[derive(Debug, Clone)]
pub struct ToolMessage {
    pub content: String,
    pub tool_call_id: String,
}

#[derive(Debug, Clone)]
pub enum Message {
    Assistant(AssistantMessage),
    Tool(ToolMessage),
    // Would be either system, developer or user, others are not supported by OpenAI and the
    // client library. We do not have stronger typing for the role, as we do not have a good way
    // to send errors (and stop skill execution) where the Message is constructed from the WIT
    // bindings.
    Other { role: String, content: String },
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self::Other {
            role: "system".to_owned(),
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::Other {
            role: "user".to_owned(),
            content: content.into(),
        }
    }

    pub fn as_otel_message(&self) -> OTelMessage {
        match self {
            Self::Assistant(message) => OTelMessage {
                role: "assistant".to_owned(),
                content: message.content.clone().unwrap_or_default(),
            },
            Self::Tool(message) => OTelMessage {
                role: "tool".to_owned(),
                content: message.content.clone(),
            },
            Self::Other { role, content } => OTelMessage {
                role: role.clone(),
                content: content.clone(),
            },
        }
    }
}

/// Message format expected by the `OTel GenAI` semantic convention.
/// See: <https://opentelemetry.io/docs/specs/semconv/gen-ai/gen-ai-agent-spans/>
#[derive(Serialize)]
pub struct OTelMessage {
    role: String,
    content: String,
}

/// A tool call as requested by the model.
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    /// The name of the function to call.
    pub name: String,
    /// The arguments to call the function with, as generated by the model in JSON format.
    /// Note that the model does not always generate valid JSON, and may hallucinate parameters not
    /// defined by your function schema.
    /// Validate the arguments in your code before calling your function.
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolChoice {
    None,
    Auto,
    Required,
    Named(String),
}

pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub params: ChatParams,
}

impl CompletionRequest {
    pub fn new(prompt: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            model: model.into(),
            params: CompletionParams::default(),
        }
    }

    pub fn with_params(mut self, params: CompletionParams) -> Self {
        self.params = params;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishReason {
    Stop,
    Length,
    ContentFilter,
    ToolCalls,
}

/// Parse a finish reason from a string.
///
/// This implementation is only used for completion responses. In case the Aleph Alpha API adds the
/// [`FinishReason::ToolCalls`] variant for completion responses, we must NOT simply add it here,
/// because when mapping this type to the variants of the different WIT worlds, we map the
/// [`FinishReason::ToolCalls`] variant to an unreachable code path.
impl FromStr for FinishReason {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> anyhow::Result<Self> {
        match s {
            "length" | "maximum_tokens" => Ok(Self::Length),
            "stop" | "end_of_text" | "stop_sequence_reached" => Ok(Self::Stop),
            "content_filter" => Ok(Self::ContentFilter),
            _ => Err(anyhow::anyhow!("Unknown finish reason: {}", s)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenUsage {
    pub prompt: u32,
    pub completion: u32,
}

#[derive(Debug, Clone)]
pub struct Completion {
    pub text: String,
    pub finish_reason: FinishReason,
    /// Contains the logprobs for the sampled and top n tokens, given that [`crate::Logprobs`] has
    /// been set to [`crate::Logprobs::Sampled`] or [`crate::Logprobs::Top`].
    pub logprobs: Vec<Distribution>,
    pub usage: TokenUsage,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CompletionEvent {
    Append {
        text: String,
        logprobs: Vec<Distribution>,
    },
    End {
        finish_reason: FinishReason,
    },
    Usage {
        usage: TokenUsage,
    },
}

#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub message: AssistantMessage,
    pub finish_reason: FinishReason,
    /// Contains the logprobs for the sampled and top n tokens, given that [`crate::Logprobs`] has
    /// been set to [`crate::Logprobs::Sampled`] or [`crate::Logprobs::Top`].
    pub logprobs: Vec<Distribution>,
    pub usage: TokenUsage,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ChatEvent {
    MessageBegin {
        role: String,
    },
    MessageAppend {
        /// Chat completion chunk generated by the model when streaming is enabled.
        /// The role is always "assistant".
        content: String,
        /// Log probabilities of the completion tokens if requested via logprobs parameter in request.
        logprobs: Vec<Distribution>,
    },
    /// The last chunk of a chat completion stream.
    MessageEnd {
        /// The reason the model stopped generating tokens.
        finish_reason: FinishReason,
    },
    /// Summary of the chat completion stream.
    Usage {
        usage: TokenUsage,
    },
    ToolCall(Vec<ToolCallChunk>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCallChunk {
    /// The index of the tool call in the list of tool calls.
    pub index: u32,
    /// The ID of the tool call.
    pub id: Option<String>,
    /// The name of the tool to call.
    pub name: Option<String>,
    /// The arguments to call the function with, as generated by the model in JSON format.
    pub arguments: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Distribution {
    // Logarithmic probability of the token returned in the completion
    pub sampled: Logprob,
    // Logarithmic probabilities of the most probable tokens, filled if user has requested [`crate::Logprobs::Top`]
    pub top: Vec<Logprob>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Logprob {
    pub token: Vec<u8>,
    pub logprob: f64,
}

/// Private implementation of the inference actor running in its own dedicated green thread.
struct InferenceActor<C: InferenceClient> {
    client: Arc<C>,
    recv: mpsc::Receiver<InferenceMsg>,
    running_requests: FuturesUnordered<Pin<Box<dyn Future<Output = ()> + Send>>>,
    /// Whether to capture `GenAI` content (prompts/completions) in traces
    gen_ai_content_capture: bool,
}

impl<C: InferenceClient> InferenceActor<C> {
    fn new(client: C, recv: mpsc::Receiver<InferenceMsg>, gen_ai_content_capture: bool) -> Self {
        InferenceActor {
            client: Arc::new(client),
            recv,
            running_requests: FuturesUnordered::new(),
            gen_ai_content_capture,
        }
    }

    async fn run(&mut self) {
        loop {
            // While there are messages and completions, poll both.
            // If there is a message, add it to the queue.
            // If there are completions, make progress on them.
            select! {
                msg = self.recv.recv() => match msg {
                    Some(msg) => self.act(msg),
                    // Senders are gone, break out of the loop for shutdown.
                    None => break
                },
                // FuturesUnordered will let them run in parallel. It will
                // yield once one of them is completed.
                () = self.running_requests.select_next_some(), if !self.running_requests.is_empty()  => {}
            };
        }
    }

    fn act(&mut self, msg: InferenceMsg) {
        let client = self.client.clone();
        let gen_ai_content_capture = self.gen_ai_content_capture;
        self.running_requests.push(Box::pin(async move {
            msg.act(client.as_ref(), gen_ai_content_capture).await;
        }));
    }
}

/// Opaque wrapper around a sender to the inference actor, so we do not need to expose our message
/// type.
#[derive(Clone)]
pub struct InferenceSender(mpsc::Sender<InferenceMsg>);

enum InferenceMsg {
    Complete {
        request: CompletionRequest,
        send: oneshot::Sender<Result<Completion, InferenceError>>,
        auth: Authentication,
        tracing_context: TracingContext,
    },
    CompletionStream {
        request: CompletionRequest,
        send: mpsc::Sender<Result<CompletionEvent, InferenceError>>,
        auth: Authentication,
        tracing_context: TracingContext,
    },
    Chat {
        request: ChatRequest,
        send: oneshot::Sender<Result<ChatResponse, InferenceError>>,
        auth: Authentication,
        tracing_context: TracingContext,
    },
    ChatStream {
        request: ChatRequest,
        send: mpsc::Sender<Result<ChatEvent, InferenceError>>,
        auth: Authentication,
        tracing_context: TracingContext,
    },
    Explain {
        request: ExplanationRequest,
        send: oneshot::Sender<Result<Explanation, InferenceError>>,
        auth: Authentication,
        tracing_context: TracingContext,
    },
}

impl InferenceMsg {
    #[allow(clippy::too_many_lines)]
    async fn act(self, client: &impl InferenceClient, gen_ai_content_capture: bool) {
        match self {
            Self::Complete {
                request,
                send,
                auth,
                tracing_context,
            } => {
                let context = tracing_context.child_from_completion_request(&request);
                let result = client.complete(&request, auth, &context).await;
                drop(send.send(result));
            }
            Self::CompletionStream {
                request,
                send,
                auth,
                tracing_context,
            } => {
                let context = tracing_context.child_from_completion_request(&request);
                let (event_send, mut event_recv) = mpsc::channel(1);
                let mut stream =
                    Box::pin(client.stream_completion(&request, auth, &context, event_send));

                loop {
                    // Pass along messages that we get from the stream while also checking if we get an error
                    select! {
                        // Pull from receiver as long as there are still senders
                        Some(msg) = event_recv.recv(), if !event_recv.is_closed() =>  {
                            let Ok(()) = send.send(Ok(msg)).await else {
                                // The receiver is dropped so we can stop polling the stream.
                                break;
                            };
                        },
                        result = &mut stream =>  {
                            if let Err(err) = result {
                                drop(send.send(Err(err)).await);
                            }
                            // Break out of the loop once the stream is done
                            break;
                        }
                    };
                }

                // Finish sending through any remaining messages
                while let Some(msg) = event_recv.recv().await {
                    drop(send.send(Ok(msg)).await);
                }
            }
            Self::Chat {
                request,
                send,
                auth,
                tracing_context,
            } => {
                let context = tracing_context.child_from_chat_request(&request);
                if gen_ai_content_capture {
                    context.capture_input_messages(&request.messages);
                }
                let result = client.chat_with_reasoning(&request, auth, &context).await;
                if let Ok(response) = &result
                    && gen_ai_content_capture
                {
                    context.capture_output_message(&response.message);
                }
                drop(send.send(result));
            }
            Self::ChatStream {
                request,
                send,
                auth,
                tracing_context,
            } => {
                let context = tracing_context.child_from_chat_request(&request);
                if gen_ai_content_capture {
                    context.capture_input_messages(&request.messages);
                }
                let (event_send, mut event_recv) = mpsc::channel(1);
                let mut stream = Box::pin(client.stream_chat_with_reasoning(&request, auth, &context, event_send));

                // Reconstruct the entire assistant message from the stream.
                let mut span_content = String::new();

                loop {
                    // Pass along messages that we get from the stream while also checking if we get an error
                    select! {
                        // Pull from receiver as long as there are still senders
                        Some(msg) = event_recv.recv(), if !event_recv.is_closed() =>  {
                            if gen_ai_content_capture &&let ChatEvent::MessageAppend { content, .. } = &msg {
                                span_content.push_str(content);
                            }
                            let Ok(()) = send.send(Ok(msg)).await else {
                                // The receiver is dropped so we can stop polling the stream.
                                break;
                            };
                        },
                        result = &mut stream =>  {
                            if let Err(err) = result {
                                drop(send.send(Err(err)).await);
                            }
                            // Break out of the loop once the stream is done
                            break;
                        }
                    };
                }

                // Finish sending through any remaining messages
                while let Some(msg) = event_recv.recv().await {
                    drop(send.send(Ok(msg)).await);
                }

                if gen_ai_content_capture {
                    let message = AssistantMessage {
                        content: Some(span_content),
                        reasoning_content: None,
                        tool_calls: None,
                    };
                    context.capture_output_message(&message);
                }
            }
            Self::Explain {
                request,
                send,
                auth,
                tracing_context,
            } => {
                let context = context!(tracing_context, "pharia-kernel::inference", "explain");
                let result = client.explain(&request, auth, &context).await;
                drop(send.send(result));
            }
        }
    }
}

#[cfg(test)]
pub mod tests {
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    use anyhow::anyhow;
    use tokio::{time::sleep, try_join};

    use crate::inference::client::{InferenceClientDouble, InferenceError};

    use super::*;

    impl AssistantMessage {
        pub fn dummy() -> Self {
            Self {
                content: Some("dummy-content".to_string()),
                reasoning_content: None,
                tool_calls: None,
            }
        }
    }

    impl Completion {
        pub fn from_text(completion: impl Into<String>) -> Self {
            Self {
                text: completion.into(),
                finish_reason: FinishReason::Stop,
                logprobs: vec![],
                usage: TokenUsage {
                    prompt: 0,
                    completion: 0,
                },
            }
        }
    }

    pub struct InferenceStub {
        complete:
            Box<dyn Fn(CompletionRequest) -> Result<Completion, InferenceError> + Send + Sync>,
    }

    impl InferenceStub {
        pub fn new() -> Self {
            Self {
                complete: Box::new(|_| Err(InferenceError::Other(anyhow!("Not implemented")))),
            }
        }

        pub fn with_complete(
            mut self,
            complete: impl Fn(CompletionRequest) -> Result<Completion, InferenceError>
            + Send
            + Sync
            + 'static,
        ) -> Self {
            self.complete = Box::new(complete);
            self
        }
    }

    impl InferenceApiDouble for InferenceStub {
        async fn complete(
            &self,
            request: CompletionRequest,
            _auth: Authentication,
            _tracing_context: TracingContext,
        ) -> Result<Completion, InferenceError> {
            let completion = (self.complete)(request)?;
            Ok(completion)
        }
    }

    struct SaboteurClient {
        remaining_failures: AtomicUsize,
    }

    impl SaboteurClient {
        fn new(remaining_failures: usize) -> Self {
            Self {
                remaining_failures: remaining_failures.into(),
            }
        }
    }

    impl InferenceClientDouble for SaboteurClient {
        async fn complete(
            &self,
            _params: &super::CompletionRequest,
            _auth: Authentication,
            _tracing_context: &TracingContext,
        ) -> Result<Completion, InferenceError> {
            let remaining = self
                .remaining_failures
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |f| {
                    Some(f.saturating_sub(1))
                })
                .unwrap();

            if remaining == 0 {
                Err(InferenceError::Other(anyhow!("Inference error")))
            } else {
                Ok(Completion::from_text("Completion succeeded"))
            }
        }
    }

    #[tokio::test]
    async fn recover_from_connection_loss() {
        // given
        let client = SaboteurClient::new(2);
        let inference = Inference::with_client(client, false);
        let inference_api = inference.api();
        let request = CompletionRequest {
            prompt: "dummy_prompt".to_owned(),
            model: "dummy_model".to_owned(),
            params: CompletionParams::default(),
        };

        // when
        let result = inference_api
            .complete(request, Authentication::none(), TracingContext::dummy())
            .await;

        // then
        assert!(result.is_ok());
    }

    /// Dummy complete text params for test, if you do not care particular about them.
    fn complete_text_params_dummy() -> CompletionRequest {
        CompletionRequest::new("Dummy prompt".to_owned(), "Dummy model name".to_owned())
    }

    /// This Client will only resolve a completion once the correct number of
    /// requests have been reached.
    struct AssertConcurrentClient {
        /// Number of requests we are still waiting on
        expected_concurrent_requests: AtomicUsize,
    }

    impl AssertConcurrentClient {
        pub fn new(expected_concurrent_requests: impl Into<AtomicUsize>) -> Self {
            Self {
                expected_concurrent_requests: expected_concurrent_requests.into(),
            }
        }
    }

    impl InferenceClientDouble for AssertConcurrentClient {
        async fn complete(
            &self,
            request: &CompletionRequest,
            _auth: Authentication,
            _tracing_context: &TracingContext,
        ) -> Result<Completion, InferenceError> {
            self.expected_concurrent_requests
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |e| {
                    Some(e.saturating_sub(1))
                })
                .unwrap();

            while self.expected_concurrent_requests.load(Ordering::SeqCst) > 0 {
                sleep(Duration::from_millis(1)).await;
            }
            Ok(Completion::from_text(&request.prompt))
        }
    }

    /// We want to ensure that the actor invokes the client multiple times concurrently instead of
    /// only one inference request at a time.
    #[tokio::test(start_paused = true)]
    async fn concurrent_invocation_of_client() {
        // Given
        let client = AssertConcurrentClient::new(2);
        let inference = Inference::with_client(client, false);
        let api = inference.api();

        // When
        // Schedule two tasks
        let resp = try_join!(
            api.complete(
                complete_text_params_dummy(),
                Authentication::none(),
                TracingContext::dummy()
            ),
            api.complete(
                complete_text_params_dummy(),
                Authentication::none(),
                TracingContext::dummy()
            )
        );

        // Then: Both run concurrently and only return once both are completed.
        assert!(resp.is_ok());

        // We need to drop the sender in order for `actor.run` to terminate
        drop(api);
        inference.wait_for_shutdown().await;
    }
}
