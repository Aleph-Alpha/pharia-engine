use aleph_alpha_client::Client;
use derive_more::{Constructor, Deref, Display, IntoIterator};
use futures::{StreamExt, stream::FuturesUnordered};
use std::{future::Future, pin::Pin, str::FromStr, sync::Arc};
use tokio::{
    select,
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use super::client::InferenceClient;

/// Handle to the inference actor. Spin this up in order to use the inference API.
pub struct Inference {
    send: mpsc::Sender<InferenceMessage>,
    handle: JoinHandle<()>,
}

impl Inference {
    /// Starts a new inference Actor. Calls to this method be balanced by calls to
    /// [`Self::shutdown`].
    pub fn new(inference_addr: String) -> Self {
        let client = Client::new(inference_addr, None).unwrap();
        Self::with_client(client)
    }

    pub fn with_client(client: impl InferenceClient) -> Self {
        let (send, recv) = tokio::sync::mpsc::channel::<InferenceMessage>(1);
        let mut actor = InferenceActor::new(client, recv);
        let handle = tokio::spawn(async move { actor.run().await });
        Inference { send, handle }
    }

    pub fn api(&self) -> mpsc::Sender<InferenceMessage> {
        self.send.clone()
    }

    /// Inference is going to shutdown, as soon as the last instance of [`InferenceApi`] is dropped.
    pub async fn wait_for_shutdown(self) {
        drop(self.send);
        self.handle.await.unwrap();
    }
}

pub trait InferenceApi {
    fn explain(
        &self,
        request: ExplanationRequest,
        api_token: String,
    ) -> impl Future<Output = anyhow::Result<Explanation>> + Send;

    fn complete(
        &self,
        request: CompletionRequest,
        api_token: String,
    ) -> impl Future<Output = anyhow::Result<Completion>> + Send;

    fn completion_stream(
        &self,
        request: CompletionRequest,
        api_token: String,
    ) -> impl Future<Output = mpsc::Receiver<anyhow::Result<CompletionEvent>>> + Send;

    fn chat(
        &self,
        request: ChatRequest,
        api_token: String,
    ) -> impl Future<Output = anyhow::Result<ChatResponse>> + Send;

    fn chat_stream(
        &self,
        request: ChatRequest,
        api_token: String,
    ) -> impl Future<Output = mpsc::Receiver<anyhow::Result<ChatEvent>>> + Send;
}

impl InferenceApi for mpsc::Sender<InferenceMessage> {
    async fn explain(
        &self,
        request: ExplanationRequest,
        api_token: String,
    ) -> anyhow::Result<Explanation> {
        let (send, recv) = oneshot::channel();
        let msg = InferenceMessage::Explain {
            request,
            send,
            api_token,
        };
        self.send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await
            .expect("sender must be alive when awaiting for answers")
    }

    async fn complete(
        &self,
        request: CompletionRequest,
        api_token: String,
    ) -> anyhow::Result<Completion> {
        let (send, recv) = oneshot::channel();
        let msg = InferenceMessage::Complete {
            request,
            send,
            api_token,
        };
        self.send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await
            .expect("sender must be alive when awaiting for answers")
    }

    async fn completion_stream(
        &self,
        request: CompletionRequest,
        api_token: String,
    ) -> mpsc::Receiver<anyhow::Result<CompletionEvent>> {
        let (send, recv) = mpsc::channel(1);
        let msg = InferenceMessage::CompletionStream {
            request,
            send,
            api_token,
        };
        self.send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv
    }

    async fn chat(&self, request: ChatRequest, api_token: String) -> anyhow::Result<ChatResponse> {
        let (send, recv) = oneshot::channel();
        let msg = InferenceMessage::Chat {
            request,
            send,
            api_token,
        };
        self.send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await
            .expect("sender must be alive when awaiting for answers")
    }

    async fn chat_stream(
        &self,
        request: ChatRequest,
        api_token: String,
    ) -> mpsc::Receiver<anyhow::Result<ChatEvent>> {
        let (send, recv) = mpsc::channel(1);
        let msg = InferenceMessage::ChatStream {
            request,
            send,
            api_token,
        };
        self.send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv
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
}

#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub prompt: String,
    pub model: String,
    pub params: CompletionParams,
}

#[derive(Debug, Default, PartialEq)]
pub struct ChatParams {
    pub max_tokens: Option<u32>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub frequency_penalty: Option<f64>,
    pub presence_penalty: Option<f64>,
    pub logprobs: Logprobs,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub role: String,
    pub content: String,
}

impl Message {
    pub fn new(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
        }
    }
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
}

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
    pub message: Message,
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
    recv: mpsc::Receiver<InferenceMessage>,
    running_requests: FuturesUnordered<Pin<Box<dyn Future<Output = ()> + Send>>>,
}

