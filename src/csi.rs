use async_trait::async_trait;
use chunking::ChunkParams;
use futures::future::try_join_all;
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::trace;

use crate::{
    inference::{ChatRequest, ChatResponse, Completion, CompletionRequest, InferenceApi},
    language_selection::{select_language, Language, SelectLanguageRequest},
    search::{
        Document, DocumentIndexMessage, DocumentPath, SearchApi, SearchRequest, SearchResult,
    },
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
    async fn complete(
        &self,
        auth: String,
        requests: Vec<CompletionRequest>,
    ) -> anyhow::Result<Vec<Completion>>;

    async fn chat(
        &self,
        auth: String,
        requests: Vec<ChatRequest>,
    ) -> anyhow::Result<Vec<ChatResponse>>;

    async fn chunk(
        &self,
        auth: String,
        requests: Vec<ChunkRequest>,
    ) -> anyhow::Result<Vec<Vec<String>>>;

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
    ) -> anyhow::Result<Vec<SearchResult>>;

    async fn documents(
        &self,
        auth: String,
        requests: Vec<DocumentPath>,
    ) -> anyhow::Result<Vec<Document>>;

    async fn document_metadata(
        &self,
        auth: String,
        requests: Vec<DocumentPath>,
    ) -> anyhow::Result<Vec<Option<Value>>>;
}

pub enum CsiMetrics {
    CsiRequestsTotal,
}

impl From<CsiMetrics> for metrics::KeyName {
    fn from(value: CsiMetrics) -> Self {
        Self::from_const_str(match value {
            CsiMetrics::CsiRequestsTotal => "kernel_csi_requests_total",
        })
    }
}

