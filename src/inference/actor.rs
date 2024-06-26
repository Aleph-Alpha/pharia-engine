use aleph_alpha_client::Client;
use anyhow::Error;
use serde::{Deserialize, Serialize};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};
use tracing::{error, warn};

use super::client::InferenceClient;

pub struct Inference {
    send: mpsc::Sender<InferenceMessage>,
    handle: JoinHandle<()>,
}

impl Inference {
    pub fn new() -> Self {
        let client = Client::new("DUMMY").unwrap();
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

    pub async fn shutdown(self) {
        drop(self.send);
        self.handle.await.unwrap();
    }
}

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
    use anyhow::{anyhow, Error};
    use tokio::{sync::mpsc, task::JoinHandle};

    use crate::inference::client::InferenceClient;

    use super::{CompleteTextParameters, Inference, InferenceApi, InferenceMessage};

    /// Always return the same completion
    pub struct InferenceStub {
        send: mpsc::Sender<InferenceMessage>,
        join_handle: JoinHandle<()>,
    }

    impl InferenceStub {
        pub fn new(completion: impl Into<String>) -> Self {
            let (send, mut recv) = mpsc::channel::<InferenceMessage>(1);
            let completion = completion.into();
            let join_handle = tokio::spawn(async move {
                while let Some(msg) = recv.recv().await {
                    match msg {
                        InferenceMessage::CompleteText { send, .. } => {
                            send.send(Ok(completion.clone())).unwrap();
                        }
                    }
                }
            });

            Self { send, join_handle }
        }

        pub async fn shutdown(self) {
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
}
