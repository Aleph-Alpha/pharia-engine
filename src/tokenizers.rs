use std::sync::Arc;

use futures::channel::oneshot;
use tokenizers::Tokenizer;
use tokio::{sync::mpsc, task::JoinHandle,};
use anyhow::{anyhow, Context as _};

#[derive(Clone)]
pub struct TokenizersApi {
    sender: mpsc::Sender<TokenizersMsg>
}

impl TokenizersApi {
    pub fn new(sender: mpsc::Sender<TokenizersMsg>) -> Self {
        TokenizersApi { sender }
    }

    pub async fn tokenizer_by_model(&mut self, api_token: String, model_name: String) -> Result<Arc<Tokenizer>, anyhow::Error> {
        let (send, recv) = oneshot::channel();
        let msg = TokenizersMsg::TokenizerByModel { api_token, model_name, send };
        self.sender.send(msg).await.unwrap();
        recv.await.unwrap()
    }
}

/// Actor providing tokenizers. These tokenizers are currently used to power chunking logic for CSI
pub struct Tokenizers {
    sender: mpsc::Sender<TokenizersMsg>,
    handle: JoinHandle<()>
}

impl Tokenizers {
    pub fn new(api_base_url: String) -> Self {
        let (sender, receiver) = mpsc::channel(1);
        let handle = tokio::spawn(async move {
            let mut actor = TokenizersActor::new(receiver, api_base_url);
            actor.run().await
        });
        Tokenizers {
            sender,
            handle
        }
    }

    pub fn api(&self) -> TokenizersApi {
        TokenizersApi::new(self.sender.clone())
    }

    pub async fn wait_for_shutdown(self) {
        drop(self.sender);
        self.handle.await.unwrap();
    }
}

pub enum TokenizersMsg {
    TokenizerByModel{
        api_token: String,
        model_name: String,
        send: oneshot::Sender<Result<Arc<Tokenizer>, anyhow::Error>>
    }
}

struct TokenizersActor {
    receiver: mpsc::Receiver<TokenizersMsg>,
    api_base_url: String,
}

impl TokenizersActor {
    pub fn new(receiver: mpsc::Receiver<TokenizersMsg>, api_base_url: String) -> Self {
        TokenizersActor { receiver, api_base_url }
    }

    pub async fn run(&mut self) {
        while let Some(msg) = self.receiver.recv().await {
            self.act(msg).await
        }
    }

    async fn act(&mut self, msg: TokenizersMsg) {
        match msg {
            TokenizersMsg::TokenizerByModel { api_token, model_name, send } => {
                let result = tokenizer_for_model(&self.api_base_url, api_token, &model_name).await;
                let send_result = send.send(result.map(Arc::new));
                drop(send_result)
            },
        }
    }
}

/// This method does the actual work of sending the request which fetches the tonkenizer from the API
async fn tokenizer_for_model(api_base_url: &str, api_token: String, model: &str) -> Result<Tokenizer, anyhow::Error> {
    let url = format!("{}/models/{model}/tokenizer", api_base_url);
    let client = reqwest::Client::new();
    let response = client
        .get(url)
        .bearer_auth(api_token)
        .send()
        .await
        .with_context(|| format!("Error fetching tokenizer for {model}"))?;
    response
        .error_for_status_ref()
        .with_context(|| format!("Error fetching tokenizer for {model}"))?;
    let tokenizer = response
        .bytes()
        .await
        .with_context(|| format!("Error fetching tokenizer for {model}"))?;

    let tokenizer = Tokenizer::from_bytes(tokenizer).map_err(|e| {
        anyhow!(
            "Error deserializing tokenizer for {model}: {}",
            e.to_string()
        )
    })?;
    Ok(tokenizer)
}

#[cfg(test)]
pub mod tests {
    use std::sync::Arc;

    use tokenizers::Tokenizer;
    use tokio::{sync::mpsc, task::JoinHandle};
    use super::{tokenizer_for_model, TokenizersApi, TokenizersMsg};

    use crate::tests::{api_token, inference_address};

    /// A real world hugging face tokenizer for testing
    pub fn pharia_1_llm_7b_control_tokenizer() -> Tokenizer {
        let tokenizer = include_bytes!("tokenizers/pharia-1-llm-7b-control_tokenizer.json");
        Tokenizer::from_bytes(tokenizer).unwrap()
    }

    /// A skill executer double, loaded up with predefined answers.
    pub struct StubTokenizers {
        send: mpsc::Sender<TokenizersMsg>,
        handle: JoinHandle<()>,
    }

    impl StubTokenizers {
        pub fn new(
        ) -> StubTokenizers {
            let (send, mut recv) = mpsc::channel(1);
            let handle = tokio::spawn(async move {
                while let Some(msg) = recv.recv().await {
                    match msg {
                        TokenizersMsg::TokenizerByModel { api_token: _, model_name, send } => {
                            if model_name == "Pharia-1-LLM-7B-control" {
                                send.send(Ok(Arc::new(pharia_1_llm_7b_control_tokenizer()))).unwrap();
                            }
                        },
                    }
                }
            });
            Self { send, handle }
        }

        pub fn api(&self) -> TokenizersApi {
            TokenizersApi::new(self.send.clone())
        }

        pub async fn shutdown(self) {
            drop(self.send);
            self.handle.await.unwrap();
        }
    }

    #[tokio::test]
    async fn fetch_pharia_1_llm_7b_control_tokenizer() {
        // Given a model name and the actual inference API
        let model_name = "Pharia-1-LLM-7B-control";
        let base_url = inference_address();
        let api_token = api_token().to_owned();

        // When we can request a tokenizer from the AA API
        let tokenizer = tokenizer_for_model(&base_url, api_token, model_name).await.unwrap();

        // Then we can use the tokenizer
        assert_eq!(tokenizer.get_vocab_size(true), 128_000);
    }
}