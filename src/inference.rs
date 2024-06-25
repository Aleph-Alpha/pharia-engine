use std::future::Future;

use aleph_alpha_client::{Client, How, TaskCompletion};
use serde::{Deserialize, Serialize};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

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
    ) -> String {
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
        recv.await.unwrap()
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

pub trait InferenceClient {
    fn complete_text(
        &self,
        params: &CompleteTextParameters,
        api_token: String,
    ) -> impl Future<Output = String> + Send;
}

impl InferenceClient for Client {
    async fn complete_text(&self, params: &CompleteTextParameters, api_token: String) -> String {
        let task = TaskCompletion::from_text(&params.prompt, params.max_tokens);
        self.completion(
            &task,
            &params.model,
            &How {
                api_token: Some(api_token.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .completion
    }
}

pub enum InferenceMessage {
    CompleteText {
        params: CompleteTextParameters,
        send: oneshot::Sender<String>,
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
                let result = client.complete_text(&params, api_token).await;
                drop(send.send(result));
            }
        }
    }
}

#[cfg(test)]
pub mod tests {
    use tokio::{sync::mpsc, task::JoinHandle};

    use super::{InferenceApi, InferenceMessage};

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
                            send.send(completion.clone()).unwrap();
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
}
