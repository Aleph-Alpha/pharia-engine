use std::{
    borrow::Cow,
    fmt,
    future::{Future, pending},
    pin::Pin,
    sync::Arc,
    time::Instant,
};

use aleph_alpha_client::{ChatChunk, StreamChatEvent, StreamMessage};
use anyhow::anyhow;
use async_stream::stream;
use async_trait::async_trait;
use futures::{Stream, StreamExt, stream::FuturesUnordered};
use opentelemetry::Context;
use serde_json::Value;
use tokio::{
    pin, select,
    sync::{mpsc, oneshot},
    task::JoinHandle,
};
use tracing::{Level, span};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use crate::{
    chunking::{Chunk, ChunkRequest},
    csi::{Csi, CsiForSkills},
    inference::{
        ChatRequest, ChatResponse, ChatStream, Completion, CompletionRequest, Explanation,
        ExplanationRequest,
    },
    language_selection::{Language, SelectLanguageRequest},
    search::{Document, DocumentPath, SearchRequest, SearchResult},
    skill_store::{SkillStoreApi, SkillStoreError},
    skills::{Engine, SkillMetadata, SkillPath},
};

pub struct WasmRuntime {
    /// Used to execute skills. We will share the engine with multiple running skills, and skill
    /// provider to convert bytes into executable skills.
    engine: Arc<Engine>,
    skill_store_api: SkillStoreApi,
}

impl WasmRuntime {
    pub fn new(engine: Arc<Engine>, skill_store_api: SkillStoreApi) -> Self {
        Self {
            engine,
            skill_store_api,
        }
    }

    pub async fn run(
        &self,
        skill_path: &SkillPath,
        input: Value,
        ctx: Box<dyn CsiForSkills + Send>,
    ) -> Result<Value, SkillRuntimeError> {
        let skill = self.skill_store_api.fetch(skill_path.to_owned()).await?;
        // Unwrap Skill, raise error if it is not existing
        let skill = skill.ok_or(SkillRuntimeError::SkillNotConfigured)?;
        Ok(skill.run(&self.engine, ctx, input).await?)
    }

    pub async fn metadata(
        &self,
        skill_path: &SkillPath,
        ctx: Box<dyn CsiForSkills + Send>,
    ) -> Result<Option<SkillMetadata>, SkillRuntimeError> {
        let skill = self.skill_store_api.fetch(skill_path.to_owned()).await?;
        // Unwrap Skill, raise error if it is not existing
        let skill = skill.ok_or(SkillRuntimeError::SkillNotConfigured)?;
        Ok(skill.metadata(self.engine.as_ref(), ctx).await?)
    }
}

/// Starts and stops the execution of skills as it owns the skill runtime actor.
pub struct SkillRuntime {
    send: mpsc::Sender<SkillRuntimeMsg>,
    handle: JoinHandle<()>,
}

impl SkillRuntime {
    /// Create a new skill runtime with the default web assembly runtime
    pub fn new<C>(engine: Arc<Engine>, csi_apis: C, skill_store_api: SkillStoreApi) -> Self
    where
        C: Csi + Clone + Send + Sync + 'static,
    {
        let runtime = WasmRuntime::new(engine, skill_store_api);
        let (send, recv) = mpsc::channel::<SkillRuntimeMsg>(1);
        let handle = tokio::spawn(async {
            SkillRuntimeActor::new(runtime, recv, csi_apis).run().await;
        });
        SkillRuntime { send, handle }
    }

    /// Retrieve a handle in order to interact with skills. All handles have to be dropped in order
    /// for [`Self::wait_for_shutdown`] to complete.
    pub fn api(&self) -> SkillRuntimeApi {
        SkillRuntimeApi::new(self.send.clone())
    }

    pub async fn wait_for_shutdown(self) {
        drop(self.send);
        self.handle.await.unwrap();
    }
}

#[derive(Clone)]
pub struct SkillRuntimeApi {
    send: mpsc::Sender<SkillRuntimeMsg>,
}

impl SkillRuntimeApi {
    pub fn new(send: mpsc::Sender<SkillRuntimeMsg>) -> Self {
        Self { send }
    }

    /// Run a skill and receive either a single value or a stream of intermediate results.
    pub async fn skill_run(
        &self,
        skill_path: SkillPath,
        input: Value,
        api_token: String,
    ) -> Result<SkillOutput, SkillRuntimeError> {
        let (send, recv) = oneshot::channel();
        let msg = SkillRuntimeMsg::Run(SkillRunRequest {
            skill_path,
            input,
            send,
            api_token,
        });
        self.send
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }

