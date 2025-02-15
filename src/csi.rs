use async_trait::async_trait;
use futures::future::try_join_all;
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::trace;

use crate::{
    chunking::{self, Chunk, ChunkRequest},
    inference::{
        ChatRequest, ChatResponse, Completion, CompletionRequest, Explanation, ExplanationRequest,
        InferenceApi,
    },
    language_selection::{Language, SelectLanguageRequest, select_language},
    search::{
        Document, DocumentIndexMessage, DocumentPath, SearchApi, SearchRequest, SearchResult,
    },
    tokenizers::TokenizerApi,
};

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

/// Cognitive System Interface (CSI) as consumed by Skill developers. In particular some accidental
/// complexity has been stripped away, by implementations due to removing accidental errors from the
/// interface. It also assumes all authentication and authorization is handled behind the scenes.
/// This is the CSI as passed to user defined code in WASM.
#[async_trait]
pub trait CsiForSkills {
    async fn explain(&mut self, requests: Vec<ExplanationRequest>) -> Vec<Explanation>;
    async fn write(&mut self, data: Vec<u8>);
    async fn complete(&mut self, requests: Vec<CompletionRequest>) -> Vec<Completion>;
    async fn chunk(&mut self, requests: Vec<ChunkRequest>) -> Vec<Vec<Chunk>>;
    async fn select_language(
        &mut self,
        requests: Vec<SelectLanguageRequest>,
    ) -> Vec<Option<Language>>;
    async fn chat(&mut self, requests: Vec<ChatRequest>) -> Vec<ChatResponse>;
    async fn search(&mut self, requests: Vec<SearchRequest>) -> Vec<Vec<SearchResult>>;
    async fn document_metadata(&mut self, document_paths: Vec<DocumentPath>) -> Vec<Option<Value>>;
    async fn documents(&mut self, document_paths: Vec<DocumentPath>) -> Vec<Document>;
}

