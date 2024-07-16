use aleph_alpha_client::Client;
use anyhow::Error;
use serde::{Deserialize, Serialize};
use tokio::{
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

    pub fn with_client(client: impl InferenceClient + Send + 'static) -> Self {
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
        &mut self,
        params: CompleteTextParameters,
        api_token: String,
    ) -> Result<String, Error> {
        let (send, recv) = oneshot::channel();
        let msg = InferenceMessage::CompleteText {
            params,
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

#[derive(Debug, Serialize, Deserialize)]
pub struct CompleteTextParameters {
    pub prompt: String,
    pub model: String,
    pub max_tokens: u32,
}

/// Private implementation of the inference actor running in its own dedicated green thread.
struct InferenceActor<C> {
    client: C,
    recv: mpsc::Receiver<InferenceMessage>,
}

impl<C> InferenceActor<C> {
    fn new(client: C, recv: mpsc::Receiver<InferenceMessage>) -> Self {
        InferenceActor { client, recv }
    }

    async fn run(&mut self)
    where
        C: InferenceClient,
    {
        while let Some(msg) = self.recv.recv().await {
            msg.act(&mut self.client).await;
        }
    }
}

pub enum InferenceMessage {
    CompleteText {
        params: CompleteTextParameters,
        send: oneshot::Sender<Result<String, Error>>,
        api_token: String,
    },
}

impl InferenceMessage {
    async fn act(self, client: &mut impl InferenceClient) {
        match self {
            InferenceMessage::CompleteText {
                params,
                send,
                api_token,
            } => {
                let mut remaining_retries = 5;
                let result = loop {
                    match client.complete_text(&params, api_token.clone()).await {
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
    use std::time::Duration;

    use anyhow::{anyhow, Error};
    use tokio::{
        sync::{mpsc, oneshot},
        task::JoinHandle, time::timeout,
    };

    use crate::inference::client::InferenceClient;

    use super::{CompleteTextParameters, Inference, InferenceApi, InferenceMessage};

    /// Always return the same completion
    pub struct InferenceStub {
        send: mpsc::Sender<InferenceMessage>,
        join_handle: JoinHandle<()>,
    }

    impl InferenceStub {
        pub fn new(result: impl Fn() -> Result<String, Error> + Send + 'static) -> Self {
            let (send, mut recv) = mpsc::channel::<InferenceMessage>(1);
            let join_handle = tokio::spawn(async move {
                while let Some(msg) = recv.recv().await {
                    match msg {
                        InferenceMessage::CompleteText { send, .. } => {
                            send.send(result()).unwrap();
                        }
                    }
                }
            });

            Self { send, join_handle }
        }
        pub fn with_completion(completion: impl Into<String>) -> Self {
            let completion = completion.into();
            Self::new(move || Ok(completion.clone()))
        }

        pub async fn wait_for_shutdown(self) {
            drop(self.send);
            self.join_handle.await.unwrap();
        }

        pub fn api(&self) -> InferenceApi {
            InferenceApi::new(self.send.clone())
        }
    }

    struct SaboteurClient {
        remaining_failures: usize,
    }

    impl SaboteurClient {
        fn new(remaining_failures: usize) -> Self {
            Self { remaining_failures }
        }
    }

    impl InferenceClient for SaboteurClient {
        async fn complete_text(
            &mut self,
            _params: &super::CompleteTextParameters,
            _api_token: String,
        ) -> Result<String, Error> {
            if self.remaining_failures > 0 {
                self.remaining_failures -= 1;
                Err(anyhow!("Inference error"))
            } else {
                Ok("Completion succeeded".to_owned())
            }
        }
    }

    #[tokio::test]
    async fn recover_from_connection_loss() {
        // given
        let client = SaboteurClient::new(2);
        let inference = Inference::with_client(client);
        let mut inference_api = inference.api();
        let params = CompleteTextParameters {
            prompt: "dummy_prompt".to_owned(),
            model: "dummy_model".to_owned(),
            max_tokens: 42,
        };

        // when
        let result = inference_api
            .complete_text(params, "dummy_api".to_owned())
            .await;

        // then
        assert!(result.is_ok());
    }

    /// A test double which for an inference client, which has every request pending until
    /// explicitly told to be ready.
    struct LatchClient {
        /// For every call to inference we want to pop one receiver in order to wait for the answer.
        /// It is up to the [`LatchControl`] to see to it, that these might be answered.
        receivers: Vec<oneshot::Receiver<String>>,
    }

    impl LatchClient {
        pub fn new(number_of_supported_requests: usize) -> (Self, LatchControl) {
            let mut senders = Vec::new();
            let mut receivers = Vec::new();
            for _ in 0..number_of_supported_requests {
                let (send, recv) = oneshot::channel();
                senders.push(Some(send));
                receivers.push(recv);
            }
            (LatchClient { receivers }, LatchControl { senders })
        }
    }

    impl InferenceClient for LatchClient {
        async fn complete_text(
            &mut self,
            _params: &CompleteTextParameters,
            _api_token: String,
        ) -> Result<String, Error> {
            let recv = self.receivers.pop().expect(
                "complete_text must not be called more often than supported by test double",
            );
            Ok(recv.await.unwrap())
        }
    }

    /// Sibling of [`LatchClient`]. Used to answer the completions in any order desired by the caller
    /// in a different thread of execution
    struct LatchControl {
        /// We maintain the senders as Options, so we do not need mess with the indices of the Vec
        /// if we consume the senders. This is to support test scenarios in which we want the
        /// completions to be answered out of order.
        senders: Vec<Option<oneshot::Sender<String>>>,
    }

    impl LatchControl {
        /// Answer the nth completion requested with complete text
        pub fn answer_nth(&mut self, n: usize, completion: String) {
            // We pop the receivers in reverse orders, so we also revert the index when answering
            // requests.
            let index = self.senders.len() - n - 1;
            let send = self.senders.get_mut(index).unwrap().take().unwrap();
            let _result = send.send(completion);
        }
    }

    /// Dummy complete text params for test, if you do not care particular about them.
    fn complete_text_params_dummy() -> CompleteTextParameters {
        CompleteTextParameters {
            prompt: "Dummy prompt".to_owned(),
            model: "Dummy model name".to_owned(),
            max_tokens: 128,
        }
    }

    /// We want to ensure that the actor invokes the client multiple times concurrently instead of
    /// only one inference request at a time.
    #[tokio::test]
    #[should_panic] // Inference is not concurrent yet
    async fn concurrent_invocation_of_client() {
        // Given
        let (client, mut control) = LatchClient::new(2);
        let inference = Inference::with_client(client);
        let mut api_one = inference.api();
        let mut api_two = inference.api();

        // When
        // This means the second call to complete_text over the api, will succeed almost immediatly,
        // if only it is actually invoked.
        control.answer_nth(1, "second".to_owned());

        // Schedule two tasks
        let first =
            api_one.complete_text(complete_text_params_dummy(), "dummy api token".to_owned());
        let second =
            api_two.complete_text(complete_text_params_dummy(), "dummy api token".to_owned());

        // Wait for the second one to be completed before answering the first one. This will block
        // forever if inference 
        let potential_timeout = timeout(Duration::from_secs(1), second).await;

        // Then: Second task did not timeout
        assert!(potential_timeout.is_ok());

        // Free resources associated with test
        // Second one done, now answer first task
        control.answer_nth(0, "first".to_owned());
        let _any_completion = first.await.unwrap();
        // We need to drop the sender in order for `actor.run` to terminate
        drop(api_one);
        drop(api_two);
        inference.wait_for_shutdown().await;
    }
}
