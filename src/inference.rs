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
        let (send, recv) = tokio::sync::mpsc::channel::<InferenceMessage>(1);
        let handle = tokio::spawn(async {
            InferenceActor::new(recv).run().await;
        });
        Inference { send, handle }
    }
    pub fn api(&self) -> InferenceApi {
        InferenceApi {
            send: self.send.clone(),
        }
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
    pub async fn complete_text(&mut self, params: CompleteTextParameters) -> String {
        let (send, recv) = oneshot::channel();
        let msg = InferenceMessage::CompleteText(params, send);
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

struct InferenceActor {
    client: Client,
    recv: mpsc::Receiver<InferenceMessage>,
}

impl InferenceActor {
    fn new(recv: mpsc::Receiver<InferenceMessage>) -> Self {
        let aa_api_token = std::env::var("AA_API_TOKEN").expect("AA_API_TOKEN variable not set");
        let client = Client::new(&aa_api_token).unwrap();
        InferenceActor { client, recv }
    }
    async fn run(&mut self) {
        while let Some(msg) = self.recv.recv().await {
            msg.act(&self.client).await;
        }
    }
}

enum InferenceMessage {
    CompleteText(CompleteTextParameters, oneshot::Sender<String>),
}

impl InferenceMessage {
    async fn act(self, client: &Client) {
        let _ = match self {
            InferenceMessage::CompleteText(params, out) => {
                let task = TaskCompletion::from_text(&params.prompt, params.max_tokens);
                let response = client
                    .completion(&task, &params.model, &How::default())
                    .await
                    .unwrap();
                out.send(response.completion)
            }
        };
    }
}
