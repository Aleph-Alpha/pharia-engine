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
        tracing_context: TracingContext,
        requests: Vec<ExplanationRequest>,
    ) -> impl Future<Output = Result<Vec<Explanation>, CsiError>> + Send;

    fn complete(
        &self,
        tracing_context: TracingContext,
        requests: Vec<CompletionRequest>,
    ) -> impl Future<Output = anyhow::Result<Vec<Completion>>> + Send;

    fn completion_stream(
        &self,
        tracing_context: TracingContext,
        request: CompletionRequest,
    ) -> impl Future<Output = mpsc::Receiver<Result<CompletionEvent, InferenceError>>> + Send;

    fn chat(
        &self,
        tracing_context: TracingContext,
        requests: Vec<ChatRequest>,
    ) -> impl Future<Output = anyhow::Result<Vec<ChatResponse>>> + Send;

    fn chat_stream(
        &self,
        tracing_context: TracingContext,
        request: ChatRequest,
    ) -> impl Future<Output = mpsc::Receiver<Result<ChatEvent, InferenceError>>> + Send;

    fn chunk(
        &self,
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
        tracing_context: TracingContext,
        requests: Vec<SearchRequest>,
    ) -> impl Future<Output = anyhow::Result<Vec<Vec<SearchResult>>>> + Send;

    fn documents(
        &self,
        tracing_context: TracingContext,
        requests: Vec<DocumentPath>,
    ) -> impl Future<Output = anyhow::Result<Vec<Document>>> + Send;

    fn document_metadata(
        &self,
        tracing_context: TracingContext,
        requests: Vec<DocumentPath>,
    ) -> impl Future<Output = anyhow::Result<Vec<Option<Value>>>> + Send;

    fn invoke_tool(
        &self,
        tracing_context: TracingContext,
        requests: Vec<InvokeRequest>,
    ) -> impl Future<Output = Result<Vec<Value>, ToolError>> + Send;
}

/// Takes a [`RawCsi`] and converts it into a [`ContextualCsi`] by binding the namespace and api
/// token associated with the invocation.
pub struct InvocationContext<R> {
    raw_csi: R,
    // The namespace of the Skill that is being invoked. Required for tool invocations to check the
    // list of mcp servers a Skill may access.
    namespace: Namespace,
    // The authentication provided as part of the Skill invocation.
    api_token: String,
}

impl<R> InvocationContext<R> {
    pub fn new(raw_csi: R, namespace: Namespace, api_token: String) -> Self {
        Self {
            raw_csi,
            namespace,
            api_token,
        }
    }
}

impl<R> ContextualCsi for InvocationContext<R>
where
    R: RawCsi,
{
    fn explain(
        &self,
        tracing_context: TracingContext,
        requests: Vec<ExplanationRequest>,
    ) -> impl Future<Output = Result<Vec<Explanation>, CsiError>> + Send {
        self.raw_csi
            .explain(self.api_token.clone(), tracing_context, requests)
    }

    fn complete(
        &self,
        tracing_context: TracingContext,
        requests: Vec<CompletionRequest>,
    ) -> impl Future<Output = anyhow::Result<Vec<Completion>>> + Send {
        self.raw_csi
            .complete(self.api_token.clone(), tracing_context, requests)
    }

    fn completion_stream(
        &self,
        tracing_context: TracingContext,
        request: CompletionRequest,
    ) -> impl Future<Output = mpsc::Receiver<Result<CompletionEvent, InferenceError>>> + Send {
        self.raw_csi
            .completion_stream(self.api_token.clone(), tracing_context, request)
    }

    fn chat(
        &self,
        tracing_context: TracingContext,
        requests: Vec<ChatRequest>,
    ) -> impl Future<Output = anyhow::Result<Vec<ChatResponse>>> + Send {
        self.raw_csi
            .chat(self.api_token.clone(), tracing_context, requests)
    }

    fn chat_stream(
        &self,
        tracing_context: TracingContext,
        request: ChatRequest,
    ) -> impl Future<Output = mpsc::Receiver<Result<ChatEvent, InferenceError>>> + Send {
        self.raw_csi
            .chat_stream(self.api_token.clone(), tracing_context, request)
    }

    fn chunk(
        &self,
        tracing_context: TracingContext,
        requests: Vec<ChunkRequest>,
    ) -> impl Future<Output = anyhow::Result<Vec<Vec<Chunk>>>> + Send {
        self.raw_csi
            .chunk(self.api_token.clone(), tracing_context, requests)
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
        tracing_context: TracingContext,
        requests: Vec<SearchRequest>,
    ) -> impl Future<Output = anyhow::Result<Vec<Vec<SearchResult>>>> + Send {
        self.raw_csi
            .search(self.api_token.clone(), tracing_context, requests)
    }

    fn documents(
        &self,
        tracing_context: TracingContext,
        requests: Vec<DocumentPath>,
    ) -> impl Future<Output = anyhow::Result<Vec<Document>>> + Send {
        self.raw_csi
            .documents(self.api_token.clone(), tracing_context, requests)
    }

    fn document_metadata(
        &self,
        tracing_context: TracingContext,
        requests: Vec<DocumentPath>,
    ) -> impl Future<Output = anyhow::Result<Vec<Option<Value>>>> + Send {
        self.raw_csi
            .document_metadata(self.api_token.clone(), tracing_context, requests)
    }

    fn invoke_tool(
        &self,
        tracing_context: TracingContext,
        requests: Vec<InvokeRequest>,
    ) -> impl Future<Output = Result<Vec<Value>, ToolError>> + Send {
        self.raw_csi
            .invoke_tool(self.namespace.clone(), tracing_context, requests)
    }
}