/// Cognitive System Interface (CSI) as consumed internally by PhariaKernel, before the CSI is
/// passed to the end user in Skill code we further strip away some of the accidental complexity.
/// See its sibling trait `CsiForSkills`. These methods take `Vec`s rather than individual requests
/// in order to allow for parallization behind the scenes.
#[async_trait]
pub trait Csi {
    async fn explain(
        &self,
        auth: String,
        requests: Vec<ExplanationRequest>,
    ) -> anyhow::Result<Vec<Explanation>>;
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
    ) -> anyhow::Result<Vec<Vec<Chunk>>>;

    // While the implementation might not be async, we want the interface to be asynchronous.
    // It is up to the implementer whether the actual implementation is async.
    async fn select_language(
        &self,
        requests: Vec<SelectLanguageRequest>,
    ) -> anyhow::Result<Vec<Option<Language>>> {
        // default implementation can be provided here because language selection is stateless
        Ok(try_join_all(
            requests
                .into_iter()
                .map(|request| tokio::task::spawn_blocking(move || select_language(request))),
        )
        .await?
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?)
    }

    async fn search(
        &self,
        auth: String,
        requests: Vec<SearchRequest>,
    ) -> anyhow::Result<Vec<Vec<SearchResult>>>;

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
    async fn explain(
        &self,
        auth: String,
        requests: Vec<ExplanationRequest>,
    ) -> anyhow::Result<Vec<Explanation>> {
        metrics::counter!(CsiMetrics::CsiRequestsTotal, &[("function", "explain")])
            .increment(requests.len() as u64);
        try_join_all(requests.into_iter().map(|r| {
            trace!(
                "explain: request.model={} request.granularity={}",
                r.model, r.granularity,
            );
            self.inference.explain(r, auth.clone())
        }))
        .await
    }

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
    ) -> anyhow::Result<Vec<Vec<Chunk>>> {
        metrics::counter!(CsiMetrics::CsiRequestsTotal, &[("function", "chunk")])
            .increment(requests.len() as u64);

        try_join_all(requests.into_iter().map(async |request| {
            let text_len = request.text.len();
            let max_tokens = request.params.max_tokens;

            let chunks = chunking::chunking(request, &self.tokenizers, auth.clone()).await?;

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
        requests: Vec<SearchRequest>,
    ) -> anyhow::Result<Vec<Vec<SearchResult>>> {
        metrics::counter!(CsiMetrics::CsiRequestsTotal, &[("function", "search")])
            .increment(requests.len() as u64);

        try_join_all(requests.into_iter().map(|request| {
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
            self.search.search(request, auth.clone())
        }))
        .await
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

    use std::sync::{Arc, Mutex};

    use anyhow::bail;
    use serde_json::json;

    use crate::{
        chunking::ChunkParams,
        inference::{
            ChatParams, CompletionParams, FinishReason, Message, TextScore, TokenUsage,
            tests::InferenceStub,
        },
        search::{TextCursor, tests::SearchStub},
        tests::api_token,
        tokenizers::tests::FakeTokenizers,
    };

    use super::*;

    #[tokio::test]
    async fn chat() {
        // Given a chat request
        let chat_request = ChatRequest {
            model: "dummy_model".to_owned(),
            messages: vec![Message::new("user", "Hello")],
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
            params: ChunkParams {
                model,
                max_tokens,
                overlap: 0,
            },
            character_offsets: false,
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
        async fn explain(
            &self,
            _auth: String,
            _requests: Vec<ExplanationRequest>,
        ) -> anyhow::Result<Vec<Explanation>> {
            panic!("DummyCsi explain called")
        }
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
        ) -> anyhow::Result<Vec<Vec<Chunk>>> {
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
            _requests: Vec<SearchRequest>,
        ) -> anyhow::Result<Vec<Vec<SearchResult>>> {
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
        dyn Fn(Vec<ChunkRequest>) -> anyhow::Result<Vec<Vec<Chunk>>> + Send + Sync + 'static;

    type ExplainFn =
        dyn Fn(ExplanationRequest) -> anyhow::Result<Explanation> + Send + Sync + 'static;

    #[derive(Clone)]
    pub struct StubCsi {
        pub completion: Arc<Box<CompleteFn>>,
        pub chunking: Arc<Box<ChunkFn>>,
        pub explain: Arc<Box<ExplainFn>>,
    }

    impl StubCsi {
        pub fn empty() -> Self {
            StubCsi {
                completion: Arc::new(Box::new(|_| bail!("Completion not set in StubCsi"))),
                chunking: Arc::new(Box::new(|_| bail!("Chunking not set in StubCsi"))),
                explain: Arc::new(Box::new(|_| bail!("Explain not set in StubCsi"))),
            }
        }

        pub fn set_chunking(
            &mut self,
            f: impl Fn(Vec<ChunkRequest>) -> anyhow::Result<Vec<Vec<Chunk>>> + Send + Sync + 'static,
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

        pub fn with_explain(
            f: impl Fn(ExplanationRequest) -> Explanation + Send + Sync + 'static,
        ) -> Self {
            StubCsi {
                explain: Arc::new(Box::new(move |er| Ok(f(er)))),
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
        async fn explain(
            &self,
            _auth: String,
            requests: Vec<ExplanationRequest>,
        ) -> anyhow::Result<Vec<Explanation>> {
            requests.into_iter().map(|r| (*self.explain)(r)).collect()
        }
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
        ) -> anyhow::Result<Vec<Vec<Chunk>>> {
            (*self.chunking)(requests)
        }

        async fn chat(
            &self,
            _auth: String,
            requests: Vec<ChatRequest>,
        ) -> anyhow::Result<Vec<ChatResponse>> {
            Ok(requests
                .into_iter()
                .map(|mut request| ChatResponse {
                    message: request.messages.remove(0),
                    finish_reason: FinishReason::Stop,
                    logprobs: vec![],
                    usage: TokenUsage {
                        prompt: 0,
                        completion: 0,
                    },
                })
                .collect())
        }

        async fn search(
            &self,
            _auth: String,
            _requests: Vec<SearchRequest>,
        ) -> anyhow::Result<Vec<Vec<SearchResult>>> {
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

    /// A test double for a [`Csi`] implementation which always completes with the provided function.
    pub struct CsiCompleteStub {
        complete_fn: Box<dyn FnMut(CompletionRequest) -> Completion + Send>,
    }

    impl CsiCompleteStub {
        pub fn new(complete: impl FnMut(CompletionRequest) -> Completion + Send + 'static) -> Self {
            Self {
                complete_fn: Box::new(complete),
            }
        }
    }

    #[async_trait]
    impl CsiForSkills for CsiCompleteStub {
        async fn write(&mut self, _data: Vec<u8>) {
            unimplemented!()
        }

        async fn complete(&mut self, requests: Vec<CompletionRequest>) -> Vec<Completion> {
            requests
                .into_iter()
                .map(|r| (self.complete_fn)(r))
                .collect()
        }

        async fn chunk(&mut self, _requests: Vec<ChunkRequest>) -> Vec<Vec<Chunk>> {
            unimplemented!()
        }

        async fn select_language(
            &mut self,
            _requests: Vec<SelectLanguageRequest>,
        ) -> Vec<Option<Language>> {
            unimplemented!()
        }

        async fn explain(&mut self, _requests: Vec<ExplanationRequest>) -> Vec<Explanation> {
            unimplemented!()
        }

        async fn chat(&mut self, _requests: Vec<ChatRequest>) -> Vec<ChatResponse> {
            unimplemented!()
        }

        async fn search(&mut self, _requests: Vec<SearchRequest>) -> Vec<Vec<SearchResult>> {
            unimplemented!()
        }

        async fn documents(&mut self, _requests: Vec<DocumentPath>) -> Vec<Document> {
            unimplemented!()
        }

        async fn document_metadata(&mut self, _requests: Vec<DocumentPath>) -> Vec<Option<Value>> {
            unimplemented!()
        }
    }
    /// Asserts a specific prompt and model and returns a greeting message
    #[derive(Clone)]
    pub struct CsiGreetingMock;

    impl CsiGreetingMock {
        fn complete_text(request: CompletionRequest) -> Completion {
            let expected_prompt = "<|begin_of_text|><|start_header_id|>system<|end_header_id|>

Cutting Knowledge Date: December 2023
Today Date: 23 Jul 2024

You are a helpful assistant.<|eot_id|><|start_header_id|>user<|end_header_id|>

Provide a nice greeting for the person named: Homer<|eot_id|><|start_header_id|>assistant<|end_header_id|>";

            let expected_model = "pharia-1-llm-7b-control";

            // Print actual parameters in case of failure
            eprintln!("{request:?}");

            if matches!(request, CompletionRequest{ prompt, model, ..} if model == expected_model && prompt == expected_prompt)
            {
                Completion::from_text("Hello Homer")
            } else {
                Completion::from_text("Mock expectation violated")
            }
        }
    }

    #[async_trait]
    impl CsiForSkills for CsiGreetingMock {
        async fn explain(&mut self, _requests: Vec<ExplanationRequest>) -> Vec<Explanation> {
            vec![Explanation::new(vec![TextScore {
                score: 0.0,
                start: 0,
                length: 0,
            }])]
        }

        async fn write(&mut self, _data: Vec<u8>) {
            unimplemented!()
        }

        async fn complete(&mut self, requests: Vec<CompletionRequest>) -> Vec<Completion> {
            requests.into_iter().map(Self::complete_text).collect()
        }

        async fn chunk(&mut self, _requests: Vec<ChunkRequest>) -> Vec<Vec<Chunk>> {
            unimplemented!()
        }

        async fn select_language(
            &mut self,
            requests: Vec<SelectLanguageRequest>,
        ) -> Vec<Option<Language>> {
            try_join_all(
                requests
                    .into_iter()
                    .map(|request| tokio::task::spawn_blocking(move || select_language(request))),
            )
            .await
            .unwrap()
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
        }

        async fn chat(&mut self, requests: Vec<ChatRequest>) -> Vec<ChatResponse> {
            requests
                .iter()
                .map(|_| ChatResponse {
                    message: Message::new("assistant", "dummy-content"),
                    finish_reason: FinishReason::Stop,
                    logprobs: vec![],
                    usage: TokenUsage {
                        prompt: 0,
                        completion: 0,
                    },
                })
                .collect()
        }

        async fn search(&mut self, requests: Vec<SearchRequest>) -> Vec<Vec<SearchResult>> {
            requests
                .into_iter()
                .map(|request| {
                    let document_path = DocumentPath {
                        namespace: "aleph-alpha".to_owned(),
                        collection: "test-collection".to_owned(),
                        name: "small".to_owned(),
                    };
                    vec![SearchResult {
                        document_path,
                        content: request.query,
                        score: 1.0,
                        start: TextCursor {
                            item: 0,
                            position: 0,
                        },
                        end: TextCursor {
                            item: 0,
                            position: 0,
                        },
                    }]
                })
                .collect()
        }

        async fn documents(&mut self, _requests: Vec<DocumentPath>) -> Vec<Document> {
            vec![Document::dummy()]
        }

        async fn document_metadata(&mut self, _requests: Vec<DocumentPath>) -> Vec<Option<Value>> {
            vec![Some(json!({ "url": "http://example.de" }))]
        }
    }

    #[derive(Default, Clone)]
    pub struct CsiCounter {
        counter: Arc<Mutex<u32>>,
    }

    #[async_trait]
    impl CsiForSkills for CsiCounter {
        async fn write(&mut self, _data: Vec<u8>) {
            unimplemented!()
        }

        async fn complete(&mut self, requests: Vec<CompletionRequest>) -> Vec<Completion> {
            requests
                .iter()
                .map(|_| {
                    let mut counter = self.counter.lock().unwrap();
                    *counter += 1;
                    Completion::from_text(counter.to_string())
                })
                .collect()
        }

        async fn chunk(&mut self, _requests: Vec<ChunkRequest>) -> Vec<Vec<Chunk>> {
            unimplemented!()
        }

        async fn select_language(
            &mut self,
            _requests: Vec<SelectLanguageRequest>,
        ) -> Vec<Option<Language>> {
            unimplemented!()
        }

        async fn chat(&mut self, _requests: Vec<ChatRequest>) -> Vec<ChatResponse> {
            unimplemented!()
        }

        async fn search(&mut self, _requests: Vec<SearchRequest>) -> Vec<Vec<SearchResult>> {
            unimplemented!()
        }

        async fn documents(&mut self, _requests: Vec<DocumentPath>) -> Vec<Document> {
            unimplemented!()
        }

        async fn document_metadata(&mut self, _requests: Vec<DocumentPath>) -> Vec<Option<Value>> {
            unimplemented!()
        }

        async fn explain(&mut self, _requests: Vec<ExplanationRequest>) -> Vec<Explanation> {
            unimplemented!()
        }
    }
}