    pub async fn skill_metadata(
        &self,
        skill_path: SkillPath,
    ) -> Result<Option<SkillMetadata>, SkillRuntimeError> {
        let (send, recv) = oneshot::channel();
        let msg = SkillRuntimeMsg::Metadata(SkillMetadataRequest { skill_path, send });
        self.send
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SkillRuntimeError {
    #[error(
        "The requested skill does not exist. Make sure it is configured in the configuration \
        associated with the namespace."
    )]
    SkillNotConfigured,
    #[error(transparent)]
    StoreError(#[from] SkillStoreError),
    #[error(transparent)]
    ExecutionError(#[from] anyhow::Error),
}

struct SkillRuntimeActor<C> {
    runtime: Arc<WasmRuntime>,
    recv: mpsc::Receiver<SkillRuntimeMsg>,
    csi_apis: C,
    // Can be a skill execution or a skill metadata request
    running_requests: FuturesUnordered<Pin<Box<dyn Future<Output = ()> + Send>>>,
}

impl<C> SkillRuntimeActor<C>
where
    C: Csi + Clone + Send + Sync + 'static,
{
    fn new(runtime: WasmRuntime, recv: mpsc::Receiver<SkillRuntimeMsg>, csi_apis: C) -> Self {
        SkillRuntimeActor {
            runtime: Arc::new(runtime),
            recv,
            csi_apis,
            running_requests: FuturesUnordered::new(),
        }
    }

    async fn run(&mut self) {
        loop {
            // While there are messages and running skills, poll both.
            // If there is a message, add it to the queue.
            // If there are skills, make progress on them.
            select! {
                msg = self.recv.recv() => match msg {
                    Some(msg) => {
                        let csi_apis = self.csi_apis.clone();
                        let runtime = self.runtime.clone();
                        self.running_requests.push(Box::pin(async move {
                            msg.act(csi_apis, runtime.as_ref()).await;
                        }));
                    },
                    // Senders are gone, break out of the loop for shutdown.
                    None => break
                },
                // FuturesUnordered will let them run in parallel. It will
                // yield once one of them is completed.
                () = self.running_requests.select_next_some(), if !self.running_requests.is_empty()  => {}
            }
        }
    }
}

pub enum SkillRuntimeMetrics {
    SkillExecutionTotal,
    SkillExecutionDurationSeconds,
}

impl From<SkillRuntimeMetrics> for metrics::KeyName {
    fn from(value: SkillRuntimeMetrics) -> Self {
        Self::from_const_str(match value {
            SkillRuntimeMetrics::SkillExecutionTotal => "kernel_skill_execution_total",
            SkillRuntimeMetrics::SkillExecutionDurationSeconds => {
                "kernel_skill_execution_duration_seconds"
            }
        })
    }
}

pub enum SkillRuntimeMsg {
    Run(SkillRunRequest),
    Metadata(SkillMetadataRequest),
}

impl SkillRuntimeMsg {
    async fn act(self, csi_apis: impl Csi + Send + Sync + 'static, runtime: &WasmRuntime) {
        match self {
            SkillRuntimeMsg::Run(skill_run_request) => {
                skill_run_request.act(csi_apis, runtime).await;
            }
            SkillRuntimeMsg::Metadata(skill_metadata_request) => {
                skill_metadata_request.act(runtime).await;
            }
        }
    }
}
pub struct SkillMetadataRequest {
    pub skill_path: SkillPath,
    pub send: oneshot::Sender<Result<Option<SkillMetadata>, SkillRuntimeError>>,
}

impl SkillMetadataRequest {
    pub async fn act(self, runtime: &WasmRuntime) {
        let (send_rt_err, recv_rt_err) = oneshot::channel();
        let ctx = Box::new(SkillMetadataCtx::new(send_rt_err));
        let response = select! {
            result = runtime.metadata(&self.skill_path, ctx) => result,
            // An error occurred during skill execution.
            Ok(error) = recv_rt_err => Err(SkillRuntimeError::ExecutionError(error))
        };
        drop(self.send.send(response));
    }
}

/// Running a Skill can result in either receiving a single value or a stream of intermediate results.
#[derive(Debug)]
pub enum SkillOutput {
    Stream(SkillStream),
    Value(Value),
}

/// A wrapper struct so we can implement Debug on the Stream
pub struct SkillStream(pub Pin<Box<dyn Stream<Item = Result<Value, serde_json::Error>> + Send>>);

impl fmt::Debug for SkillStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SkillStream")
    }
}

pub struct SkillRunRequest {
    pub skill_path: SkillPath,
    pub input: Value,
    pub send: oneshot::Sender<Result<SkillOutput, SkillRuntimeError>>,
    pub api_token: String,
}

impl SkillRunRequest {
    async fn act(self, csi_apis: impl Csi + Send + Sync + 'static, runtime: &WasmRuntime) {
        let start = Instant::now();
        let SkillRunRequest {
            skill_path,
            input,
            send,
            api_token,
        } = self;

        let span = span!(
            Level::DEBUG,
            "skill_run",
            skill_path = skill_path.to_string(),
        );
        let context = span.context();
        let (send_rt_err, recv_rt_err) = oneshot::channel();
        let (send_rt_write, mut recv_rt_write) = mpsc::channel(1);
        let ctx = Box::new(SkillInvocationCtx::new(
            send_rt_err,
            send_rt_write,
            csi_apis,
            api_token,
            Some(context),
        ));

        // A future that runs the skill or terminates it on an error of csi drivers.
        let skill_fut = async {
            let result = select! {
                result = runtime.run(&skill_path, input, ctx) => result.map(SkillOutput::Value),
                Ok(error) = recv_rt_err => Err(SkillRuntimeError::ExecutionError(error)),
            };
            let latency = start.elapsed().as_secs_f64();
            let labels = [
                ("namespace", Cow::from(skill_path.namespace.to_string())),
                ("name", Cow::from(skill_path.name)),
                (
                    "status",
                    match result {
                        Ok(_) => "ok",
                        Err(SkillRuntimeError::SkillNotConfigured) => "not_found",
                        Err(
                            SkillRuntimeError::StoreError(_) | SkillRuntimeError::ExecutionError(_),
                        ) => "internal_error",
                    }
                    .into(),
                ),
            ];
            metrics::counter!(SkillRuntimeMetrics::SkillExecutionTotal, &labels).increment(1);
            metrics::histogram!(SkillRuntimeMetrics::SkillExecutionDurationSeconds, &labels)
                .record(latency);
            result
        };

        // A future that waits for messages from the running skill and returns a stream over it.
        let recv_fut = async {
            if let Some(first_write) = recv_rt_write.recv().await {
                let stream = Box::pin(stream! {
                    yield serde_json::from_slice(&first_write);
                    while let Some(bytes) = recv_rt_write.recv().await {
                        yield serde_json::from_slice(&bytes);
                    }
                });
                Ok(SkillOutput::Stream(SkillStream(stream)))
            } else {
                pending().await
            }
        };

        // Pinning of the future allows borrowing it in the second select branch.
        pin!(skill_fut);
        select! {
            result = &mut skill_fut => {
                drop(send.send(result));
            }
            result = recv_fut => {
                // In case we receive a stream item, still run the skill future to completion
                drop(send.send(result));
                drop(skill_fut.await);
            }
        }
    }
}

