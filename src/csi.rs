use async_trait::async_trait;
use chunking::ChunkParams;
use futures::future::try_join_all;
use tokio::sync::mpsc;
use tracing::trace;

use crate::{
    inference::{Completion, CompletionRequest, InferenceApi},
    language_selection::{select_language, Language, SelectLanguageRequest},
    search::SearchMessage,
    tokenizers::{TokenizerApi as _, TokenizersMsg},
};

pub use self::chunking::ChunkRequest;

pub mod chunking;

/// Collection of api handles to the actors used to implement the Cognitive System Interface (CSI)
///
/// For now this is just a collection of all the APIs without providing logic on its own
#[derive(Clone)]
pub struct CsiDrivers {
    /// We use the inference Api to complete text
    pub inference: InferenceApi,
    #[expect(dead_code, reason = "Unused so far")]
    pub search: mpsc::Sender<SearchMessage>,
    pub tokenizers: mpsc::Sender<TokenizersMsg>,
}

/// Cognitive System Interface (CSI) as consumed internally by Pharia Kernel, before the CSI is
/// passed to the end user in Skill code we further strip away some of the accidental complexity.
/// See its sibling trait `CsiForSkills`.
#[async_trait]
pub trait Csi: Clone + Send + Sync + 'static {
    async fn complete_text(
        &self,
        auth: String,
        request: CompletionRequest,
    ) -> Result<Completion, anyhow::Error>;

    async fn complete_all(
        &self,
        auth: String,
        requests: Vec<CompletionRequest>,
    ) -> Result<Vec<Completion>, anyhow::Error>;

    async fn chunk(
        &self,
        auth: String,
        request: ChunkRequest,
    ) -> Result<Vec<String>, anyhow::Error>;

    // While the implementation might not be async, we want the interface to be asynchronous.
    // It is up to the implementer whether the actual implementation is async.
    async fn select_language(
        &self,
        request: SelectLanguageRequest,
    ) -> anyhow::Result<Option<Language>> {
        // default implementation can be provided here because language selection is stateless
        Ok(tokio::task::spawn_blocking(move || select_language(request)).await?)
    }
}

#[async_trait]
impl Csi for CsiDrivers {
    async fn complete_text(
        &self,
        auth: String,
        request: CompletionRequest,
    ) -> Result<Completion, anyhow::Error> {
        trace!(
            "complete_text: request.model={} request.params.max_tokens={}",
            request.model,
            request
                .params
                .max_tokens
                .map_or_else(|| "None".to_owned(), |val| val.to_string()),
        );
        self.inference.complete_text(request, auth).await
    }

    async fn complete_all(
        &self,
        auth: String,
        requests: Vec<CompletionRequest>,
    ) -> Result<Vec<Completion>, anyhow::Error> {
        trace!("complete_all: requests.len()={}", requests.len());
        try_join_all(
            requests
                .into_iter()
                .map(|r| self.complete_text(auth.clone(), r))
                .collect::<Vec<_>>(),
        )
        .await
    }

    async fn chunk(
        &self,
        auth: String,
        request: ChunkRequest,
    ) -> Result<Vec<String>, anyhow::Error> {
        let ChunkRequest {
            text,
            params: ChunkParams { model, max_tokens },
        } = request;
        let text_len = text.len();

        let tokenizer = self.tokenizers.tokenizer_by_model(auth, model).await?;
        // Push into the blocking thread pool because this can be expensive for long documents
        let chunks =
            tokio::task::spawn_blocking(move || chunking::chunking(&text, &tokenizer, max_tokens))
                .await?;

        trace!(
            "chunk: text_len={} max_tokens={} -> chunks.len()={}",
            text_len,
            max_tokens,
            chunks.len()
        );
        Ok(chunks)
    }
}

#[cfg(test)]
pub mod tests {
    use std::sync::Arc;

    use anyhow::bail;
    use async_trait::async_trait;
    use tokio::{sync::mpsc, task::JoinHandle};

    use crate::{
        csi::chunking::ChunkParams,
        inference::{
            tests::InferenceStub, Completion, CompletionParams, CompletionRequest, InferenceApi,
        },
        tests::api_token,
        tokenizers::{tests::FakeTokenizers, TokenizerApi as _, TokenizersMsg},
    };

    use super::{ChunkRequest, Csi, CsiDrivers};

    #[tokio::test]
    async fn chunk() {
        // Given a skill invocation context with a stub tokenizer provider
        let tokenizers = FakeTokenizersActor::new();
        let csi_apis = CsiDrivers {
            tokenizers: tokenizers.api(),
            ..dummy_csi_drivers()
        };

        // When chunking a short text
        let model = "Pharia-1-LLM-7B-control".to_owned();
        let max_tokens = 10;
        let request = ChunkRequest {
            text: "Greet".to_owned(),
            params: ChunkParams { model, max_tokens },
        };
        let chunks = csi_apis
            .chunk("dummy_token".to_owned(), request)
            .await
            .unwrap();

        drop(csi_apis);
        tokenizers.shutdown().await;

        // Then a single chunk is returned
        assert_eq!(chunks.len(), 1);
    }

