use std::{collections::HashMap, sync::Arc};

use aleph_alpha_client::Client;
use anyhow::Context as _;
use async_trait::async_trait;
use tokenizers::Tokenizer;
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

#[async_trait]
pub trait TokenizerApi {
    async fn tokenizer_by_model(
        &self,
        api_token: String,
        model_name: String,
    ) -> anyhow::Result<Arc<Tokenizer>>;
}

#[async_trait]
impl TokenizerApi for mpsc::Sender<TokenizersMsg> {
    async fn tokenizer_by_model(
        &self,
        api_token: String,
        model_name: String,
    ) -> anyhow::Result<Arc<Tokenizer>> {
        let (send, recv) = oneshot::channel();
        let msg = TokenizersMsg::TokenizerByModel {
            api_token,
            model_name,
            send,
        };
        self.send(msg).await.unwrap();
        recv.await.unwrap()
    }
}

/// Actor providing tokenizers. These tokenizers are currently used to power chunking logic for CSI
pub struct Tokenizers {
    sender: mpsc::Sender<TokenizersMsg>,
    handle: JoinHandle<()>,
}

impl Tokenizers {
    pub fn new(api_base_url: String) -> anyhow::Result<Self> {
        let (sender, receiver) = mpsc::channel(1);
        let client = Client::new(api_base_url, None)?;
        let handle = tokio::spawn(async move {
            let mut actor = TokenizersActor::new(receiver, client);
            actor.run().await;
        });
        Ok(Tokenizers { sender, handle })
    }

    pub fn api(&self) -> mpsc::Sender<TokenizersMsg> {
        self.sender.clone()
    }

    pub async fn wait_for_shutdown(self) {
        drop(self.sender);
        self.handle.await.unwrap();
    }
}

pub enum TokenizersMsg {
    TokenizerByModel {
        api_token: String,
        model_name: String,
        send: oneshot::Sender<anyhow::Result<Arc<Tokenizer>>>,
    },
}

struct TokenizersActor {
    receiver: mpsc::Receiver<TokenizersMsg>,
    /// Used to connect to the Aleph Alpha inference API which does serve the tokenizers.
    client: Client,
    /// Cache Tokenizers by model name. Currently this is case sensitive, due to the AA API being
    /// case sensitive. We wrap tokenizers in `Arc` so we can send them in a fire and forget manner
    /// to the requesting skill runtime, and we do not need to worry about keeping them alive.
    cache: HashMap<String, Arc<Tokenizer>>,
}

impl TokenizersActor {
    pub fn new(receiver: mpsc::Receiver<TokenizersMsg>, client: Client) -> Self {
        TokenizersActor {
            receiver,
            client,
            cache: HashMap::new(),
        }
    }

    pub async fn run(&mut self) {
        while let Some(msg) = self.receiver.recv().await {
            self.act(msg).await;
        }
    }

    async fn act(&mut self, msg: TokenizersMsg) {
        match msg {
            TokenizersMsg::TokenizerByModel {
                api_token,
                model_name,
                send,
            } => {
                // First see if we find the tokenizer in cache
                let tokenizer_result = if let Some(tokenizer) = self.cache.get(&model_name) {
                    // Yeah, cache hit! Let's return our find
                    Ok(tokenizer.clone())
                } else {
                    // Miss, we need to request and insert it first
                    match self
                        .client
                        .tokenizer_by_model(&model_name, Some(api_token))
                        .await
                    {
                        Ok(tokenizer) => {
                            let tokenizer = Arc::new(tokenizer);
                            self.cache.insert(model_name, tokenizer.clone());
                            Ok(tokenizer)
                        }
                        Err(e) => {
                            let error: anyhow::Error = e.into();
                            Err(error).with_context(|| {
                                format!("Error fetching tokenizer for {model_name}")
                            })
                        }
                    }
                };
                let send_result = send.send(tokenizer_result);
                drop(send_result);
            }
        }
    }
}

#[cfg(test)]
pub mod tests {
    use anyhow::anyhow;
    use async_trait::async_trait;
    use std::sync::Arc;

    use super::{Tokenizer, TokenizerApi};

    use crate::{
        tests::{api_token, inference_url},
        tokenizers::Tokenizers,
    };

    /// A real world hugging face tokenizer for testing
    pub fn pharia_1_llm_7b_control_tokenizer() -> Tokenizer {
        let tokenizer = include_bytes!("tokenizers/pharia-1-llm-7b-control_tokenizer.json");
        Tokenizer::from_bytes(tokenizer).unwrap()
    }

    /// A skill executer double, loaded up with predefined answers.
    #[derive(Clone)]
    pub struct FakeTokenizers;

    #[async_trait]
    impl TokenizerApi for FakeTokenizers {
        async fn tokenizer_by_model(
            &self,
            _api_token: String,
            model_name: String,
        ) -> anyhow::Result<Arc<Tokenizer>> {
            if model_name == "Pharia-1-LLM-7B-control" {
                Ok(Arc::new(pharia_1_llm_7b_control_tokenizer()))
            } else {
                Err(anyhow!(
                    "model '{}' not supported by FakeTokenizers",
                    model_name
                ))
            }
        }
    }

    #[tokio::test]
    async fn fetch_pharia_1_llm_7b_control_tokenizer() {
        // Given a model name and the actual inference API
        let model_name = "pharia-1-llm-7b-control";
        let base_url = inference_url();
        let api_token = api_token().to_owned();

        // When we can request a tokenizer from the AA API
        let actor = Tokenizers::new(base_url.to_owned()).unwrap();
        let api = actor.api();
        let tokenizer = api
            .tokenizer_by_model(api_token, model_name.to_owned())
            .await
            .unwrap();

        // Then we can use the tokenizer
        assert_eq!(tokenizer.get_vocab_size(true), 128_000);
    }
}