/// Implementation of [`Csi`] provided to skills. It is responsible for forwarding the function
/// calls to csi, to the respective drivers and forwarding runtime errors directly to the actor
/// so the User defined code must not worry about accidental complexity.
pub struct SkillInvocationCtx<C> {
    /// This is used to send any runtime error (as opposed to logic error) back to the actor, so it
    /// can drop the future invoking the skill, and report the error appropriately to user and
    /// operator.
    send_rt_err: Option<oneshot::Sender<anyhow::Error>>,
    /// This is used to enable the skill to send itermediate results. If a value is sent in this
    /// channel, the [`SkillRunRequest::act`] will return a stream of the values sent.
    send_rt_write: mpsc::Sender<Vec<u8>>,
    csi_apis: C,
    // How the user authenticates with us
    api_token: String,
    // For tracing, we wire the skill invocation span context with the CSI spans
    parent_context: Option<Context>,
    // running chat streams
    running_chat_streams: Vec<Option<ChatStream>>,
}

impl<C> SkillInvocationCtx<C> {
    pub fn new(
        send_rt_err: oneshot::Sender<anyhow::Error>,
        send_rt_write: mpsc::Sender<Vec<u8>>,
        csi_apis: C,
        api_token: String,
        parent_context: Option<Context>,
    ) -> Self {
        SkillInvocationCtx {
            send_rt_err: Some(send_rt_err),
            send_rt_write,
            csi_apis,
            api_token,
            parent_context,
            running_chat_streams: Vec::new(),
        }
    }

    /// Never return, we did report the error via the send error channel.
    async fn send_error<T>(&mut self, error: anyhow::Error) -> T {
        self.send_rt_err
            .take()
            .expect("Only one error must be send during skill invocation")
            .send(error)
            .unwrap();
        pending().await
    }
}

/// We know that skill metadata will not invoke any csi functions, but still need to provide an implementation of `CsiForSkills`
/// We do not want to panic, as someone could build a component that uses the csi functions inside the metadata function.
/// Therefore, we always send a runtime error which will lead to skill suspension.
struct SkillMetadataCtx(Option<oneshot::Sender<anyhow::Error>>);

impl SkillMetadataCtx {
    pub fn new(send_rt_err: oneshot::Sender<anyhow::Error>) -> Self {
        Self(Some(send_rt_err))
    }

    async fn send_error<T>(&mut self) -> T {
        self.0
            .take()
            .expect("Only one error must be send during skill invocation")
            .send(anyhow!("CSI usage from metadata is not allowed"))
            .unwrap();
        pending().await
    }
}

#[async_trait]
impl CsiForSkills for SkillMetadataCtx {
    async fn explain(&mut self, _requests: Vec<ExplanationRequest>) -> Vec<Explanation> {
        self.send_error().await
    }

    async fn new_chat_stream(&mut self, _request: ChatRequest) -> u32 {
        self.send_error().await
    }

    async fn next_chat_stream(&mut self, _id: u32) -> Option<StreamMessage> {
        self.send_error().await
    }

    async fn drop_chat_stream(&mut self, _id: u32) {
        self.send_error::<()>().await;
    }

    async fn write(&mut self, _data: Vec<u8>) {
        self.send_error::<()>().await;
    }

    async fn complete(&mut self, _requests: Vec<CompletionRequest>) -> Vec<Completion> {
        self.send_error().await
    }

    async fn chunk(&mut self, _requests: Vec<ChunkRequest>) -> Vec<Vec<Chunk>> {
        self.send_error().await
    }

    async fn select_language(
        &mut self,
        _requests: Vec<SelectLanguageRequest>,
    ) -> Vec<Option<Language>> {
        self.send_error().await
    }

    async fn chat(&mut self, _requests: Vec<ChatRequest>) -> Vec<ChatResponse> {
        self.send_error().await
    }

    async fn search(&mut self, _requests: Vec<SearchRequest>) -> Vec<Vec<SearchResult>> {
        self.send_error().await
    }

    async fn document_metadata(&mut self, _requests: Vec<DocumentPath>) -> Vec<Option<Value>> {
        self.send_error().await
    }

    async fn documents(&mut self, _requests: Vec<DocumentPath>) -> Vec<Document> {
        self.send_error().await
    }
}

