use async_trait::async_trait;
use derive_more::{Constructor, From};
use futures::future::try_join_all;
use serde_json::Value;
use thiserror::Error;
use tokio::sync::mpsc;

use crate::{
    chunking::{self, Chunk, ChunkRequest},
    context,
    inference::{
        ChatEvent, ChatRequest, ChatResponse, Completion, CompletionEvent, CompletionRequest,
        Explanation, ExplanationRequest, InferenceApi, InferenceError,
    },
    language_selection::{Language, SelectLanguageRequest, select_language},
    logging::TracingContext,
    namespace_watcher::Namespace,
    search::{Document, DocumentPath, SearchApi, SearchRequest, SearchResult},
    tokenizers::TokenizerApi,
    tool::{InvokeRequest, ToolApi, ToolError},
};

#[cfg(test)]
use double_derive::double;

/// `CompletionStreamId` is a unique identifier for a completion stream.
#[derive(Debug, Clone, Constructor, Copy, From, PartialEq, Eq, Hash)]
pub struct CompletionStreamId(usize);
/// `ChatStreamId` is a unique identifier for a chat stream.
#[derive(Debug, Clone, Constructor, Copy, From, PartialEq, Eq, Hash)]
pub struct ChatStreamId(usize);

/// Collection of api handles to the actors used to implement the Cognitive System Interface (CSI)
///
/// For now this is just a collection of all the APIs without providing logic on its own
#[derive(Clone)]
pub struct CsiDrivers<I, S, Tz, Tl> {
    /// We use the inference Api to complete text
    pub inference: I,
    pub search: S,
    pub tokenizers: Tz,
    pub tool: Tl,
}

/// Cognitive System Interface (CSI) as consumed by Skill developers. In particular some accidental
/// complexity has been stripped away, by implementations due to removing accidental errors from the
/// interface. It also assumes all authentication and authorization is handled behind the scenes.
/// This is the CSI as passed to user defined code in WASM.
#[async_trait]
#[cfg_attr(test, double(CsiForSkillsDouble))]
pub trait CsiForSkills {
    async fn explain(&mut self, requests: Vec<ExplanationRequest>) -> Vec<Explanation>;
    async fn complete(&mut self, requests: Vec<CompletionRequest>) -> Vec<Completion>;
    async fn completion_stream_new(&mut self, request: CompletionRequest) -> CompletionStreamId;
    async fn completion_stream_next(&mut self, id: &CompletionStreamId) -> Option<CompletionEvent>;
    async fn completion_stream_drop(&mut self, id: CompletionStreamId);
    async fn chunk(&mut self, requests: Vec<ChunkRequest>) -> Vec<Vec<Chunk>>;
    async fn select_language(
        &mut self,
        requests: Vec<SelectLanguageRequest>,
    ) -> Vec<Option<Language>>;
    async fn chat(&mut self, requests: Vec<ChatRequest>) -> Vec<ChatResponse>;
    async fn chat_stream_new(&mut self, request: ChatRequest) -> ChatStreamId;
    async fn chat_stream_next(&mut self, id: &ChatStreamId) -> Option<ChatEvent>;
    async fn chat_stream_drop(&mut self, id: ChatStreamId);
    async fn search(&mut self, requests: Vec<SearchRequest>) -> Vec<Vec<SearchResult>>;
    async fn document_metadata(&mut self, document_paths: Vec<DocumentPath>) -> Vec<Option<Value>>;
    async fn documents(&mut self, document_paths: Vec<DocumentPath>) -> Vec<Document>;
    async fn invoke_tool(&mut self, request: Vec<InvokeRequest>) -> Vec<Value>;
}

/// Cognitive System Interface (CSI) as consumed internally by `PhariaKernel`, before the CSI is
/// passed to the end user in Skill code we further strip away some of the accidental complexity.
/// See its sibling trait `CsiForSkills`. These methods take `Vec`s rather than individual requests
/// in order to allow for parallization behind the scenes.
#[cfg_attr(test, double(CsiDouble))]
pub trait Csi {
    fn explain(
        &self,
        auth: String,
        tracing_context: TracingContext,
        requests: Vec<ExplanationRequest>,
    ) -> impl Future<Output = Result<Vec<Explanation>, CsiError>> + Send;