#[async_trait]
impl<T> Csi for CsiDrivers<T>
where
    T: TokenizerApi + Send + Sync,
{
    async fn complete(
        &self,
        auth: String,
        requests: Vec<CompletionRequest>,
    ) -> anyhow::Result<Vec<Completion>> {
        metrics::counter!(CsiMetrics::CsiRequestsTotal, &[("function", "complete")])
            .increment(requests.len() as u64);
        try_join_all(
            requests
                .into_iter()
                .map(|r| {
                    trace!(
                        "complete: request.model={} request.params.max_tokens={}",
                        r.model,
                        r.params
                            .max_tokens
                            .map_or_else(|| "None".to_owned(), |val| val.to_string()),
                    );

                    self.inference.complete(r, auth.clone())
                })
                .collect::<Vec<_>>(),
        )
        .await
    }

    async fn chat(
        &self,
        auth: String,
        requests: Vec<ChatRequest>,
    ) -> anyhow::Result<Vec<ChatResponse>> {
        metrics::counter!(CsiMetrics::CsiRequestsTotal, &[("function", "chat")])
            .increment(requests.len() as u64);

        try_join_all(
            requests
                .into_iter()
                .map(|r| {
                    trace!(
                        "chat: request.model={} request.params.max_tokens={}",
                        r.model,
                        r.params
                            .max_tokens
                            .map_or_else(|| "None".to_owned(), |val| val.to_string()),
                    );

                    self.inference.chat(r, auth.clone())
                })
                .collect::<Vec<_>>(),
        )
        .await
    }

    async fn chunk(
        &self,
        auth: String,
        requests: Vec<ChunkRequest>,
    ) -> anyhow::Result<Vec<Vec<String>>> {
        metrics::counter!(CsiMetrics::CsiRequestsTotal, &[("function", "chunk")])
            .increment(requests.len() as u64);

        let auth = &auth;
        try_join_all(requests.into_iter().map(|request| async move {
            let ChunkRequest {
                text,
                params: ChunkParams { model, max_tokens },
            } = request;
            let text_len = text.len();

            let tokenizer = self
                .tokenizers
                .tokenizer_by_model(auth.clone(), model)
                .await?;
            // Push into the blocking thread pool because this can be expensive for long documents
            let chunks = tokio::task::spawn_blocking(move || {
                chunking::chunking(&text, &tokenizer, max_tokens)
            })
            .await?;

            trace!(
                "chunk: text_len={} max_tokens={} -> chunks.len()={}",
                text_len,
                max_tokens,
                chunks.len()
            );
            Ok(chunks)
        }))
        .await
    }

    async fn search(
        &self,
        auth: String,
        request: SearchRequest,
    ) -> anyhow::Result<Vec<SearchResult>> {
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

    async fn documents(
        &self,
        auth: String,
        requests: Vec<DocumentPath>,
    ) -> anyhow::Result<Vec<Document>> {
        metrics::counter!(CsiMetrics::CsiRequestsTotal, &[("function", "documents")]).increment(1);
        trace!("documents: requests.len()={}", requests.len());
        try_join_all(
            requests
                .into_iter()
                .map(|r| self.search.document(r, auth.clone()))
                .collect::<Vec<_>>(),
        )
        .await
    }

    async fn document_metadata(
        &self,
        auth: String,
        requests: Vec<DocumentPath>,
    ) -> anyhow::Result<Vec<Option<Value>>> {
        metrics::counter!(
            CsiMetrics::CsiRequestsTotal,
            &[("function", "document_metadata")]
        )
        .increment(1);
        trace!("document_metadata: requests.len()={}", requests.len());
        try_join_all(
            requests
                .into_iter()
                .map(|r| self.search.document_metadata(r, auth.clone()))
                .collect::<Vec<_>>(),
        )
        .await
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
        search::{tests::SearchStub, Document, DocumentPath, SearchRequest, SearchResult},
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
            .chat("dummy-token".to_owned(), vec![chat_request])
            .await
            .unwrap();

        // Then the response is the same as the request
        assert_eq!(result[0].message.content, "Hello");
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
            .chunk("dummy_token".to_owned(), vec![request])
            .await
            .unwrap();

        // Then a single chunk is returned
        assert_eq!(chunks[0].len(), 1);
    }

    #[tokio::test]
    async fn documents() {
        // Given a skill invocation context with a stub tokenizer provider
        let search = SearchStub::with_documents(|_| Document::dummy());
        let csi_apis = CsiDrivers {
            search: search.api(),
            ..dummy_csi_drivers()
        };

        // When requesting documents
        let request = vec![DocumentPath {
            namespace: "kernel".to_owned(),
            collection: "test".to_owned(),
            name: "docs".to_owned(),
        }];
        let documents = csi_apis
            .documents("dummy_token".to_owned(), request)
            .await
            .unwrap();

        // Then a single document is returned
        assert_eq!(documents.len(), 1);
    }

    #[tokio::test]
    async fn completion_requests_in_respective_order() {
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
            params: CompletionParams::default(),
        };

        let completion_req_2 = CompletionRequest {
            prompt: "2nd request".to_owned(),
            ..completion_req_1.clone()
        };

        let completions = csi_apis
            .complete(
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

    #[tokio::test]
    async fn document_metadata_requests_in_respective_order() {
        // Given a CSI drivers with stub search
        let search_stub = SearchStub::with_metadata(|r| Ok(Some(Value::String(r.name.clone()))));
        let csi_apis = CsiDrivers {
            search: search_stub.api(),
            ..dummy_csi_drivers()
        };

        // When requesting multiple metadata
        let request_1 = DocumentPath {
            namespace: "dummy_namespace".to_owned(),
            collection: "dummy_collection".to_owned(),
            name: "1st_request".to_owned(),
        };

        let request_2 = DocumentPath {
            name: "2nd request".to_owned(),
            ..request_1.clone()
        };

        let responses = csi_apis
            .document_metadata(api_token().to_owned(), vec![request_1, request_2])
            .await
            .unwrap();

        drop(csi_apis);
        search_stub.wait_for_shutdown().await;

        // Then the responses must have the same order as the respective requests
        let responses: Vec<String> = responses
            .into_iter()
            .map(|v| v.unwrap().to_string())
            .collect();

        assert_eq!(responses.len(), 2);
        assert!(responses[0].contains("1st"));
        assert!(responses[1].contains("2nd"));
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
        async fn complete(
            &self,
            _auth: String,
            _requests: Vec<CompletionRequest>,
        ) -> anyhow::Result<Vec<Completion>> {
            panic!("DummyCsi complete called")
        }

        async fn chunk(
            &self,
            _auth: String,
            _requests: Vec<ChunkRequest>,
        ) -> anyhow::Result<Vec<Vec<String>>> {
            panic!("DummyCsi complete called");
        }

        async fn chat(
            &self,
            _auth: String,
            _requests: Vec<ChatRequest>,
        ) -> anyhow::Result<Vec<ChatResponse>> {
            panic!("DummyCsi chat called")
        }

        async fn search(
            &self,
            _auth: String,
            _request: SearchRequest,
        ) -> anyhow::Result<Vec<SearchResult>> {
            panic!("DummyCsi search called")
        }

        async fn documents(
            &self,
            _auth: String,
            _requests: Vec<DocumentPath>,
        ) -> anyhow::Result<Vec<Document>> {
            panic!("DummyCsi documents called")
        }

        async fn document_metadata(
            &self,
            _auth: String,
            _document_paths: Vec<DocumentPath>,
        ) -> anyhow::Result<Vec<Option<Value>>> {
            panic!("DummyCsi metadata_document called")
        }
    }

    type CompleteFn =
        dyn Fn(CompletionRequest) -> anyhow::Result<Completion> + Send + Sync + 'static;

    type ChunkFn =
        dyn Fn(Vec<ChunkRequest>) -> anyhow::Result<Vec<Vec<String>>> + Send + Sync + 'static;

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
            f: impl Fn(Vec<ChunkRequest>) -> anyhow::Result<Vec<Vec<String>>> + Send + Sync + 'static,
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
        async fn complete(
            &self,
            _auth: String,
            requests: Vec<CompletionRequest>,
        ) -> anyhow::Result<Vec<Completion>> {
            requests
                .into_iter()
                .map(|r| (*self.completion)(r))
                .collect()
        }

        async fn chunk(
            &self,
            _auth: String,
            requests: Vec<ChunkRequest>,
        ) -> anyhow::Result<Vec<Vec<String>>> {
            (*self.chunking)(requests)
        }

        async fn chat(
            &self,
            _auth: String,
            requests: Vec<ChatRequest>,
        ) -> anyhow::Result<Vec<ChatResponse>> {
            Ok(requests
                .into_iter()
                .map(|request| ChatResponse {
                    message: request.messages.first().unwrap().clone(),
                    finish_reason: FinishReason::Stop,
                })
                .collect())
        }

        async fn search(
            &self,
            _auth: String,
            _request: SearchRequest,
        ) -> anyhow::Result<Vec<SearchResult>> {
            unimplemented!()
        }

        async fn documents(
            &self,
            _auth: String,
            _requests: Vec<DocumentPath>,
        ) -> anyhow::Result<Vec<Document>> {
            unimplemented!()
        }

        async fn document_metadata(
            &self,
            _auth: String,
            _document_paths: Vec<DocumentPath>,
        ) -> anyhow::Result<Vec<Option<Value>>> {
            unimplemented!()
        }
    }
}
