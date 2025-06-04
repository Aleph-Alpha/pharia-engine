use serde_json::Value;
use tokio::sync::mpsc;

use crate::{
    chunking::{Chunk, ChunkRequest},
    csi::{CsiError, RawCsi},
    inference::{
        ChatEvent, ChatRequest, ChatResponse, Completion, CompletionEvent, CompletionRequest,
        Explanation, ExplanationRequest, InferenceError,
    },
    language_selection::{Language, SelectLanguageRequest},
    logging::TracingContext,
    namespace_watcher::Namespace,
    search::{Document, DocumentPath, SearchRequest, SearchResult},
    tool::{InvokeRequest, ToolError},
};

/// An abstraction level on top of [`crate::csi::RawCsi`] that encapsulates context about user
/// invocation and Skill. This includes Skill namespace and authentication.
/// This is the intermediate abstraction level between [`crate::csi::RawCsi`] and
/// [`crate::csi::Csi`].
pub trait ContextualCsi {
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

pub struct InvocationContext<R> {
    raw_csi: R,
}

impl<R> InvocationContext<R> {
    pub fn new(raw_csi: R) -> Self {
        Self { raw_csi }
    }
}

impl<R> ContextualCsi for InvocationContext<R>
where
    R: RawCsi,
{
    fn explain(
        &self,
        auth: String,
        tracing_context: TracingContext,
        requests: Vec<ExplanationRequest>,
    ) -> impl Future<Output = Result<Vec<Explanation>, CsiError>> + Send {
        self.raw_csi.explain(auth, tracing_context, requests)
    }

    fn complete(
        &self,
        auth: String,
        tracing_context: TracingContext,
        requests: Vec<CompletionRequest>,
    ) -> impl Future<Output = anyhow::Result<Vec<Completion>>> + Send {
        self.raw_csi.complete(auth, tracing_context, requests)
    }

    fn completion_stream(
        &self,
        auth: String,
        tracing_context: TracingContext,
        request: CompletionRequest,
    ) -> impl Future<Output = mpsc::Receiver<Result<CompletionEvent, InferenceError>>> + Send {
        self.raw_csi
            .completion_stream(auth, tracing_context, request)
    }

    fn chat(
        &self,
        auth: String,
        tracing_context: TracingContext,
        requests: Vec<ChatRequest>,
    ) -> impl Future<Output = anyhow::Result<Vec<ChatResponse>>> + Send {
        self.raw_csi.chat(auth, tracing_context, requests)
    }

    fn chat_stream(
        &self,
        auth: String,
        tracing_context: TracingContext,
        request: ChatRequest,
    ) -> impl Future<Output = mpsc::Receiver<Result<ChatEvent, InferenceError>>> + Send {
        self.raw_csi.chat_stream(auth, tracing_context, request)
    }

    fn chunk(
        &self,
        auth: String,
        tracing_context: TracingContext,
        requests: Vec<ChunkRequest>,
    ) -> impl Future<Output = anyhow::Result<Vec<Vec<Chunk>>>> + Send {
        self.raw_csi.chunk(auth, tracing_context, requests)
    }

    fn select_language(
        &self,
        requests: Vec<SelectLanguageRequest>,
        tracing_context: TracingContext,
    ) -> impl Future<Output = anyhow::Result<Vec<Option<Language>>>> + Send {
        self.raw_csi.select_language(requests, tracing_context)
    }

    fn search(
        &self,
        auth: String,
        tracing_context: TracingContext,
        requests: Vec<SearchRequest>,
    ) -> impl Future<Output = anyhow::Result<Vec<Vec<SearchResult>>>> + Send {
        self.raw_csi.search(auth, tracing_context, requests)
    }

    fn documents(
        &self,
        auth: String,
        tracing_context: TracingContext,
        requests: Vec<DocumentPath>,
    ) -> impl Future<Output = anyhow::Result<Vec<Document>>> + Send {
        self.raw_csi.documents(auth, tracing_context, requests)
    }

    fn document_metadata(
        &self,
        auth: String,
        tracing_context: TracingContext,
        requests: Vec<DocumentPath>,
    ) -> impl Future<Output = anyhow::Result<Vec<Option<Value>>>> + Send {
        self.raw_csi
            .document_metadata(auth, tracing_context, requests)
    }

    fn invoke_tool(
        &self,
        namespace: Namespace,
        tracing_context: TracingContext,
        requests: Vec<InvokeRequest>,
    ) -> impl Future<Output = Result<Vec<Value>, ToolError>> + Send {
        self.raw_csi
            .invoke_tool(namespace, tracing_context, requests)
    }
}
