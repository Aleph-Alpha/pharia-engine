use std::{future::Future, pin::Pin, str::FromStr, sync::Arc};

use aleph_alpha_client::Client;
use futures::{stream::FuturesUnordered, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::{
    select,
    sync::{mpsc, oneshot},
    task::JoinHandle,
};
use tracing::{error, warn};

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

    pub fn with_client(client: impl InferenceClient + Send + Sync + 'static) -> Self {
        let (send, recv) = tokio::sync::mpsc::channel::<InferenceMessage>(1);
        let mut actor = InferenceActor::new(client, recv);
        let handle = tokio::spawn(async move { actor.run().await });
        Inference { send, handle }
    }

    pub fn api(&self) -> InferenceApi {
        InferenceApi::new(self.send.clone())
    }

    /// Inference is going to shutdown, as soon as the last instance of [`InferenceApi`] is dropped.
    pub async fn wait_for_shutdown(self) {
        drop(self.send);
        self.handle.await.unwrap();
    }
}

/// Use this to execute tasks with the inference API. The existence of this API handle implies the
/// actor is alive and running. This means this handle must be disposed of, before the inference
/// actor can shut down.
#[derive(Clone)]
pub struct InferenceApi {
    send: mpsc::Sender<InferenceMessage>,
}

impl InferenceApi {
    pub fn new(send: mpsc::Sender<InferenceMessage>) -> InferenceApi {
        InferenceApi { send }
    }

    pub async fn complete_text(
        &self,
        request: CompletionRequest,
        api_token: String,
    ) -> anyhow::Result<Completion> {
        let (send, recv) = oneshot::channel();
        let msg = InferenceMessage::CompleteText {
            request,
            send,
            api_token,
        };
        self.send
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await
            .expect("sender must be alive when awaiting for answers")
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CompletionParams {
    pub max_tokens: Option<u32>,
    pub temperature: Option<f64>,
    pub top_k: Option<u32>,
    pub top_p: Option<f64>,
    pub stop: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CompletionRequest {
    pub prompt: String,
    pub model: String,
    pub params: CompletionParams,
}

impl CompletionRequest {
    pub fn new(prompt: String, model: String) -> Self {
        Self {
            prompt,
            model,
            params: CompletionParams::default(),
        }
    }

    pub fn with_params(mut self, params: CompletionParams) -> Self {
        self.params = params;
        self
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Completion {
    pub text: String,
    pub finish_reason: FinishReason,
}

/// Private implementation of the inference actor running in its own dedicated green thread.
struct InferenceActor<C: InferenceClient + Send + Sync + 'static> {
    client: Arc<C>,
    recv: mpsc::Receiver<InferenceMessage>,
    running_requests: FuturesUnordered<Pin<Box<dyn Future<Output = ()> + Send>>>,
}

impl<C: InferenceClient + Send + Sync + 'static> InferenceActor<C> {
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

#[derive(Debug)]
pub enum InferenceMessage {
    CompleteText {
        request: CompletionRequest,
        send: oneshot::Sender<anyhow::Result<Completion>>,
        api_token: String,
    },
}

impl InferenceMessage {
    async fn act(self, client: &impl InferenceClient) {
        match self {
            Self::CompleteText {
                request,
                send,
                api_token,
            } => {
                let mut remaining_retries = 5;
                let result = loop {
                    match client.complete_text(&request, api_token.clone()).await {
                        Ok(value) => break Ok(value),
                        Err(e) if remaining_retries <= 0 => {
                            error!("Error completing text: {e}");
                            break Err(e);
                        }
                        Err(e) => {
                            warn!("Retrying completion: {e}");
                        }
                    };
                    remaining_retries -= 1;
                };
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

    use crate::inference::client::InferenceClient;

    use super::*;

    impl Completion {
        pub fn from_text(completion: impl Into<String>) -> Self {
            Self {
                text: completion.into(),
                finish_reason: FinishReason::Stop,
            }
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
        async fn complete_text(
            &self,
            _params: &super::CompletionRequest,
            _api_token: String,
        ) -> anyhow::Result<Completion> {
            let remaining = self
                .remaining_failures
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |f| {
                    Some(f.saturating_sub(1))
                })
                .unwrap();

            if remaining == 0 {
                Err(anyhow!("Inference error"))
            } else {
                Ok(Completion::from_text("Completion succeeded"))
            }
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
            ..Default::default()
        };

        // when
        let result = inference_api
            .complete_text(request, "dummy_api".to_owned())
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
    pub struct AssertConcurrentClient {
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
        async fn complete_text(
            &self,
            request: &CompletionRequest,
            _api_token: String,
        ) -> anyhow::Result<Completion> {
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
        let inference = Inference::with_client(client);
        let api = inference.api();

        // When
        // Schedule two tasks
        let resp = try_join!(
            api.complete_text(complete_text_params_dummy(), "0".to_owned()),
            api.complete_text(complete_text_params_dummy(), "1".to_owned())
        );

        // Then: Both run concurrently and only return once both are completed.
        assert!(resp.is_ok());

        // We need to drop the sender in order for `actor.run` to terminate
        drop(api);
        inference.wait_for_shutdown().await;
    }
}