    fn complete(
        &self,
        auth: String,
        tracing_context: TracingContext,
        requests: Vec<CompletionRequest>,
    ) -> impl Future<Output = anyhow::Result<Vec<Completion>>> + Send;

    fn completion_stream(
        &self,
        auth: String,
        tracing_context: TracingContext,
        request: CompletionRequest,
    ) -> impl Future<Output = mpsc::Receiver<Result<CompletionEvent, InferenceError>>> + Send;

    fn chat(
        &self,
        auth: String,
        tracing_context: TracingContext,
        requests: Vec<ChatRequest>,
    ) -> impl Future<Output = anyhow::Result<Vec<ChatResponse>>> + Send;

    fn chat_stream(
        &self,
        auth: String,
        tracing_context: TracingContext,
        request: ChatRequest,
    ) -> impl Future<Output = mpsc::Receiver<Result<ChatEvent, InferenceError>>> + Send;

    fn chunk(
        &self,
        auth: String,
        tracing_context: TracingContext,
        requests: Vec<ChunkRequest>,
    ) -> impl Future<Output = anyhow::Result<Vec<Vec<Chunk>>>> + Send;

    // While the implementation might not be async, we want the interface to be asynchronous.
    // It is up to the implementer whether the actual implementation is async.
    fn select_language(
        &self,
        requests: Vec<SelectLanguageRequest>,
        tracing_context: TracingContext,
    ) -> impl Future<Output = anyhow::Result<Vec<Option<Language>>>> + Send;

    fn search(
        &self,
        auth: String,
        tracing_context: TracingContext,
        requests: Vec<SearchRequest>,
    ) -> impl Future<Output = anyhow::Result<Vec<Vec<SearchResult>>>> + Send;

    fn documents(
        &self,
        auth: String,
        tracing_context: TracingContext,
        requests: Vec<DocumentPath>,
    ) -> impl Future<Output = anyhow::Result<Vec<Document>>> + Send;

    fn document_metadata(
        &self,
        auth: String,
        tracing_context: TracingContext,
        requests: Vec<DocumentPath>,
    ) -> impl Future<Output = anyhow::Result<Vec<Option<Value>>>> + Send;

    fn invoke_tool(
        &self,
        namespace: Namespace,
        tracing_context: TracingContext,
        requests: Vec<InvokeRequest>,
    ) -> impl Future<Output = Result<Vec<Value>, ToolError>> + Send;
}

