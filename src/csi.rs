use async_trait::async_trait;
use chunking::ChunkParams;
use futures::future::try_join_all;
use serde_json::Value;
use strum::IntoStaticStr;
use tokio::sync::mpsc;
use tracing::trace;

use crate::{
    inference::{ChatRequest, ChatResponse, Completion, CompletionRequest, InferenceApi},
    language_selection::{select_language, Language, SelectLanguageRequest},
    search::{DocumentIndexMessage, DocumentPath, SearchApi, SearchRequest, SearchResult},
    tokenizers::TokenizerApi,
};

pub use self::chunking::ChunkRequest;

pub mod chunking;

/// Collection of api handles to the actors used to implement the Cognitive System Interface (CSI)
///
/// For now this is just a collection of all the APIs without providing logic on its own
#[derive(Clone)]
pub struct CsiDrivers<T> {
    /// We use the inference Api to complete text
    pub inference: InferenceApi,
    pub search: mpsc::Sender<DocumentIndexMessage>,
    pub tokenizers: T,
}

/// Cognitive System Interface (CSI) as consumed internally by Pharia Kernel, before the CSI is
/// passed to the end user in Skill code we further strip away some of the accidental complexity.
/// See its sibling trait `CsiForSkills`.
#[async_trait]
pub trait Csi {
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

    async fn chat(&self, auth: String, request: ChatRequest)
        -> Result<ChatResponse, anyhow::Error>;

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

    async fn search(
        &self,
        auth: String,
        request: SearchRequest,
    ) -> Result<Vec<SearchResult>, anyhow::Error>;

    async fn document_metadata(
        &self,
        auth: String,
        document_path: DocumentPath,
    ) -> Result<Option<Value>, anyhow::Error>;
}

#[derive(IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum CsiMetrics {
    #[strum(to_string = "kernel_csi_requests_total")]
    CsiRequestsTotal,
}

impl From<CsiMetrics> for metrics::KeyName {
    fn from(value: CsiMetrics) -> Self {
        Self::from_const_str(value.into())
    }
}