    #[tokio::test]
    async fn complete_all_completion_requests_in_respective_order() {
        // Given a CSI drivers with stub completion
        let inference_stub = InferenceStub::new(|r| Ok(Completion::from_text(r.prompt)));
        let csi_apis = CsiDrivers {
            inference: inference_stub.api(),
            ..dummy_csi_drivers()
        };

        // When requesting multiple completions
        let completion_req_1 = CompletionRequest {
            model: "dummy_model".to_owned(),
            prompt: "1st_request".to_owned(),
            params: CompletionParams {
                max_tokens: None,
                temperature: None,
                top_k: None,
                top_p: None,
                stop: vec![],
            },
        };

        let completion_req_2 = CompletionRequest {
            prompt: "2nd request".to_owned(),
            ..completion_req_1.clone()
        };

        let completions = csi_apis
            .complete_all(
                api_token().to_owned(),
                vec![completion_req_1, completion_req_2],
            )
            .await
            .unwrap();

        drop(csi_apis);
        inference_stub.wait_for_shutdown().await;

        // Then the completion must have the same order as the respective requests
        assert_eq!(completions.len(), 2);
        assert!(completions.first().unwrap().text.contains("1st"));
        assert!(completions.get(1).unwrap().text.contains("2nd"));
    }

    pub fn dummy_csi_drivers() -> CsiDrivers {
        let (send, _recv) = mpsc::channel(1);
        let inference = InferenceApi::new(send);

        let (search, _recv) = mpsc::channel(1);

        let (send, _recv) = mpsc::channel(1);
        let tokenizers = send;

        CsiDrivers {
            inference,
            search,
            tokenizers,
        }
    }

    #[derive(Clone)]
    pub struct DummyCsi;

    #[async_trait]
    impl Csi for DummyCsi {
        async fn complete_text(
            &self,
            _auth: String,
            _request: CompletionRequest,
        ) -> Result<Completion, anyhow::Error> {
            panic!("DummyCsi complete_text called")
        }

        async fn complete_all(
            &self,
            _auth: String,
            _requests: Vec<CompletionRequest>,
        ) -> Result<Vec<Completion>, anyhow::Error> {
            panic!("DummyCsi complete_all called")
        }

        async fn chunk(
            &self,
            _auth: String,
            _request: ChunkRequest,
        ) -> Result<Vec<String>, anyhow::Error> {
            panic!("DummyCsi complete_all called");
        }
    }

    #[derive(Clone)]
    pub struct StubCsi {
        pub completion: Arc<
            Box<
                dyn Fn(CompletionRequest) -> Result<Completion, anyhow::Error>
                    + Send
                    + Sync
                    + 'static,
            >,
        >,
        pub chunking: Arc<
            Box<dyn Fn(ChunkRequest) -> Result<Vec<String>, anyhow::Error> + Send + Sync + 'static>,
        >,
    }

    impl StubCsi {
        pub fn empty() -> Self {
            StubCsi {
                completion: Arc::new(Box::new(|_| bail!("Completion not set in StubCsi"))),
                chunking: Arc::new(Box::new(|_| bail!("Chunking not set in StubCsi"))),
            }
        }

        pub fn set_chunking(
            &mut self,
            f: impl Fn(ChunkRequest) -> Result<Vec<String>, anyhow::Error> + Send + Sync + 'static,
        ) {
            self.chunking = Arc::new(Box::new(f))
        }

        pub fn with_completion(
            f: impl Fn(CompletionRequest) -> Completion + Send + Sync + 'static,
        ) -> Self {
            StubCsi {
                completion: Arc::new(Box::new(move |cr| Ok(f(cr)))),
                ..Self::empty()
            }
        }

        pub fn with_completion_from_text(text: impl Into<String>) -> Self {
            let text: String = text.into();
            let completion = Completion::from_text(text);
            StubCsi::with_completion(move |_| completion.clone())
        }
    }

    #[async_trait]
    impl Csi for StubCsi {
        async fn complete_text(
            &self,
            _auth: String,
            request: CompletionRequest,
        ) -> Result<Completion, anyhow::Error> {
            (*self.completion)(request)
        }

        async fn complete_all(
            &self,
            _auth: String,
            requests: Vec<CompletionRequest>,
        ) -> Result<Vec<Completion>, anyhow::Error> {
            requests
                .into_iter()
                .map(|r| (*self.completion)(r))
                .collect()
        }

        async fn chunk(
            &self,
            _auth: String,
            request: ChunkRequest,
        ) -> Result<Vec<String>, anyhow::Error> {
            (*self.chunking)(request)
        }
    }

    struct FakeTokenizersActor {
        send: mpsc::Sender<TokenizersMsg>,
        handle: JoinHandle<()>,
    }

    impl FakeTokenizersActor {
        pub fn new() -> FakeTokenizersActor {
            let (send, mut recv) = mpsc::channel(1);
            let handle = tokio::spawn(async move {
                while let Some(msg) = recv.recv().await {
                    match msg {
                        TokenizersMsg::TokenizerByModel {
                            api_token: _,
                            model_name,
                            send,
                        } => {
                            let tokenizer = FakeTokenizers
                                .tokenizer_by_model("dummy_token".to_string(), model_name)
                                .await;
                            send.send(tokenizer).unwrap();
                        }
                    }
                }
            });
            Self { send, handle }
        }

        pub fn api(&self) -> mpsc::Sender<TokenizersMsg> {
            self.send.clone()
        }

        pub async fn shutdown(self) {
            drop(self.send);
            self.handle.await.unwrap();
        }
    }
}
