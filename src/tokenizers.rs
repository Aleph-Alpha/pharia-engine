use std::{collections::HashMap, sync::Arc};

use aleph_alpha_client::Client;
use anyhow::Context as _;
use tokenizers::Tokenizer;
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};
use tracing::warn;

use crate::{
    authorization::Authentication, inference::InferenceNotConfigured, logging::TracingContext,
};

pub trait TokenizerApi {
    fn tokenizer_by_model(
        &self,
        auth: Authentication,
        tracing_context: TracingContext,
        model_name: String,
    ) -> impl Future<Output = anyhow::Result<Arc<Tokenizer>>> + Send;
}

/// Opaque wrapper around a sender to the tokenizer actor, so we do not need to expose our message
/// type.
#[derive(Clone)]
pub struct TokenizerSender(mpsc::Sender<TokenizersMsg>);

impl TokenizerApi for TokenizerSender {
    async fn tokenizer_by_model(
        &self,
        auth: Authentication,
        tracing_context: TracingContext,
        model_name: String,
    ) -> anyhow::Result<Arc<Tokenizer>> {
        let (send, recv) = oneshot::channel();
        let msg = TokenizersMsg::TokenizerByModel {
            auth,
            tracing_context,
            model_name,
            send,
        };
        self.0.send(msg).await.unwrap();
        recv.await.unwrap()
    }
}

/// Actor providing tokenizers. These tokenizers are currently used to power chunking logic for CSI
pub struct Tokenizers {
    sender: mpsc::Sender<TokenizersMsg>,
    handle: JoinHandle<()>,
}

/// Where to get the tokenizers from.
pub enum TokenizersConfig<'a> {
    /// Configured with an Aleph Alpha inference URL, which serves the tokenizers.
    AlephAlpha { inference_url: &'a str },
    /// No Aleph Alpha inference URL configured, unable to fetch tokenizers.
    None,
}

impl Tokenizers {
    pub fn new(config: TokenizersConfig<'_>) -> Self {
        if let TokenizersConfig::AlephAlpha { inference_url } = config {
            let client = Client::new(inference_url, None).unwrap();
            Self::with_client(client)
        } else {
            warn!(
                target: "pharia-kernel::tokenizers",
                "Chunking is only supported for models hosted by Aleph Alpha inference, \
                running without tokenizer capabilities."
            );
            Self::with_client(InferenceNotConfigured)
        }
    }

    fn with_client(client: impl TokenizerClient) -> Self {
        let (sender, receiver) = mpsc::channel(1);
        let handle = tokio::spawn(async move {
            let mut actor = TokenizersActor::new(receiver, client);
            actor.run().await;
        });
        Tokenizers { sender, handle }
    }

    pub fn api(&self) -> TokenizerSender {
        TokenizerSender(self.sender.clone())
    }

    pub async fn wait_for_shutdown(self) {
        drop(self.sender);
        self.handle.await.unwrap();
    }
}

enum TokenizersMsg {
    TokenizerByModel {
        auth: Authentication,
        tracing_context: TracingContext,
        model_name: String,
        send: oneshot::Sender<anyhow::Result<Arc<Tokenizer>>>,
    },
}

struct TokenizersActor<C: TokenizerClient> {
    receiver: mpsc::Receiver<TokenizersMsg>,
    /// Used to connect to the Aleph Alpha inference API which does serve the tokenizers.
    client: C,
    /// Cache Tokenizers by model name. Currently this is case sensitive, due to the AA API being
    /// case sensitive. We wrap tokenizers in `Arc` so we can send them in a fire and forget manner
    /// to the requesting skill runtime, and we do not need to worry about keeping them alive.
    cache: HashMap<String, Arc<Tokenizer>>,
}

impl<C: TokenizerClient> TokenizersActor<C>
where
    C: TokenizerClient,
{
    pub fn new(receiver: mpsc::Receiver<TokenizersMsg>, client: C) -> Self {
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
                auth,
                tracing_context,
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
                        .tokenizer_by_model(&model_name, auth, tracing_context)
                        .await
                    {
                        Ok(tokenizer) => {
                            let tokenizer = Arc::new(tokenizer);
                            self.cache.insert(model_name, tokenizer.clone());
                            Ok(tokenizer)
                        }
                        Err(e) => Err(e)
                            .with_context(|| format!("Error fetching tokenizer for {model_name}")),
                    }
                };
                let send_result = send.send(tokenizer_result);
                drop(send_result);
            }
        }
    }
}

trait TokenizerClient: Send + Sync + 'static {
    fn tokenizer_by_model(
        &self,
        model_name: &str,
        auth: Authentication,
        tracing_context: TracingContext,
    ) -> impl Future<Output = anyhow::Result<Tokenizer>> + Send;
}

impl TokenizerClient for Client {
    async fn tokenizer_by_model(
        &self,
        model_name: &str,
        auth: Authentication,
        tracing_context: TracingContext,
    ) -> anyhow::Result<Tokenizer> {
        let api_token = auth.into_string().ok_or_else(|| {
            anyhow::anyhow!(
                "Fetching tokenizers from the Aleph Alpha inference API requires a PhariaAI token. \
                Please provide a valid token in the Authorization header."
            )
        })?;
        self.tokenizer_by_model(
            model_name,
            Some(api_token),
            tracing_context.as_inference_client_context(),
        )
        .await
        .map_err(anyhow::Error::from)
    }
}

impl TokenizerClient for InferenceNotConfigured {
    async fn tokenizer_by_model(
        &self,
        _model_name: &str,
        _auth: Authentication,
        _tracing_context: TracingContext,
    ) -> anyhow::Result<Tokenizer> {
        Err(anyhow::anyhow!(
            "No inference backend configured. The Kernel is running without inference \
            capabilities. To enable inference capabilities, please ask your operator to configure \
            the Inference URL in the Kernel configuration."
        ))
    }
}

#[cfg(test)]
pub mod tests {
    use anyhow::anyhow;
    use std::sync::Arc;

    use super::{Tokenizer, TokenizerApi};

    use crate::{
        authorization::Authentication,
        logging::TracingContext,
        tests::{api_token, inference_url},
        tokenizers::{Tokenizers, TokenizersConfig},
    };

    /// A real world hugging face tokenizer for testing
    pub fn pharia_1_llm_7b_control_tokenizer() -> Tokenizer {
        let tokenizer = include_bytes!("tokenizers/pharia-1-llm-7b-control_tokenizer.json");
        Tokenizer::from_bytes(tokenizer).unwrap()
    }

    /// A skill executer double, loaded up with predefined answers.
    #[derive(Clone)]
    pub struct FakeTokenizers;

    impl TokenizerApi for FakeTokenizers {
        async fn tokenizer_by_model(
            &self,
            _auth: Authentication,
            _tracing_context: TracingContext,
            model_name: String,
        ) -> anyhow::Result<Arc<Tokenizer>> {
            if model_name == "pharia-1-llm-7B-control" {
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
        let inference_url = inference_url();
        let api_token = Authentication::new(api_token());

        // When we can request a tokenizer from the AA API
        let actor = Tokenizers::new(TokenizersConfig::AlephAlpha { inference_url });
        let api = actor.api();
        let tokenizer = api
            .tokenizer_by_model(api_token, TracingContext::dummy(), model_name.to_owned())
            .await
            .unwrap();

        // Then we can use the tokenizer
        assert_eq!(tokenizer.get_vocab_size(true), 128_000);
    }
}