#[async_trait]
impl<T> Csi for CsiDrivers<T>
where
    T: TokenizerApi + Send + Sync,
{
    async fn complete_text(
        &self,
        auth: String,
        request: CompletionRequest,
    ) -> Result<Completion, anyhow::Error> {
        metrics::counter!(CsiMetrics::CsiRequestsTotal, &[("function", "complete")]).increment(1);

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

    async fn chat(
        &self,
        auth: String,
        request: ChatRequest,
    ) -> Result<ChatResponse, anyhow::Error> {
        metrics::counter!(CsiMetrics::CsiRequestsTotal, &[("function", "chat")]).increment(1);

        trace!(
            "chat: request.model={} request.params.max_tokens={}",
            request.model,
            request
                .params
                .max_tokens
                .map_or_else(|| "None".to_owned(), |val| val.to_string()),
        );

        self.inference.chat(request, auth).await
    }

    async fn chunk(
        &self,
        auth: String,
        request: ChunkRequest,
    ) -> Result<Vec<String>, anyhow::Error> {
        metrics::counter!(CsiMetrics::CsiRequestsTotal, &[("function", "chunk")]).increment(1);

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

    async fn search(
        &self,
        auth: String,
        request: SearchRequest,
    ) -> Result<Vec<SearchResult>, anyhow::Error> {
        metrics::counter!(CsiMetrics::CsiRequestsTotal, &[("function", "search")]).increment(1);

        let index_path = &request.index_path;

        trace!(
            "search: namespace={} collection={} max_results={} min_score={}",
            index_path.namespace,
            index_path.collection,
            request.max_results,
            request
                .min_score
                .map_or_else(|| "None".to_owned(), |val| val.to_string())
        );
        self.search.search(request, auth).await
    }

    async fn document_metadata(
        &self,
        auth: String,
        document_path: DocumentPath,
    ) -> Result<Option<Value>, anyhow::Error> {
        metrics::counter!(
            CsiMetrics::CsiRequestsTotal,
            &[("function", "document_metadata")]
        )
        .increment(1);
        trace!(
            "document_metadata: namespace={}, collection={}, name={}",
            document_path.namespace,
            document_path.collection,
            document_path.name,
        );
        self.search.document_metadata(document_path, auth).await
    }
}

#[cfg(test)]
pub mod tests {
    use std::sync::Arc;

    use anyhow::bail;
    use async_trait::async_trait;
    use serde_json::Value;
    use tokio::sync::mpsc;

    use crate::{
        csi::chunking::ChunkParams,
        inference::{
            tests::InferenceStub, ChatParams, ChatRequest, ChatResponse, Completion,
            CompletionParams, CompletionRequest, InferenceApi, Message, Role,
        },
        search::{DocumentPath, SearchRequest, SearchResult},
        tests::api_token,
        tokenizers::tests::FakeTokenizers,
        FinishReason,
    };

    use super::{ChunkRequest, Csi, CsiDrivers};

    #[tokio::test]
    async fn chat() {
        // Given a chat request
        let chat_request = ChatRequest {
            model: "dummy_model".to_owned(),
            messages: vec![Message {
                role: Role::User,
                content: "Hello".to_owned(),
            }],
            params: ChatParams::default(),
        };

        // When chatting with the StubCsi
        let csi = StubCsi::empty();
        let result = csi
            .chat("dummy-token".to_owned(), chat_request)
            .await
            .unwrap();

        // Then the response is the same as the request
        assert_eq!(result.message.content, "Hello");
    }

    #[tokio::test]
    async fn chunk() {
        // Given a skill invocation context with a stub tokenizer provider
        let tokenizers = FakeTokenizers;
        let csi_apis = CsiDrivers {
            tokenizers,
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

    fn dummy_csi_drivers() -> CsiDrivers<FakeTokenizers> {
        let (send, _recv) = mpsc::channel(1);
        let inference = InferenceApi::new(send);

        let (search, _recv) = mpsc::channel(1);
        let tokenizers = FakeTokenizers;

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

        async fn chat(
            &self,
            _auth: String,
            _request: ChatRequest,
        ) -> Result<ChatResponse, anyhow::Error> {
            panic!("DummyCsi chat called")
        }

        async fn search(
            &self,
            _auth: String,
            _request: SearchRequest,
        ) -> Result<Vec<SearchResult>, anyhow::Error> {
            panic!("DummyCsi search called")
        }

        async fn document_metadata(
            &self,
            _auth: String,
            _document_path: DocumentPath,
        ) -> Result<Option<Value>, anyhow::Error> {
            panic!("DummyCsi metadata_document called")
        }
    }

    type CompleteFn =
        dyn Fn(CompletionRequest) -> Result<Completion, anyhow::Error> + Send + Sync + 'static;

    type ChunkFn =
        dyn Fn(ChunkRequest) -> Result<Vec<String>, anyhow::Error> + Send + Sync + 'static;

    #[derive(Clone)]
    pub struct StubCsi {
        pub completion: Arc<Box<CompleteFn>>,
        pub chunking: Arc<Box<ChunkFn>>,
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
            self.chunking = Arc::new(Box::new(f));
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

        async fn chat(
            &self,
            _auth: String,
            request: ChatRequest,
        ) -> Result<ChatResponse, anyhow::Error> {
            Ok(ChatResponse {
                message: request.messages.first().unwrap().clone(),
                finish_reason: FinishReason::Stop,
            })
        }

        async fn search(
            &self,
            _auth: String,
            _request: SearchRequest,
        ) -> Result<Vec<SearchResult>, anyhow::Error> {
            unimplemented!()
        }

        async fn document_metadata(
            &self,
            _auth: String,
            _document_path: DocumentPath,
        ) -> Result<Option<Value>, anyhow::Error> {
            unimplemented!()
        }
    }
}