#[async_trait]
impl<C> CsiForSkills for SkillInvocationCtx<C>
where
    C: Csi + Send + Sync,
{
    async fn explain(&mut self, requests: Vec<ExplanationRequest>) -> Vec<Explanation> {
        let span = span!(Level::DEBUG, "explain", requests_len = requests.len());
        if let Some(context) = self.parent_context.as_ref() {
            span.set_parent(context.clone());
        }
        match self
            .csi_apis
            .explain(self.api_token.clone(), requests)
            .await
        {
            Ok(value) => value,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn new_chat_stream(&mut self, request: ChatRequest) -> u32 {
        if let Ok(stream) = self
            .csi_apis
            .stream_chat(self.api_token.clone(), request)
            .await
        {
            let id = self.running_chat_streams.len();
            self.running_chat_streams.push(Some(stream));
            u32::try_from(id).unwrap()
        } else {
            self.send_error(anyhow!("Failed to create chat stream"))
                .await
        }
    }

    async fn next_chat_stream(&mut self, id: u32) -> Option<StreamMessage> {
        let stream = self.running_chat_streams.get_mut(id as usize);
        if let Some(Some(stream)) = stream {
            if let Some(Ok(event)) = stream.0.next().await {
                match event {
                    StreamChatEvent::Chunk(ChatChunk::Delta { delta, .. }) => Some(delta),
                    // Currently only forward message deltas to the skill and ignore the other ones
                    StreamChatEvent::Chunk(ChatChunk::Finished { .. })
                    | StreamChatEvent::Usage(_) => None,
                }
            } else {
                // Stop skill execution
                self.send_error::<()>(anyhow!("Failed to receive chat stream event"))
                    .await;
                None
            }
        } else {
            None
        }
    }

    async fn drop_chat_stream(&mut self, id: u32) {
        self.running_chat_streams[id as usize] = None;
    }

    async fn write(&mut self, data: Vec<u8>) {
        // Do not unwrap the send, as we do not know if the receiver is still be alive.
        // An example is a client who closes the connection while the skill is still running.
        // Another option would be to stop Skill execution here using `send_error`, as
        // apparently no one is interested in the result of the skill anymore.
        drop(self.send_rt_write.send(data).await);
    }

    async fn complete(&mut self, requests: Vec<CompletionRequest>) -> Vec<Completion> {
        let span = span!(Level::DEBUG, "complete", requests_len = requests.len());
        if let Some(context) = self.parent_context.as_ref() {
            span.set_parent(context.clone());
        }
        match self
            .csi_apis
            .complete(self.api_token.clone(), requests)
            .await
        {
            Ok(value) => value,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn chat(&mut self, requests: Vec<ChatRequest>) -> Vec<ChatResponse> {
        let span = span!(Level::DEBUG, "chat", requests_len = requests.len());
        if let Some(context) = self.parent_context.as_ref() {
            span.set_parent(context.clone());
        }
        match self.csi_apis.chat(self.api_token.clone(), requests).await {
            Ok(value) => value,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn chunk(&mut self, requests: Vec<ChunkRequest>) -> Vec<Vec<Chunk>> {
        let span = span!(Level::DEBUG, "chunk", requests_len = requests.len());
        if let Some(context) = self.parent_context.as_ref() {
            span.set_parent(context.clone());
        }
        match self.csi_apis.chunk(self.api_token.clone(), requests).await {
            Ok(chunks) => chunks,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn select_language(
        &mut self,
        requests: Vec<SelectLanguageRequest>,
    ) -> Vec<Option<Language>> {
        let span = span!(
            Level::DEBUG,
            "select_language",
            requests_len = requests.len()
        );
        if let Some(context) = self.parent_context.as_ref() {
            span.set_parent(context.clone());
        }
        match self.csi_apis.select_language(requests).await {
            Ok(language) => language,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn search(&mut self, requests: Vec<SearchRequest>) -> Vec<Vec<SearchResult>> {
        let span = span!(Level::DEBUG, "search", requests_len = requests.len());
        if let Some(context) = self.parent_context.as_ref() {
            span.set_parent(context.clone());
        }
        match self.csi_apis.search(self.api_token.clone(), requests).await {
            Ok(value) => value,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn documents(&mut self, requests: Vec<DocumentPath>) -> Vec<Document> {
        let span = span!(Level::DEBUG, "documents", requests_len = requests.len());
        if let Some(context) = self.parent_context.as_ref() {
            span.set_parent(context.clone());
        }
        match self
            .csi_apis
            .documents(self.api_token.clone(), requests)
            .await
        {
            Ok(value) => value,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn document_metadata(&mut self, requests: Vec<DocumentPath>) -> Vec<Option<Value>> {
        let span = span!(
            Level::DEBUG,
            "document_metadata",
            requests_len = requests.len()
        );
        if let Some(context) = self.parent_context.as_ref() {
            span.set_parent(context.clone());
        }
        if let Some(context) = self.parent_context.as_ref() {
            span.set_parent(context.clone());
        }
        match self
            .csi_apis
            .document_metadata(self.api_token.clone(), requests)
            .await
        {
            // We know there will always be exactly one element in the vector
            Ok(value) => value,
            Err(error) => self.send_error(error).await,
        }
    }
}

#[cfg(test)]
pub mod tests {
    use std::time::Duration;

    use anyhow::{anyhow, bail};
    use metrics::Label;
    use metrics_util::debugging::DebugValue;
    use metrics_util::debugging::{DebuggingRecorder, Snapshot};
    use serde_json::json;
    use test_skills::{
        given_csi_from_metadata_skill, given_explain_skill, given_greet_py_v0_3,
        given_greet_skill_v0_2, given_greet_skill_v0_3, given_invalid_output_skill,
        given_write_skill,
    };
    use tokio::try_join;

    use crate::csi::tests::{CsiCompleteStub, CsiGreetingMock};
    use crate::inference::{ChatStream, Explanation, ExplanationRequest, TextScore};
    use crate::namespace_watcher::Namespace;
    use crate::skills::SkillMetadata;
    use crate::{
        chunking::ChunkParams,
        csi::tests::{DummyCsi, StubCsi},
        inference::{
            ChatRequest, ChatResponse, CompletionRequest, Inference, tests::AssertConcurrentClient,
        },
        search::DocumentPath,
        skill_loader::{RegistryConfig, SkillLoader},
        skill_store::{SkillStore, SkillStoreMessage},
        skills::Skill,
    };

    use super::*;

    #[tokio::test]
    async fn skill_runtime_returns_stream() {
        // Given a skill that writes to the host
        let test_skill = given_write_skill();
        let skill_path = SkillPath::local("write");
        let engine = Arc::new(Engine::new(false).unwrap());
        let store = SkillStoreStub::new(engine.clone(), test_skill.bytes(), skill_path.clone());

        let mut csi = StubCsi::empty();
        let stream = |_| {
            let items = vec![
                StreamMessage {
                    role: Some("assistant".to_string()),
                    content: String::new(),
                },
                StreamMessage {
                    role: None,
                    content: "Keeps the doctor away".to_string(),
                },
            ];
            Ok(ChatStream(Box::pin(stream! {
                for item in items {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    yield Ok(StreamChatEvent::Chunk(ChatChunk::Delta {
                        delta: item,
                    }));
                }
            })))
        };
        csi.set_chat_streaming(stream);

        // When running the skill
        let runtime = SkillRuntime::new(engine, csi, store.api());
        let result = runtime
            .api()
            .skill_run(
                skill_path,
                json!({"content": "An apple a day", "role": "user"}),
                "dummy token".to_owned(),
            )
            .await;

        // Then the result is a stream
        match result {
            Ok(SkillOutput::Stream(stream)) => {
                // collect the stream
                let mut stream = stream.0;
                let mut items = Vec::new();
                while let Some(item) = stream.next().await {
                    items.push(item);
                }
                assert_eq!(items.len(), 2);
                assert_eq!(
                    items[0].as_ref().unwrap(),
                    &json!({"choices": [{"delta": {"role": "assistant", "content": ""}}]})
                );
                assert_eq!(
                    items[1].as_ref().unwrap(),
                    &json!({"choices": [{"delta": {"role": null, "content": "Keeps the doctor away"}}]}
                    )
                );
            }
            _ => panic!("Expected a stream"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn skill_runtime_with_receiver_gone() {
        // Given a skill request for a streaming skill
        let test_skill = given_write_skill();
        let skill_path = SkillPath::local("write");
        let engine = Arc::new(Engine::new(false).unwrap());
        let store = SkillStoreStub::new(engine.clone(), test_skill.bytes(), skill_path.clone());

        // And a csi with a bit of delay between chat items
        let mut csi = StubCsi::empty();
        let stream = |_| {
            let items = vec![
                StreamMessage {
                    role: Some("assistant".to_string()),
                    content: String::new(),
                },
                StreamMessage {
                    role: None,
                    content: "Keeps the doctor away".to_string(),
                },
            ];
            Ok(ChatStream(Box::pin(stream! {
                for item in items {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    yield Ok(StreamChatEvent::Chunk(ChatChunk::Delta {
                        delta: item,
                    }));
                }
            })))
        };
        csi.set_chat_streaming(stream);

        let runtime = SkillRuntime::new(engine, csi, store.api());
        let result = runtime
            .api()
            .skill_run(
                skill_path,
                json!({"content": "An apple a day", "role": "user"}),
                "dummy token".to_owned(),
            )
            .await;

        // When dropping the stream between the first and second item
        match result {
            Ok(SkillOutput::Stream(stream)) => {
                let mut stream = stream.0;
                let item = stream.next().await.unwrap().unwrap();
                assert_eq!(
                    item,
                    json!({"choices": [{"delta": {"role": "assistant", "content": ""}}]})
                );
                drop(stream);
            }
            _ => panic!("Expected a stream"),
        }
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Then the skill runtime should not panic
        runtime.wait_for_shutdown().await;
    }

    #[tokio::test]
    async fn csi_usage_from_metadata_leads_to_suspension() {
        // Given a skill runtime that always returns a skill that uses the csi from the metadata function
        let test_skills = given_csi_from_metadata_skill();
        let skill_path = SkillPath::local("invoke_csi_from_metadata");
        let engine = Arc::new(Engine::new(false).unwrap());
        let store = SkillStoreStub::new(engine.clone(), test_skills.bytes(), skill_path.clone());
        let runtime = SkillRuntime::new(engine, SaboteurCsi, store.api());

        // When metadata for a skill is requested
        let metadata = runtime.api().skill_metadata(skill_path).await;
        runtime.wait_for_shutdown().await;

        // Then the metadata is None
        assert_eq!(
            metadata.unwrap_err().to_string(),
            "CSI usage from metadata is not allowed"
        );
    }

    #[tokio::test]
    async fn skill_metadata_v0_2_is_none() {
        // Given a skill runtime that always returns a v0.2 skill
        let test_skill = given_greet_skill_v0_2();
        let skill_path = SkillPath::local("greet");
        let engine = Arc::new(Engine::new(false).unwrap());
        let store = SkillStoreStub::new(engine.clone(), test_skill.bytes(), skill_path.clone());
        let runtime = SkillRuntime::new(engine, SaboteurCsi, store.api());

        // When metadata for a skill is requested
        let metadata = runtime.api().skill_metadata(skill_path).await.unwrap();
        runtime.wait_for_shutdown().await;

        // Then the metadata is None
        assert!(metadata.is_none());
    }

    #[tokio::test]
    async fn skill_metadata_v0_3() {
        // Given a skill runtime api that always returns a v0.3 skill
        let test_skill = given_greet_skill_v0_3();
        let skill_path = SkillPath::local("greet");
        let engine = Arc::new(Engine::new(false).unwrap());
        let store = SkillStoreStub::new(engine.clone(), test_skill.bytes(), skill_path.clone());
        let runtime = SkillRuntime::new(engine, SaboteurCsi, store.api());

        // When metadata for a skill is requested
        let metadata = runtime.api().skill_metadata(skill_path).await.unwrap();
        runtime.wait_for_shutdown().await;

        // Then the metadata is returned
        match metadata {
            Some(SkillMetadata::V1(metadata)) => {
                assert_eq!(metadata.description.unwrap(), "A friendly greeting skill");
                assert_eq!(
                    metadata.input_schema,
                    json!({"type": "string", "description": "The name of the person to greet"})
                        .try_into()
                        .unwrap()
                );
                assert_eq!(
                    metadata.output_schema,
                    json!({"type": "string", "description": "A friendly greeting message"})
                        .try_into()
                        .unwrap()
                );
            }
            _ => panic!("Expected SkillMetadata::V1"),
        }
    }

    #[tokio::test]
    async fn skill_metadata_invalid_output() {
        // Given a skill runtime that always returns an invalid output skill
        let test_skill = given_invalid_output_skill();
        let skill_path = SkillPath::local("invalid_output_skill");
        let engine = Arc::new(Engine::new(false).unwrap());
        let store = SkillStoreStub::new(engine.clone(), test_skill.bytes(), skill_path.clone());
        let runtime = SkillRuntime::new(engine, SaboteurCsi, store.api());

        // When metadata for a skill is requested
        let metadata = runtime.api().skill_metadata(skill_path).await;
        runtime.wait_for_shutdown().await;

        // Then the metadata gives an error
        assert!(metadata.is_err());
    }

    #[tokio::test]
    async fn explain_skill_component() {
        let test_skill = given_explain_skill();
        let skill_path = SkillPath::local("explain");
        let engine = Arc::new(Engine::new(false).unwrap());
        let skill_store =
            SkillStoreStub::new(engine.clone(), test_skill.bytes(), skill_path.clone());
        let (send, _) = oneshot::channel();
        let (send_rt_write, _) = mpsc::channel(1);
        let csi = StubCsi::with_explain(|_| {
            Explanation::new(vec![TextScore {
                score: 0.0,
                start: 0,
                length: 2,
            }])
        });
        let skill_ctx = Box::new(SkillInvocationCtx::new(
            send,
            send_rt_write,
            csi,
            "dummy token".to_owned(),
            None,
        ));

        let runtime = WasmRuntime::new(engine, skill_store.api());
        let resp = runtime
            .run(
                &skill_path,
                json!({"prompt": "An apple a day", "target": " keeps the doctor away"}),
                skill_ctx,
            )
            .await;

        drop(runtime);
        skill_store.wait_for_shutdown().await;

        assert_eq!(resp.unwrap(), json!([{"start": 0, "length": 2}]));
    }

    #[tokio::test]
    async fn greet_skill_component() {
        let test_skill = given_greet_skill_v0_2();
        let skill_path = SkillPath::local("greet");
        let engine = Arc::new(Engine::new(false).unwrap());
        let skill_store =
            SkillStoreStub::new(engine.clone(), test_skill.bytes(), skill_path.clone());

        let runtime = WasmRuntime::new(engine, skill_store.api());
        let skill_ctx = Box::new(CsiCompleteStub::new(|_| Completion::from_text("Hello")));
        let resp = runtime.run(&skill_path, json!("name"), skill_ctx).await;

        drop(runtime);
        skill_store.wait_for_shutdown().await;

        assert_eq!(resp.unwrap(), "Hello");
    }

    #[tokio::test]
    async fn errors_for_non_existing_skill() {
        let engine = Arc::new(Engine::new(false).unwrap());
        let namespace = Namespace::new("local").unwrap();
        let skill_loader = SkillLoader::with_file_registry(engine.clone(), namespace).api();
        let skill_store = SkillStore::new(skill_loader, Duration::from_secs(10));
        let runtime = WasmRuntime::new(engine, skill_store.api());
        let skill_ctx = Box::new(CsiCompleteStub::new(|_| Completion::from_text("")));
        let resp = runtime
            .run(&SkillPath::dummy(), json!("name"), skill_ctx)
            .await;

        drop(runtime);
        skill_store.wait_for_shutdown().await;

        assert!(resp.is_err());
    }

    #[tokio::test]
    async fn rust_greeting_skill() {
        let test_skill = given_greet_skill_v0_2();
        let skill_path = SkillPath::local("greet_skill_v0_2");
        let engine = Arc::new(Engine::new(false).unwrap());
        let skill_store =
            SkillStoreStub::new(engine.clone(), test_skill.bytes(), skill_path.clone());
        let skill_ctx = Box::new(CsiGreetingMock);

        let runtime = WasmRuntime::new(engine, skill_store.api());
        let actual = runtime
            .run(&skill_path, json!("Homer"), skill_ctx)
            .await
            .unwrap();

        drop(runtime);
        skill_store.wait_for_shutdown().await;

        assert_eq!(actual, "Hello Homer");
    }

    #[tokio::test]
    async fn python_greeting_skill() {
        let test_skill = given_greet_py_v0_3();
        let skill_ctx = Box::new(CsiGreetingMock);
        let skill_path = SkillPath::local("greet");
        let engine = Arc::new(Engine::new(false).unwrap());
        let skill_store =
            SkillStoreStub::new(engine.clone(), test_skill.bytes(), skill_path.clone());

        let runtime = WasmRuntime::new(engine, skill_store.api());

        let actual = runtime
            .run(&skill_path, json!("Homer"), skill_ctx)
            .await
            .unwrap();

        drop(runtime);
        skill_store.wait_for_shutdown().await;

        assert_eq!(actual, "Hello Homer");
    }

    #[tokio::test]
    async fn chunk() {
        // Given a skill invocation context with a stub tokenizer provider
        let (send, _) = oneshot::channel();
        let (send_rt_write, _) = mpsc::channel(1);
        let mut csi = StubCsi::empty();
        csi.set_chunking(|r| {
            Ok(r.into_iter()
                .map(|_| {
                    vec![Chunk {
                        text: "my_chunk".to_owned(),
                        byte_offset: 0,
                        character_offset: None,
                    }]
                })
                .collect())
        });

        let mut invocation_ctx =
            SkillInvocationCtx::new(send, send_rt_write, csi, "dummy token".to_owned(), None);

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
        let chunks = invocation_ctx.chunk(vec![request]).await;

        // Then a single chunk is returned
        assert_eq!(
            chunks[0],
            vec![Chunk {
                text: "my_chunk".to_owned(),
                byte_offset: 0,
                character_offset: None,
            }]
        );
    }

    #[tokio::test]
    async fn receive_error_if_chunk_failed() {
        // Given a skill invocation context with a saboteur tokenizer provider
        let (send, recv) = oneshot::channel();
        let (send_rt_write, _) = mpsc::channel(1);
        let mut csi = StubCsi::empty();
        csi.set_chunking(|_| Err(anyhow!("Failed to load tokenizer")));
        let mut invocation_ctx =
            SkillInvocationCtx::new(send, send_rt_write, csi, "dummy token".to_owned(), None);

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
        let error = select! {
            error = recv => error.unwrap(),
            _ = invocation_ctx.chunk(vec![request])  => unreachable!(),
        };

        // Then receive the error from saboteur tokenizer provider
        assert_eq!(error.to_string(), "Failed to load tokenizer");
    }

    #[tokio::test]
    async fn dedicated_error_for_skill_not_found() {
        // Given a skill executer with no skills
        let engine = Arc::new(Engine::new(false).unwrap());
        let registry_config = RegistryConfig::empty();
        let skill_loader = SkillLoader::from_config(engine.clone(), registry_config).api();

        let skill_store = SkillStore::new(skill_loader, Duration::from_secs(10));
        let csi_apis = DummyCsi;
        let executer = SkillRuntime::new(engine, csi_apis, skill_store.api());
        let api = executer.api();

        // When a skill is requested, but it is not listed in the namespace
        let result = api
            .skill_run(
                SkillPath::local("my_skill"),
                json!("Any input"),
                "Dummy api token".to_owned(),
            )
            .await;

        drop(api);
        executer.wait_for_shutdown().await;
        skill_store.wait_for_shutdown().await;

        // Then result indicates that the skill is missing
        assert!(matches!(result, Err(SkillRuntimeError::SkillNotConfigured)));
    }

    #[tokio::test]
    async fn skill_runtime_forwards_csi_errors() {
        // Given csi which emits errors for completion request
        let test_skill = given_greet_skill_v0_3();
        let engine = Arc::new(Engine::new(false).unwrap());
        let store = SkillStoreStub::new(
            engine.clone(),
            test_skill.bytes(),
            SkillPath::local("greet"),
        );
        let runtime = SkillRuntime::new(engine, SaboteurCsi, store.api());

        // When trying to generate a greeting for Homer using the greet skill
        let result = runtime
            .api()
            .skill_run(
                SkillPath::local("greet"),
                json!("Homer"),
                "TOKEN_NOT_REQUIRED".to_owned(),
            )
            .await;

        runtime.wait_for_shutdown().await;
        store.wait_for_shutdown().await;

        // Then
        assert_eq!(result.unwrap_err().to_string(), "Test error");
    }

    #[tokio::test]
    async fn greeting_skill() {
        // Given
        let test_skill = given_greet_skill_v0_3();
        let csi = StubCsi::with_completion_from_text("Hello");
        let engine = Arc::new(Engine::new(false).unwrap());
        let store = SkillStoreStub::new(
            engine.clone(),
            test_skill.bytes(),
            SkillPath::local("greet"),
        );

        // When
        let runtime = SkillRuntime::new(engine, csi, store.api());
        let result = runtime
            .api()
            .skill_run(
                SkillPath::local("greet"),
                json!(""),
                "TOKEN_NOT_REQUIRED".to_owned(),
            )
            .await;
        runtime.wait_for_shutdown().await;
        store.wait_for_shutdown().await;

        // Then
        match result {
            Ok(SkillOutput::Value(result)) => assert_eq!(result, "Hello"),
            _ => panic!("Expected a result"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn concurrent_skill_execution() {
        // Given
        let test_skill = given_greet_skill_v0_3();
        let engine = Arc::new(Engine::new(false).unwrap());
        let client = AssertConcurrentClient::new(2);
        let inference = Inference::with_client(client);
        let csi = StubCsi::with_completion_from_text("Hello, Homer!");
        let store = SkillStoreStub::new(
            engine.clone(),
            test_skill.bytes(),
            SkillPath::local("greet"),
        );
        let runtime = SkillRuntime::new(engine, csi, store.api());
        let api = runtime.api();

        // When executing two tasks in parallel
        let skill_path = SkillPath::local("greet");
        let input = json!("Homer");
        let token = "TOKEN_NOT_REQUIRED";

        let result = try_join!(
            api.skill_run(skill_path.clone(), input.clone(), token.to_owned()),
            api.skill_run(skill_path, input, token.to_owned()),
        );

        drop(api);
        runtime.wait_for_shutdown().await;
        inference.wait_for_shutdown().await;
        store.wait_for_shutdown().await;

        // Then they both have completed with the same values
        // Then they both have completed with the same values
        match result.unwrap() {
            (SkillOutput::Value(result1), SkillOutput::Value(result2)) => {
                assert_eq!(result1, result2);
            }
            _ => panic!("Expected a result"),
        }
    }

    #[test]
    fn skill_runtime_metrics_emitted() {
        let test_skill = given_greet_skill_v0_3();
        let engine = Arc::new(Engine::new(false).unwrap());
        let csi = StubCsi::with_completion_from_text("Hello");
        let (send, _) = oneshot::channel();
        let skill_path = SkillPath::local("greet");
        let msg = SkillRunRequest {
            skill_path: skill_path.clone(),
            input: json!("Hello"),
            send,
            api_token: "dummy".to_owned(),
        };
        // Metrics requires sync, so all of the async parts are moved into this closure.
        let snapshot = metrics_snapshot(async || {
            let store = SkillStoreStub::new(
                engine.clone(),
                test_skill.bytes(),
                SkillPath::local("greet"),
            );
            let runtime = WasmRuntime::new(engine, store.api());
            msg.act(csi, &runtime).await;
            drop(runtime);
            store.wait_for_shutdown().await;
        });

        let metrics = snapshot.into_vec();
        let expected_labels = [
            &Label::new("namespace", skill_path.namespace.to_string()),
            &Label::new("name", skill_path.name),
            &Label::new("status", "ok"),
        ];
        assert!(metrics.iter().any(|(key, _, _, value)| {
            let key = key.key();
            let labels = key.labels().collect::<Vec<_>>();
            key.name() == "kernel_skill_execution_total"
                && labels == expected_labels
                && value == &DebugValue::Counter(1)
        }));
        assert!(metrics.iter().any(|(key, _, _, _)| {
            let key = key.key();
            let labels = key.labels().collect::<Vec<_>>();
            key.name() == "kernel_skill_execution_duration_seconds" && labels == expected_labels
        }));
    }

    fn metrics_snapshot<F: Future<Output = ()>>(f: impl FnOnce() -> F) -> Snapshot {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        metrics::with_local_recorder(&recorder, || runtime.block_on(f()));
        snapshotter.snapshot()
    }

    #[derive(Clone)]
    struct SaboteurCsi;

    #[async_trait]
    impl Csi for SaboteurCsi {
        async fn explain(
            &self,
            _auth: String,
            _requests: Vec<ExplanationRequest>,
        ) -> anyhow::Result<Vec<Explanation>> {
            bail!("Test error")
        }
        async fn complete(
            &self,
            _auth: String,
            _requests: Vec<CompletionRequest>,
        ) -> anyhow::Result<Vec<Completion>> {
            bail!("Test error")
        }

        async fn chunk(
            &self,
            _auth: String,
            _requests: Vec<ChunkRequest>,
        ) -> anyhow::Result<Vec<Vec<Chunk>>> {
            bail!("Test error")
        }

        async fn chat(
            &self,
            _auth: String,
            _requests: Vec<ChatRequest>,
        ) -> anyhow::Result<Vec<ChatResponse>> {
            bail!("Test error")
        }

        async fn stream_chat(
            &self,
            _auth: String,
            _request: ChatRequest,
        ) -> anyhow::Result<ChatStream> {
            bail!("Test error")
        }

        async fn search(
            &self,
            _auth: String,
            _requests: Vec<SearchRequest>,
        ) -> anyhow::Result<Vec<Vec<SearchResult>>> {
            bail!("Test error")
        }

        async fn documents(
            &self,
            _auth: String,
            _requests: Vec<DocumentPath>,
        ) -> anyhow::Result<Vec<Document>> {
            bail!("Test error")
        }

        async fn document_metadata(
            &self,
            _auth: String,
            _document_paths: Vec<DocumentPath>,
        ) -> anyhow::Result<Vec<Option<Value>>> {
            bail!("Test error")
        }
    }
    pub struct SkillStoreStub {
        send: mpsc::Sender<SkillStoreMessage>,
        join_handle: JoinHandle<()>,
    }

    impl SkillStoreStub {
        pub fn new(engine: Arc<Engine>, bytes: Vec<u8>, path: SkillPath) -> Self {
            let skill = Skill::new(&engine, bytes).unwrap();
            let skill = Arc::new(skill);

            let (send, mut recv) = mpsc::channel::<SkillStoreMessage>(1);
            let join_handle = tokio::spawn(async move {
                while let Some(msg) = recv.recv().await {
                    match msg {
                        SkillStoreMessage::Fetch { skill_path, send } => {
                            let skill = if skill_path == path {
                                Some(skill.clone())
                            } else {
                                None
                            };
                            drop(send.send(Ok(skill)));
                        }
                        _ => panic!("Operation unimplemented in test stub"),
                    }
                }
            });

            Self { send, join_handle }
        }

        pub async fn wait_for_shutdown(self) {
            drop(self.send);
            self.join_handle.await.unwrap();
        }

        pub fn api(&self) -> SkillStoreApi {
            SkillStoreApi::new(self.send.clone())
        }
    }
}