impl<C: InferenceClient> InferenceActor<C> {
    fn new(client: C, recv: mpsc::Receiver<InferenceMessage>) -> Self {
        InferenceActor {
            client: Arc::new(client),
            recv,
            running_requests: FuturesUnordered::new(),
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

    fn act(&mut self, msg: InferenceMessage) {
        let client = self.client.clone();
        self.running_requests.push(Box::pin(async move {
            msg.act(client.as_ref()).await;
        }));
    }
}

pub enum InferenceMessage {
    Complete {
        request: CompletionRequest,
        send: oneshot::Sender<anyhow::Result<Completion>>,
        api_token: String,
    },
    CompletionStream {
        request: CompletionRequest,
        send: mpsc::Sender<anyhow::Result<CompletionEvent>>,
        api_token: String,
    },
    Chat {
        request: ChatRequest,
        send: oneshot::Sender<anyhow::Result<ChatResponse>>,
        api_token: String,
    },
    ChatStream {
        request: ChatRequest,
        send: mpsc::Sender<anyhow::Result<ChatEvent>>,
        api_token: String,
    },
    Explain {
        request: ExplanationRequest,
        send: oneshot::Sender<anyhow::Result<Explanation>>,
        api_token: String,
    },
}

impl InferenceMessage {
    async fn act(self, client: &impl InferenceClient) {
        match self {
            Self::Complete {
                request,
                send,
                api_token,
            } => {
                let result = client.complete(&request, api_token.clone()).await;
                drop(send.send(result.map_err(Into::into)));
            }
            Self::CompletionStream {
                request,
                send,
                api_token,
            } => {
                let (event_send, mut event_recv) = mpsc::channel(1);
                let mut stream =
                    Box::pin(client.stream_completion(&request, api_token, event_send));

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
                                drop(send.send(Err(err.into())).await);
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
                api_token,
            } => {
                let result = client.chat(&request, api_token.clone()).await;
                drop(send.send(result.map_err(Into::into)));
            }
            Self::ChatStream {
                request,
                send,
                api_token,
            } => {
                let (event_send, mut event_recv) = mpsc::channel(1);
                let mut stream = Box::pin(client.stream_chat(&request, api_token, event_send));

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
                                drop(send.send(Err(err.into())).await);
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
            Self::Explain {
                request,
                send,
                api_token,
            } => {
                let result = client.explain(&request, api_token.clone()).await;
                drop(send.send(result.map_err(Into::into)));
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
    use tokio::{sync::mpsc, time::sleep, try_join};

    use crate::inference::client::{InferenceClient, InferenceClientError};

    use super::*;

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
        complete: Box<dyn Fn(CompletionRequest) -> anyhow::Result<Completion> + Send + Sync>,
        chat: Box<dyn Fn(ChatRequest) -> anyhow::Result<ChatResponse> + Send + Sync>,
    }

    impl InferenceStub {
        pub fn new() -> Self {
            Self {
                complete: Box::new(|_| Err(anyhow::anyhow!("Not implemented"))),
                chat: Box::new(|_| Err(anyhow::anyhow!("Not implemented"))),
            }
        }

        pub fn with_complete(
            mut self,
            complete: impl Fn(CompletionRequest) -> anyhow::Result<Completion> + Send + Sync + 'static,
        ) -> Self {
            self.complete = Box::new(complete);
            self
        }

        pub fn with_chat(
            mut self,
            chat: impl Fn(ChatRequest) -> anyhow::Result<ChatResponse> + Send + Sync + 'static,
        ) -> Self {
            self.chat = Box::new(chat);
            self
        }
    }

    impl InferenceApi for InferenceStub {
        async fn explain(
            &self,
            _request: ExplanationRequest,
            _api_token: String,
        ) -> anyhow::Result<Explanation> {
            unimplemented!()
        }

        async fn complete(
            &self,
            request: CompletionRequest,
            _api_token: String,
        ) -> anyhow::Result<Completion> {
            (self.complete)(request)
        }

        async fn completion_stream(
            &self,
            request: CompletionRequest,
            _api_token: String,
        ) -> mpsc::Receiver<anyhow::Result<CompletionEvent>> {
            let (send, recv) = mpsc::channel(3);
            // Load up the receiver with events before returning it
            match (self.complete)(request) {
                Ok(Completion {
                    text,
                    finish_reason,
                    logprobs,
                    usage,
                }) => {
                    send.send(Ok(CompletionEvent::Append { text, logprobs }))
                        .await
                        .unwrap();
                    send.send(Ok(CompletionEvent::End { finish_reason }))
                        .await
                        .unwrap();
                    send.send(Ok(CompletionEvent::Usage { usage }))
                        .await
                        .unwrap();
                }
                Err(e) => {
                    send.send(Err(e)).await.unwrap();
                }
            };
            recv
        }

        async fn chat(
            &self,
            request: ChatRequest,
            _api_token: String,
        ) -> anyhow::Result<ChatResponse> {
            (self.chat)(request)
        }

        async fn chat_stream(
            &self,
            request: ChatRequest,
            _api_token: String,
        ) -> mpsc::Receiver<anyhow::Result<ChatEvent>> {
            let (send, recv) = mpsc::channel(4);
            // Load up the receiver with events before returning it
            match (self.chat)(request) {
                Ok(ChatResponse {
                    message,
                    finish_reason,
                    logprobs,
                    usage,
                }) => {
                    send.send(Ok(ChatEvent::MessageBegin { role: message.role }))
                        .await
                        .unwrap();
                    send.send(Ok(ChatEvent::MessageAppend {
                        content: message.content,
                        logprobs,
                    }))
                    .await
                    .unwrap();
                    send.send(Ok(ChatEvent::MessageEnd { finish_reason }))
                        .await
                        .unwrap();
                    send.send(Ok(ChatEvent::Usage { usage })).await.unwrap();
                }
                Err(e) => {
                    send.send(Err(e)).await.unwrap();
                }
            };
            recv
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

    impl InferenceClient for SaboteurClient {
        async fn explain(
            &self,
            _request: &ExplanationRequest,
            _api_token: String,
        ) -> Result<Explanation, InferenceClientError> {
            unimplemented!()
        }
        async fn complete(
            &self,
            _params: &super::CompletionRequest,
            _api_token: String,
        ) -> Result<Completion, InferenceClientError> {
            let remaining = self
                .remaining_failures
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |f| {
                    Some(f.saturating_sub(1))
                })
                .unwrap();

            if remaining == 0 {
                Err(InferenceClientError::Other(anyhow!("Inference error")))
            } else {
                Ok(Completion::from_text("Completion succeeded"))
            }
        }
        async fn stream_completion(
            &self,
            _request: &CompletionRequest,
            _api_token: String,
            _send: mpsc::Sender<CompletionEvent>,
        ) -> Result<(), InferenceClientError> {
            unimplemented!()
        }
        async fn chat(
            &self,
            _request: &ChatRequest,
            _api_token: String,
        ) -> Result<ChatResponse, InferenceClientError> {
            unimplemented!()
        }
        async fn stream_chat(
            &self,
            _request: &ChatRequest,
            _api_token: String,
            _send: mpsc::Sender<ChatEvent>,
        ) -> Result<(), InferenceClientError> {
            unimplemented!()
        }
    }

    #[tokio::test]
    async fn recover_from_connection_loss() {
        // given
        let client = SaboteurClient::new(2);
        let inference = Inference::with_client(client);
        let inference_api = inference.api();
        let request = CompletionRequest {
            prompt: "dummy_prompt".to_owned(),
            model: "dummy_model".to_owned(),
            params: CompletionParams::default(),
        };

        // when
        let result = inference_api
            .complete(request, "dummy_api".to_owned())
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

    impl InferenceClient for AssertConcurrentClient {
        async fn explain(
            &self,
            _request: &ExplanationRequest,
            _api_token: String,
        ) -> Result<Explanation, InferenceClientError> {
            unimplemented!()
        }
        async fn complete(
            &self,
            request: &CompletionRequest,
            _api_token: String,
        ) -> Result<Completion, InferenceClientError> {
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

        async fn stream_completion(
            &self,
            _request: &CompletionRequest,
            _api_token: String,
            _send: mpsc::Sender<CompletionEvent>,
        ) -> Result<(), InferenceClientError> {
            unimplemented!()
        }

        async fn chat(
            &self,
            _request: &ChatRequest,
            _api_token: String,
        ) -> Result<ChatResponse, InferenceClientError> {
            unimplemented!()
        }

        async fn stream_chat(
            &self,
            _request: &ChatRequest,
            _api_token: String,
            _send: mpsc::Sender<ChatEvent>,
        ) -> Result<(), InferenceClientError> {
            unimplemented!()
        }
    }

    /// We want to ensure that the actor invokes the client multiple times concurrently instead of
    /// only one inference request at a time.
    #[tokio::test(start_paused = true)]
    async fn concurrent_invocation_of_client() {
        // Given
        let client = AssertConcurrentClient::new(2);
        let inference = Inference::with_client(client);
        let api = inference.api();

        // When
        // Schedule two tasks
        let resp = try_join!(
            api.complete(complete_text_params_dummy(), "0".to_owned()),
            api.complete(complete_text_params_dummy(), "1".to_owned())
        );

        // Then: Both run concurrently and only return once both are completed.
        assert!(resp.is_ok());

        // We need to drop the sender in order for `actor.run` to terminate
        drop(api);
        inference.wait_for_shutdown().await;
    }
}