/// Errors which occurr during interacting with the outside world via CSIs.
#[derive(Debug, Error)]
pub enum CsiError {
    #[error("We got an error from our inference driver:\n\n{0}")]
    Inference(#[from] InferenceError),
    #[error(transparent)]
    Any(#[from] anyhow::Error),
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

impl<I, S, Tz, Tl> Csi for CsiDrivers<I, S, Tz, Tl>
where
    I: InferenceApi + Send + Sync,
    S: SearchApi + Send + Sync,
    Tz: TokenizerApi + Send + Sync,
    Tl: ToolApi + Send + Sync,
{
    async fn explain(
        &self,
        auth: String,
        tracing_context: TracingContext,
        requests: Vec<ExplanationRequest>,
    ) -> Result<Vec<Explanation>, CsiError> {
        metrics::counter!(CsiMetrics::CsiRequestsTotal, &[("function", "explain")])
            .increment(requests.len() as u64);

        let tracing_context = if requests.len() > 1 {
            context!(
                tracing_context,
                "pharia-kernel::csi",
                "explain_concurrent",
                requests = requests.len()
            )
        } else {
            tracing_context
        };

        let explanations = try_join_all(requests.into_iter().map(|r| {
            let child_context = context!(
                tracing_context,
                "pharia-kernel::csi",
                "explain",
                model = r.model
            );
            self.inference.explain(r, auth.clone(), child_context)
        }))
        .await?;
        Ok(explanations)
    }

    async fn complete(
        &self,
        auth: String,
        tracing_context: TracingContext,
        requests: Vec<CompletionRequest>,
    ) -> anyhow::Result<Vec<Completion>> {
        metrics::counter!(CsiMetrics::CsiRequestsTotal, &[("function", "complete")])
            .increment(requests.len() as u64);

        let tracing_context = if requests.len() > 1 {
            context!(
                tracing_context,
                "pharia-kernel::csi",
                "complete_concurrent",
                requests = requests.len()
            )
        } else {
            tracing_context
        };

        let completions = try_join_all(
            requests
                .into_iter()
                .map(|r| {
                    let child_context = context!(
                        tracing_context,
                        "pharia-kernel::csi",
                        "complete",
                        model = r.model
                    );
                    self.inference.complete(r, auth.clone(), child_context)
                })
                .collect::<Vec<_>>(),
        )
        .await?;
        Ok(completions)
    }

    async fn completion_stream(
        &self,
        auth: String,
        tracing_context: TracingContext,
        request: CompletionRequest,
    ) -> mpsc::Receiver<Result<CompletionEvent, InferenceError>> {
        metrics::counter!(
            CsiMetrics::CsiRequestsTotal,
            &[("function", "completion_stream")]
        )
        .increment(1);

        let context = context!(
            tracing_context,
            "pharia-kernel::csi",
            "completion_stream",
            model = request.model
        );
        self.inference
            .completion_stream(request, auth, context)
            .await
    }

    async fn chat(
        &self,
        auth: String,
        tracing_context: TracingContext,
        requests: Vec<ChatRequest>,
    ) -> anyhow::Result<Vec<ChatResponse>> {
        metrics::counter!(CsiMetrics::CsiRequestsTotal, &[("function", "chat")])
            .increment(requests.len() as u64);

        let tracing_context = if requests.len() > 1 {
            context!(
                tracing_context,
                "pharia-kernel::csi",
                "chat_concurrent",
                requests = requests.len()
            )
        } else {
            tracing_context
        };

        let responses = try_join_all(
            requests
                .into_iter()
                .map(|r| {
                    let context = context!(
                        tracing_context,
                        "pharia-kernel::csi",
                        "chat",
                        model = r.model
                    );
                    self.inference.chat(r, auth.clone(), context)
                })
                .collect::<Vec<_>>(),
        )
        .await?;
        Ok(responses)
    }

    async fn chat_stream(
        &self,
        auth: String,
        tracing_context: TracingContext,
        request: ChatRequest,
    ) -> mpsc::Receiver<Result<ChatEvent, InferenceError>> {
        metrics::counter!(CsiMetrics::CsiRequestsTotal, &[("function", "chat_stream")])
            .increment(1);

        let context = context!(
            tracing_context,
            "pharia-kernel::csi",
            "chat_stream",
            model = request.model
        );
        self.inference.chat_stream(request, auth, context).await
    }

    async fn chunk(
        &self,
        auth: String,
        tracing_context: TracingContext,
        requests: Vec<ChunkRequest>,
    ) -> anyhow::Result<Vec<Vec<Chunk>>> {
        metrics::counter!(CsiMetrics::CsiRequestsTotal, &[("function", "chunk")])
            .increment(requests.len() as u64);

        let tracing_context = if requests.len() > 1 {
            context!(
                tracing_context,
                "pharia-kernel::csi",
                "chunk_concurrent",
                requests = requests.len()
            )
        } else {
            tracing_context
        };

        try_join_all(requests.into_iter().map(async |request| {
            let text_len = request.text.len();
            let max_tokens = request.params.max_tokens;
            let context = context!(
                tracing_context,
                "pharia-kernel::csi",
                "chunk",
                text_len = text_len,
                max_tokens = max_tokens
            );
            let chunks =
                chunking::chunking(request, &self.tokenizers, auth.clone(), context).await?;
            Ok(chunks)
        }))
        .await
    }

    async fn select_language(
        &self,
        requests: Vec<SelectLanguageRequest>,
        tracing_context: TracingContext,
    ) -> anyhow::Result<Vec<Option<Language>>> {
        metrics::counter!(
            CsiMetrics::CsiRequestsTotal,
            &[("function", "select_language")]
        )
        .increment(requests.len() as u64);

        let tracing_context = if requests.len() > 1 {
            context!(
                tracing_context,
                "pharia-kernel::csi",
                "select_language_concurrent",
                requests = requests.len()
            )
        } else {
            tracing_context
        };

        Ok(try_join_all(requests.into_iter().map(|request| {
            let context = context!(
                tracing_context,
                "pharia-kernel::csi",
                "select_language",
                text_len = request.text.len(),
                languages = request.languages.len()
            );
            tokio::task::spawn_blocking(move || select_language(request, context))
        }))
        .await?
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?)
    }

    async fn search(
        &self,
        auth: String,
        tracing_context: TracingContext,
        requests: Vec<SearchRequest>,
    ) -> anyhow::Result<Vec<Vec<SearchResult>>> {
        metrics::counter!(CsiMetrics::CsiRequestsTotal, &[("function", "search")])
            .increment(requests.len() as u64);

        let tracing_context = if requests.len() > 1 {
            context!(
                tracing_context,
                "pharia-kernel::csi",
                "search_concurrent",
                requests = requests.len()
            )
        } else {
            tracing_context
        };

        try_join_all(requests.into_iter().map(|request| {
            let index_path = &request.index_path;
            let context = context!(
                tracing_context,
                "pharia-kernel::csi",
                "search",
                namespace = index_path.namespace,
                collection = index_path.collection,
                max_results = request.max_results,
                min_score = request
                    .min_score
                    .map_or_else(|| "None".to_owned(), |val| val.to_string())
            );
            self.search.search(request, auth.clone(), context)
        }))
        .await
    }

    async fn documents(
        &self,
        auth: String,
        tracing_context: TracingContext,
        requests: Vec<DocumentPath>,
    ) -> anyhow::Result<Vec<Document>> {
        metrics::counter!(CsiMetrics::CsiRequestsTotal, &[("function", "documents")])
            .increment(requests.len() as u64);

        let tracing_context = if requests.len() > 1 {
            context!(
                tracing_context,
                "pharia-kernel::csi",
                "document_concurrent",
                requests = requests.len()
            )
        } else {
            tracing_context
        };

        try_join_all(
            requests
                .into_iter()
                .map(|r| {
                    let context = context!(
                        tracing_context,
                        "pharia-kernel::csi",
                        "document",
                        namespace = r.namespace,
                        collection = r.collection,
                        name = r.name
                    );
                    self.search.document(r, auth.clone(), context)
                })
                .collect::<Vec<_>>(),
        )
        .await
    }

    async fn document_metadata(
        &self,
        auth: String,
        tracing_context: TracingContext,
        requests: Vec<DocumentPath>,
    ) -> anyhow::Result<Vec<Option<Value>>> {
        metrics::counter!(
            CsiMetrics::CsiRequestsTotal,
            &[("function", "document_metadata")]
        )
        .increment(requests.len() as u64);

        let tracing_context = if requests.len() > 1 {
            context!(
                tracing_context,
                "pharia-kernel::csi",
                "document_metadata_concurrent",
                requests = requests.len()
            )
        } else {
            tracing_context
        };

        try_join_all(
            requests
                .into_iter()
                .map(|r| {
                    let context = context!(
                        tracing_context,
                        "pharia-kernel::csi",
                        "document_metadata",
                        namespace = r.namespace,
                        collection = r.collection,
                        name = r.name
                    );
                    self.search.document_metadata(r, auth.clone(), context)
                })
                .collect::<Vec<_>>(),
        )
        .await
    }

    async fn invoke_tool(
        &self,
        namespace: Namespace,
        tracing_context: TracingContext,
        requests: Vec<InvokeRequest>,
    ) -> Result<Vec<Value>, ToolError> {
        metrics::counter!(CsiMetrics::CsiRequestsTotal, &[("function", "invoke_tool")])
            .increment(requests.len() as u64);

        let tracing_context = if requests.len() > 1 {
            context!(
                tracing_context,
                "pharia-kernel::csi",
                "invoke_tool_concurrent",
                requests = requests.len()
            )
        } else {
            tracing_context
        };

        try_join_all(
            requests
                .into_iter()
                .map(|request| {
                    let context = context!(
                        tracing_context,
                        "pharia-kernel::csi",
                        "invoke_tool",
                        tool_name = request.tool_name
                    );
                    self.tool.invoke_tool(request, namespace.clone(), context)
                })
                .collect::<Vec<_>>(),
        )
        .await
    }
}

#[cfg(test)]
pub mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use anyhow::{anyhow, bail};

    use crate::{
        chunking::ChunkParams,
        inference::{
            ChatEvent, ChatParams, CompletionEvent, CompletionParams, FinishReason, Message,
            TokenUsage, tests::InferenceStub,
        },
        search::{TextCursor, tests::SearchStub},
        tests::api_token,
        tokenizers::tests::FakeTokenizers,
        tool::tests::ToolDummy,
    };

    use super::*;

    #[derive(Clone)]
    pub struct CsiSaboteur;

    impl CsiDouble for CsiSaboteur {
        async fn complete(
            &self,
            _auth: String,
            _tracing_context: TracingContext,
            _requests: Vec<CompletionRequest>,
        ) -> anyhow::Result<Vec<Completion>> {
            bail!("Test error")
        }

        async fn chat_stream(
            &self,
            _auth: String,
            _tracing_context: TracingContext,
            _request: ChatRequest,
        ) -> mpsc::Receiver<Result<ChatEvent, InferenceError>> {
            let (send, recv) = mpsc::channel(1);
            send.send(Err(InferenceError::Other(anyhow!("Test error"))))
                .await
                .unwrap();
            recv
        }
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
        let model = "pharia-1-llm-7B-control".to_owned();
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
            .chunk(
                "dummy_token".to_owned(),
                TracingContext::dummy(),
                vec![request],
            )
            .await
            .unwrap();

        // Then a single chunk is returned
        assert_eq!(chunks[0].len(), 1);
    }

    #[tokio::test]
    async fn documents() {
        // Given a skill invocation context with a stub tokenizer provider
        let search = SearchStub::new().with_document(|_| Ok(Document::dummy()));
        let csi_apis = CsiDrivers {
            search,
            ..dummy_csi_drivers()
        };

        // When requesting documents
        let request = vec![DocumentPath {
            namespace: "kernel".to_owned(),
            collection: "test".to_owned(),
            name: "docs".to_owned(),
        }];
        let documents = csi_apis
            .documents("dummy_token".to_owned(), TracingContext::dummy(), request)
            .await
            .unwrap();

        // Then a single document is returned
        assert_eq!(documents.len(), 1);
    }

    #[tokio::test]
    async fn completion_stream_events() {
        // Given a CSI drivers with stub completion
        let inference = InferenceStub::new().with_complete(|r| Ok(Completion::from_text(r.prompt)));
        let csi_apis = CsiDrivers {
            inference,
            ..dummy_csi_drivers()
        };

        // When requesting a streamed completion
        let completion_req = CompletionRequest {
            model: "dummy_model".to_owned(),
            prompt: "request".to_owned(),
            params: CompletionParams::default(),
        };

        let mut completion = csi_apis
            .completion_stream(
                api_token().to_owned(),
                TracingContext::dummy(),
                completion_req,
            )
            .await;

        let mut events = vec![];
        while let Some(Ok(event)) = completion.recv().await {
            events.push(event);
        }

        drop(csi_apis);

        // Then the completion must have the same order as the respective requests
        assert_eq!(events.len(), 3);
        assert!(matches!(events[0], CompletionEvent::Append { .. }));
        assert!(matches!(events[1], CompletionEvent::End { .. }));
        assert!(matches!(events[2], CompletionEvent::Usage { .. }));
    }

    #[tokio::test]
    async fn chat_stream_events() {
        // Given a CSI drivers with stub completion
        let inference = InferenceStub::new().with_chat(|_| {
            Ok(ChatResponse {
                message: Message {
                    role: "assistant".to_owned(),
                    content: "Hello".to_owned(),
                },
                finish_reason: FinishReason::Stop,
                logprobs: vec![],
                usage: TokenUsage {
                    prompt: 1,
                    completion: 1,
                },
            })
        });
        let csi_apis = CsiDrivers {
            inference,
            ..dummy_csi_drivers()
        };

        // When requesting a streamed completion
        let chat_req = ChatRequest {
            model: "dummy_model".to_owned(),
            messages: vec![Message {
                role: "user".to_owned(),
                content: "request".to_owned(),
            }],
            params: ChatParams::default(),
        };

        let mut chat = csi_apis
            .chat_stream(api_token().to_owned(), TracingContext::dummy(), chat_req)
            .await;

        let mut events = vec![];
        while let Some(Ok(event)) = chat.recv().await {
            events.push(event);
        }

        drop(csi_apis);

        // Then the completion must have the same order as the respective requests
        assert_eq!(events.len(), 4);
        assert!(matches!(events[0], ChatEvent::MessageBegin { .. }));
        assert!(matches!(events[1], ChatEvent::MessageAppend { .. }));
        assert!(matches!(events[2], ChatEvent::MessageEnd { .. }));
        assert!(matches!(events[3], ChatEvent::Usage { .. }));
    }

    #[tokio::test]
    async fn completion_requests_in_respective_order() {
        // Given a CSI drivers with stub completion
        let inference = InferenceStub::new().with_complete(|r| Ok(Completion::from_text(r.prompt)));
        let csi_apis = CsiDrivers {
            inference,
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
                TracingContext::dummy(),
                vec![completion_req_1, completion_req_2],
            )
            .await
            .unwrap();

        drop(csi_apis);

        // Then the completion must have the same order as the respective requests
        assert_eq!(completions.len(), 2);
        assert!(completions.first().unwrap().text.contains("1st"));
        assert!(completions.get(1).unwrap().text.contains("2nd"));
    }

    #[tokio::test]
    async fn document_metadata_requests_in_respective_order() {
        // Given a CSI drivers with stub search
        let search =
            SearchStub::new().with_document_metadata(|r| Ok(Some(Value::String(r.name.clone()))));
        let csi_apis = CsiDrivers {
            search,
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
            .document_metadata(
                api_token().to_owned(),
                TracingContext::dummy(),
                vec![request_1, request_2],
            )
            .await
            .unwrap();

        drop(csi_apis);

        // Then the responses must have the same order as the respective requests
        let responses: Vec<String> = responses
            .into_iter()
            .map(|v| v.unwrap().to_string())
            .collect();

        assert_eq!(responses.len(), 2);
        assert!(responses[0].contains("1st"));
        assert!(responses[1].contains("2nd"));
    }

    fn dummy_csi_drivers() -> CsiDrivers<InferenceStub, SearchStub, FakeTokenizers, ToolDummy> {
        CsiDrivers {
            inference: InferenceStub::new(),
            search: SearchStub::new(),
            tokenizers: FakeTokenizers,
            tool: ToolDummy,
        }
    }

    #[derive(Clone)]
    pub struct CsiDummy;

    impl CsiDouble for CsiDummy {}

    type ChatFn = dyn Fn(ChatRequest) -> anyhow::Result<ChatResponse> + Send + Sync + 'static;

    type CompleteFn =
        dyn Fn(CompletionRequest) -> anyhow::Result<Completion> + Send + Sync + 'static;

    type ChunkFn =
        dyn Fn(Vec<ChunkRequest>) -> anyhow::Result<Vec<Vec<Chunk>>> + Send + Sync + 'static;

    type ExplainFn =
        dyn Fn(ExplanationRequest) -> Result<Explanation, CsiError> + Send + Sync + 'static;

    #[derive(Clone)]
    pub struct StubCsi {
        pub chat: Arc<Box<ChatFn>>,
        pub completion: Arc<Box<CompleteFn>>,
        pub chunking: Arc<Box<ChunkFn>>,
        pub explain: Arc<Box<ExplainFn>>,
    }

    impl StubCsi {
        pub fn empty() -> Self {
            StubCsi {
                chat: Arc::new(Box::new(|_| bail!("Chat not set in StubCsi"))),
                completion: Arc::new(Box::new(|_| bail!("Completion not set in StubCsi"))),
                chunking: Arc::new(Box::new(|_| bail!("Chunking not set in StubCsi"))),
                explain: Arc::new(Box::new(|_| unimplemented!("Explain not set in StubCsi"))),
            }
        }

        pub fn set_chunking(
            &mut self,
            f: impl Fn(Vec<ChunkRequest>) -> anyhow::Result<Vec<Vec<Chunk>>> + Send + Sync + 'static,
        ) {
            self.chunking = Arc::new(Box::new(f));
        }

        pub fn with_chat(f: impl Fn(ChatRequest) -> ChatResponse + Send + Sync + 'static) -> Self {
            StubCsi {
                chat: Arc::new(Box::new(move |cr| Ok(f(cr)))),
                ..Self::empty()
            }
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
    }

    impl CsiDouble for StubCsi {
        async fn explain(
            &self,
            _auth: String,
            _tracing_context: TracingContext,
            requests: Vec<ExplanationRequest>,
        ) -> Result<Vec<Explanation>, CsiError> {
            requests.into_iter().map(|r| (*self.explain)(r)).collect()
        }
        async fn complete(
            &self,
            _auth: String,
            _tracing_context: TracingContext,
            requests: Vec<CompletionRequest>,
        ) -> anyhow::Result<Vec<Completion>> {
            requests
                .into_iter()
                .map(|r| (*self.completion)(r))
                .collect()
        }

        async fn completion_stream(
            &self,
            _auth: String,
            _tracing_context: TracingContext,
            request: CompletionRequest,
        ) -> mpsc::Receiver<Result<CompletionEvent, InferenceError>> {
            let (sender, receiver) = mpsc::channel(1);
            let Completion {
                text,
                finish_reason,
                logprobs,
                usage,
            } = (*self.completion)(request).unwrap();
            tokio::spawn(async move {
                sender
                    .send(Ok(CompletionEvent::Append { text, logprobs }))
                    .await
                    .unwrap();
                sender
                    .send(Ok(CompletionEvent::End { finish_reason }))
                    .await
                    .unwrap();
                sender
                    .send(Ok(CompletionEvent::Usage { usage }))
                    .await
                    .unwrap();
            });
            receiver
        }

        async fn chunk(
            &self,
            _auth: String,
            _tracing_context: TracingContext,
            requests: Vec<ChunkRequest>,
        ) -> anyhow::Result<Vec<Vec<Chunk>>> {
            (*self.chunking)(requests)
        }

        async fn chat(
            &self,
            _auth: String,
            _tracing_context: TracingContext,
            requests: Vec<ChatRequest>,
        ) -> anyhow::Result<Vec<ChatResponse>> {
            requests.into_iter().map(|r| (*self.chat)(r)).collect()
        }

        async fn chat_stream(
            &self,
            _auth: String,
            _tracing_context: TracingContext,
            request: ChatRequest,
        ) -> mpsc::Receiver<Result<ChatEvent, InferenceError>> {
            let (sender, receiver) = mpsc::channel(1);
            let ChatResponse {
                message,
                finish_reason,
                logprobs,
                usage,
            } = (*self.chat)(request).unwrap();
            tokio::spawn(async move {
                sender
                    .send(Ok(ChatEvent::MessageBegin { role: message.role }))
                    .await
                    .unwrap();
                sender
                    .send(Ok(ChatEvent::MessageAppend {
                        content: message.content,
                        logprobs,
                    }))
                    .await
                    .unwrap();
                sender
                    .send(Ok(ChatEvent::MessageEnd { finish_reason }))
                    .await
                    .unwrap();
                sender.send(Ok(ChatEvent::Usage { usage })).await.unwrap();
            });
            receiver
        }

        async fn select_language(
            &self,
            requests: Vec<SelectLanguageRequest>,
            _tracing_context: TracingContext,
        ) -> anyhow::Result<Vec<Option<Language>>> {
            Ok(try_join_all(requests.into_iter().map(|request| {
                tokio::task::spawn_blocking(move || {
                    select_language(request, TracingContext::dummy())
                })
            }))
            .await?
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?)
        }
    }

    pub struct CsiCompleteStreamStub {
        current_id: usize,
        events: Vec<CompletionEvent>,
        streams: HashMap<CompletionStreamId, Vec<CompletionEvent>>,
    }

    impl CsiCompleteStreamStub {
        pub fn new(mut events: Vec<CompletionEvent>) -> Self {
            events.reverse();
            Self {
                current_id: 0,
                events,
                streams: HashMap::new(),
            }
        }
    }

    #[async_trait]
    impl CsiForSkillsDouble for CsiCompleteStreamStub {
        async fn completion_stream_new(
            &mut self,
            _request: CompletionRequest,
        ) -> CompletionStreamId {
            let id = CompletionStreamId::new(self.current_id);
            self.streams.insert(id, self.events.clone());
            self.current_id += 1;
            id
        }

        async fn completion_stream_next(
            &mut self,
            id: &CompletionStreamId,
        ) -> Option<CompletionEvent> {
            self.streams.get_mut(id)?.pop()
        }

        async fn completion_stream_drop(&mut self, id: CompletionStreamId) {
            self.streams.remove(&id);
        }
    }

    pub struct CsiChatStreamStub {
        current_id: usize,
        events: Vec<ChatEvent>,
        streams: HashMap<ChatStreamId, Vec<ChatEvent>>,
    }

    impl CsiChatStreamStub {
        pub fn new(mut events: Vec<ChatEvent>) -> Self {
            events.reverse();
            Self {
                current_id: 0,
                events,
                streams: HashMap::new(),
            }
        }
    }

    #[async_trait]
    impl CsiForSkillsDouble for CsiChatStreamStub {
        async fn chat_stream_new(&mut self, _request: ChatRequest) -> ChatStreamId {
            let id = ChatStreamId::new(self.current_id);
            self.streams.insert(id, self.events.clone());
            self.current_id += 1;
            id
        }

        async fn chat_stream_next(&mut self, id: &ChatStreamId) -> Option<ChatEvent> {
            self.streams.get_mut(id)?.pop()
        }

        async fn chat_stream_drop(&mut self, id: ChatStreamId) {
            self.streams.remove(&id);
        }
    }

    /// Assert that the echo parameter is set to true and return the prompt as the completion.
    pub struct CsiCompleteWithEchoMock;

    #[async_trait]
    impl CsiForSkillsDouble for CsiCompleteWithEchoMock {
        async fn complete(&mut self, requests: Vec<CompletionRequest>) -> Vec<Completion> {
            requests
                .into_iter()
                .map(|request| {
                    assert!(
                        request.params.echo,
                        "`CsiCompleteWithEchoMock` requires the `echo` parameter to be set to true"
                    );
                    Completion::from_text(request.prompt)
                })
                .collect()
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
    impl CsiForSkillsDouble for CsiGreetingMock {
        async fn complete(&mut self, requests: Vec<CompletionRequest>) -> Vec<Completion> {
            requests.into_iter().map(Self::complete_text).collect()
        }
    }

    /// Always return a hardcoded dummy response
    pub struct CsiChatStub;

    #[async_trait]
    impl CsiForSkillsDouble for CsiChatStub {
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
    }

    /// Return the content of the query as a search result
    pub struct CsiSearchMock;

    #[async_trait]
    impl CsiForSkillsDouble for CsiSearchMock {
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
    }

    #[derive(Default, Clone)]
    pub struct CsiCounter {
        counter: Arc<Mutex<u32>>,
    }

    #[async_trait]
    impl CsiForSkillsDouble for CsiCounter {
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
    }
}
