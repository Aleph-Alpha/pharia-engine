use std::{
    borrow::Cow,
    collections::HashMap,
    future::{Future, pending},
    pin::Pin,
    sync::Arc,
    time::Instant,
};

use anyhow::anyhow;
use async_trait::async_trait;
use futures::{StreamExt, stream::FuturesUnordered};
use serde_json::Value;
use thiserror::Error;
use tokio::{
    select,
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use crate::{
    chunking::{Chunk, ChunkRequest},
    csi::{ChatStreamId, CompletionStreamId, Csi, CsiForSkills},
    inference::{
        self, ChatRequest, ChatResponse, Completion, CompletionEvent, CompletionRequest,
        Explanation, ExplanationRequest,
    },
    language_selection::{Language, SelectLanguageRequest},
    namespace_watcher::Namespace,
    search::{Document, DocumentPath, SearchRequest, SearchResult},
    skill_store::{SkillStoreApi, SkillStoreError},
    skills::{AnySkillManifest, Engine, SkillError, SkillPath},
};

pub struct WasmRuntime<S> {
    /// Used to execute skills. We will share the engine with multiple running skills, and skill
    /// provider to convert bytes into executable skills.
    engine: Arc<Engine>,
    skill_store_api: S,
}

impl<S> WasmRuntime<S>
where
    S: SkillStoreApi,
{
    pub fn new(engine: Arc<Engine>, skill_store_api: S) -> Self {
        Self {
            engine,
            skill_store_api,
        }
    }

    pub async fn run_stream(
        &self,
        skill_path: &SkillPath,
        input: Value,
        ctx: Box<dyn CsiForSkills + Send>,
        sender: mpsc::Sender<StreamEvent>,
    ) -> Result<(), SkillExecutionError> {
        let skill = self
            .skill_store_api
            .fetch(skill_path.to_owned())
            .await?
            .ok_or(SkillExecutionError::SkillNotConfigured)?;
        skill
            .run_as_generator(&self.engine, ctx, input, sender)
            .await?;
        Ok(())
    }

    pub async fn run_function(
        &self,
        skill_path: &SkillPath,
        input: Value,
        ctx: Box<dyn CsiForSkills + Send>,
    ) -> Result<Value, SkillExecutionError> {
        let skill = self.skill_store_api.fetch(skill_path.to_owned()).await?;
        // Unwrap Skill, raise error if it is not existing
        let skill = skill.ok_or(SkillExecutionError::SkillNotConfigured)?;
        let output = skill.run_as_function(&self.engine, ctx, input).await?;
        Ok(output)
    }

    pub async fn metadata(
        &self,
        skill_path: &SkillPath,
        ctx: Box<dyn CsiForSkills + Send>,
    ) -> Result<AnySkillManifest, SkillExecutionError> {
        let skill = self.skill_store_api.fetch(skill_path.to_owned()).await?;
        // Unwrap Skill, raise error if it is not existing
        let skill = skill.ok_or(SkillExecutionError::SkillNotConfigured)?;
        let manifest = skill.manifest(self.engine.as_ref(), ctx).await?;
        Ok(manifest)
    }
}

impl From<SkillStoreError> for SkillExecutionError {
    fn from(source: SkillStoreError) -> Self {
        match source {
            SkillStoreError::SkillLoaderError(skill_loader_error) => {
                SkillExecutionError::RuntimeError(anyhow!(
                    "Error loading skill: {}",
                    skill_loader_error
                ))
            }
            SkillStoreError::InvalidNamespaceError(namespace, original_syntax_error) => {
                SkillExecutionError::MisconfiguredNamespace {
                    namespace,
                    original_syntax_error,
                }
            }
        }
    }
}

impl From<SkillError> for SkillExecutionError {
    fn from(source: SkillError) -> Self {
        match source {
            SkillError::Any(error) => Self::UserCode(error.to_string()),
            SkillError::InvalidInput(error) => Self::InvalidInput(error),
            SkillError::UserCode(error) => Self::UserCode(error),
            SkillError::InvalidOutput(error) => Self::InvalidOutput(error),
            SkillError::RuntimeError(error) => SkillExecutionError::RuntimeError(error),
            SkillError::IsFunction => SkillExecutionError::IsFunction,
            SkillError::IsGenerator => SkillExecutionError::IsGenerator,
        }
    }
}

/// Starts and stops the execution of skills as it owns the skill runtime actor.
pub struct SkillRuntime {
    send: mpsc::Sender<SkillRuntimeMsg>,
    handle: JoinHandle<()>,
}

impl SkillRuntime {
    /// Create a new skill runtime with the default web assembly runtime
    pub fn new<C>(
        engine: Arc<Engine>,
        csi_apis: C,
        skill_store_api: impl SkillStoreApi + Send + Sync + 'static,
    ) -> Self
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
    pub fn api(&self) -> mpsc::Sender<SkillRuntimeMsg> {
        self.send.clone()
    }

    pub async fn wait_for_shutdown(self) {
        drop(self.send);
        self.handle.await.unwrap();
    }
}

/// The skill runtime API is used to interact with the skill runtime actor.
///
/// Using a trait rather than an mpsc allows for easier and more ergonomic testing, since the
/// implementation of the test double is not required to be an actor.
#[async_trait]
pub trait SkillRuntimeApi {
    async fn run_function(
        &self,
        skill_path: SkillPath,
        input: Value,
        api_token: String,
    ) -> Result<Value, SkillExecutionError>;

    async fn run_stream(
        &self,
        skill_path: SkillPath,
        input: Value,
        api_token: String,
    ) -> mpsc::Receiver<StreamEvent>;

    async fn skill_metadata(
        &self,
        skill_path: SkillPath,
    ) -> Result<AnySkillManifest, SkillExecutionError>;
}

#[async_trait]
impl SkillRuntimeApi for mpsc::Sender<SkillRuntimeMsg> {
    async fn run_function(
        &self,
        skill_path: SkillPath,
        input: Value,
        api_token: String,
    ) -> Result<Value, SkillExecutionError> {
        let (send, recv) = oneshot::channel();
        let msg = SkillRuntimeMsg::Function(RunFunctionMsg {
            skill_path,
            input,
            send,
            api_token,
        });
        self.send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }

    async fn run_stream(
        &self,
        skill_path: SkillPath,
        input: Value,
        api_token: String,
    ) -> mpsc::Receiver<StreamEvent> {
        let (send, recv) = mpsc::channel::<StreamEvent>(1);

        let msg = RunStreamMsg {
            skill_path,
            input,
            send,
            api_token,
        };

        self.send(SkillRuntimeMsg::Stream(msg))
            .await
            .expect("all api handlers must be shutdown before actors");
        recv
    }

    async fn skill_metadata(
        &self,
        skill_path: SkillPath,
    ) -> Result<AnySkillManifest, SkillExecutionError> {
        let (send, recv) = oneshot::channel();
        let msg = SkillRuntimeMsg::Metadata(MetadataMsg { skill_path, send });
        self.send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }
}

/// Errors which may prevent a skill from executing to completion successfully.
#[derive(Debug, Error)]
pub enum SkillExecutionError {
    #[error(
        "The metadata function of the invoked skill is bugged. It is forbidden to invoke any CSI \
        functions from the metadata function, yet the skill does precisely this."
    )]
    CsiUseFromMetadata,
    /// A skill name is not mentioned in the namespace and therfore it is not served. This is a
    /// logic error. Yet it does not originate in the skill code itself. It could be an error in the
    /// request by the user, or a missing configuration at the side of the skill developer.
    #[error(
        "Sorry, We could not find the skill you requested in its namespace. This can have three \
        causes:\n\n\
        1. You send the wrong skill name.\n\
        2. You send the wrong namespace.\n\
        3. The skill is not configured in the namespace you requested. You may want to check the \
        namespace configuration."
    )]
    SkillNotConfigured,
    /// Skill Logic errors are logic errors which are reported by the skill code itself. These may
    /// be due to bugs in the skill code, or invalid user input, we will not be able to tell. For
    /// the operater these are both user errors. The skill user and developer are often the same
    /// person so in either case we do well to report it in our answer.
    #[error(
        "The skill you called responded with an error. Maybe you should check your input, if it \
        seems to be correct you may want to contact the skill developer. Error reported by Skill:\n\
        \n{0}"
    )]
    UserCode(String),
    /// Skills are still responsible for parsing the input bytes. Our SDK emits a specific error if
    /// the input is not interpretable.
    #[error("The skill had trouble interpreting the input:\n\n{0}")]
    InvalidInput(String),
    #[error(
        "The skill returned an output the Kernel could not interpret. This hints to a bug in the \
        skill. Please contact its developer. Caused by: \n\n{0}"
    )]
    InvalidOutput(String),
    /// A runtime error is caused by the runtime, i.e. the dependencies and resources we need to be
    /// available in order execute skills. For us this are mostly the drivers behind the CSI. This
    /// does not mean that all CSI errors are runtime errors, because the Inference may e.g. be
    /// available, but the prompt exceeds the maximum length, etc. Yet any error caused by spurious
    /// network errors, inference being to busy, etc. will fall in this category. For runtime errors
    /// it is most important to report them to the operator, so they can take action.
    ///
    /// For our other **users** we should not forward them transparently, but either just tell them
    /// something went wrong, or give appropriate context. E.g. whether it was inference or document
    /// index which caused the error.
    #[error(
        "The skill could not be executed to completion, something in our runtime is currently \n\
        unavailable or misconfigured. You should try again later, if the situation persists you \n\
        may want to contact the operaters. Original error:\n\n{0}"
    )]
    RuntimeError(#[source] anyhow::Error),
    /// This happens if a configuration for an individual namespace is broken. For the user calling
    /// the route to execute a skill, we treat this as a runtime, but make sure he gets all the
    /// context, because it very likely might be the skill developer who misconfigured the
    /// namespace. For the operater team, operating all of Pharia Kernel we treat this as a logic
    /// error, because there is nothing wrong about the kernel installion or inference, or network
    /// or other stuff, which they would be able to fix.
    #[error(
        "The skill could not be executed to completion, the namespace '{namespace}' is \
        misconfigured. If you are the developer who configured the skill, you should probably fix \
        this error. If you are not, there is nothing you can do, until the developer who maintains \
        the list of skills to be served, fixes this. Original Syntax error:\n\n\
        {original_syntax_error}"
    )]
    MisconfiguredNamespace {
        namespace: Namespace,
        original_syntax_error: String,
    },
    #[error(
        "The skill is designed to be executed as a function. Please invoke it via the /run endpoint."
    )]
    IsFunction,
    #[error("The skill is designed to stream output. Please invoke it via the /stream endpoint.")]
    IsGenerator,
}

struct SkillRuntimeActor<C, S> {
    runtime: Arc<WasmRuntime<S>>,
    recv: mpsc::Receiver<SkillRuntimeMsg>,
    csi_apis: C,
    // Can be a skill execution or a skill metadata request
    running_requests: FuturesUnordered<Pin<Box<dyn Future<Output = ()> + Send>>>,
}

impl<C, S> SkillRuntimeActor<C, S>
where
    C: Csi + Clone + Send + Sync + 'static,
    S: SkillStoreApi + Send + Sync + 'static,
{
    fn new(runtime: WasmRuntime<S>, recv: mpsc::Receiver<SkillRuntimeMsg>, csi_apis: C) -> Self {
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

#[derive(Debug)]
pub enum SkillRuntimeMsg {
    Stream(RunStreamMsg),
    Function(RunFunctionMsg),
    Metadata(MetadataMsg),
}

impl SkillRuntimeMsg {
    async fn act(
        self,
        csi_apis: impl Csi + Send + Sync + 'static,
        runtime: &WasmRuntime<impl SkillStoreApi>,
    ) {
        match self {
            SkillRuntimeMsg::Stream(msg) => {
                msg.act(csi_apis, runtime).await;
            }
            SkillRuntimeMsg::Function(msg) => {
                msg.act(csi_apis, runtime).await;
            }
            SkillRuntimeMsg::Metadata(msg) => {
                msg.act(runtime).await;
            }
        }
    }
}

#[derive(Debug)]
pub struct MetadataMsg {
    pub skill_path: SkillPath,
    pub send: oneshot::Sender<Result<AnySkillManifest, SkillExecutionError>>,
}

impl MetadataMsg {
    pub async fn act(self, runtime: &WasmRuntime<impl SkillStoreApi>) {
        let (send_rt_err, recv_rt_err) = oneshot::channel();
        let ctx = Box::new(SkillMetadataCtx::new(send_rt_err));
        let response = select! {
            result = runtime.metadata(&self.skill_path, ctx) => result,
            // An error occurred during skill execution.
            Ok(_error) = recv_rt_err => Err(SkillExecutionError::CsiUseFromMetadata)
        };
        drop(self.send.send(response));
    }
}

#[derive(Debug)]
pub struct RunStreamMsg {
    pub skill_path: SkillPath,
    pub input: Value,
    pub send: mpsc::Sender<StreamEvent>,
    pub api_token: String,
}

impl RunStreamMsg {
    async fn act(
        self,
        csi_apis: impl Csi + Send + Sync + 'static,
        runtime: &WasmRuntime<impl SkillStoreApi>,
    ) {
        let start = Instant::now();

        let RunStreamMsg {
            skill_path,
            input,
            send,
            api_token,
        } = self;

        let (send_rt_err, recv_rt_err) = oneshot::channel();
        let ctx = Box::new(SkillInvocationCtx::new(send_rt_err, csi_apis, api_token));
        let response = select! {
            result = runtime.run_stream(&skill_path, input, ctx, send.clone()) => result,
            // An error occurred during skill execution.
            Ok(error) = recv_rt_err => Err(SkillExecutionError::RuntimeError(error))
        };

        let label = status_label(response.as_ref().map(|&()| ()));

        // We do not bubble up the error, instead we insert it into the event stream, as the last
        // event.
        if let Err(error) = response {
            send.send(StreamEvent::Error(error.to_string()))
                .await
                .unwrap();
        };

        record_skill_metrics(start, skill_path, label);
    }
}

fn record_skill_metrics(start: Instant, skill_path: SkillPath, status: String) {
    let latency = start.elapsed().as_secs_f64();
    let labels = [
        ("namespace", Cow::from(skill_path.namespace.to_string())),
        ("name", Cow::from(skill_path.name)),
        ("status", status.into()),
    ];
    metrics::counter!(SkillRuntimeMetrics::SkillExecutionTotal, &labels).increment(1);
    metrics::histogram!(SkillRuntimeMetrics::SkillExecutionDurationSeconds, &labels)
        .record(latency);
}

fn status_label(result: Result<(), &SkillExecutionError>) -> String {
    match result {
        Ok(()) => "ok",
        Err(
            SkillExecutionError::UserCode(_)
            | SkillExecutionError::CsiUseFromMetadata
            | SkillExecutionError::SkillNotConfigured
            | SkillExecutionError::InvalidInput(_)
            | SkillExecutionError::InvalidOutput(_)
            | SkillExecutionError::MisconfiguredNamespace { .. }
            | SkillExecutionError::IsFunction
            | SkillExecutionError::IsGenerator,
        ) => "logic_error",
        Err(SkillExecutionError::RuntimeError(_)) => "runtime_error",
    }
    .to_owned()
}

/// An event emitted by a streaming skill
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamEvent {
    /// Send at the beginning of each message, currently carries no information. May be used in the
    /// future to communicate the role. Can also be useful to the UI to communicate that its about
    /// time to start rendering that speech bubble.
    MessageBegin,
    /// Send at the end of each message. Can carry an arbitrary payload, to make messages more of a
    /// dropin for classical functions. Might be refined in the future. We anticipate the stop
    /// reason to be very useful for end appliacations. We also introduce end messages to keep the
    /// door open for multiple messages in a stream.
    MessageEnd { payload: Value },
    /// Append the internal string to the current message
    MessageAppend { text: String },
    /// An error occurred during skill execution. This kind of error can happen after streaming has
    /// started
    Error(String),
}

/// Message type used to transfer the input and output of a function skill execution
#[derive(Debug)]
pub struct RunFunctionMsg {
    pub skill_path: SkillPath,
    pub input: Value,
    pub send: oneshot::Sender<Result<Value, SkillExecutionError>>,
    pub api_token: String,
}

impl RunFunctionMsg {
    async fn act(
        self,
        csi_apis: impl Csi + Send + Sync + 'static,
        runtime: &WasmRuntime<impl SkillStoreApi>,
    ) {
        let start = Instant::now();
        let RunFunctionMsg {
            skill_path,
            input,
            send,
            api_token,
        } = self;

        let (send_rt_err, recv_rt_err) = oneshot::channel();
        let ctx = Box::new(SkillInvocationCtx::new(send_rt_err, csi_apis, api_token));
        let response = select! {
            result = runtime.run_function(&skill_path, input, ctx) => result,
            // An error occurred during skill execution.
            Ok(error) = recv_rt_err => Err(SkillExecutionError::RuntimeError(error))
        };

        let status = status_label(response.as_ref().map(|_| ()));
        // Error is expected to happen during shutdown. Ignore result.
        drop(send.send(response));
        record_skill_metrics(start, skill_path, status);
    }
}

/// Implementation of [`Csi`] provided to skills. It is responsible for forwarding the function
/// calls to csi, to the respective drivers and forwarding runtime errors directly to the actor
/// so the User defined code must not worry about accidental complexity.
pub struct SkillInvocationCtx<C> {
    /// This is used to send any runtime error (as opposed to logic error) back to the actor, so it
    /// can drop the future invoking the skill, and report the error appropriately to user and
    /// operator.
    send_rt_error: Option<oneshot::Sender<anyhow::Error>>,
    csi_apis: C,
    // How the user authenticates with us
    api_token: String,
    /// ID counter for stored streams.
    current_stream_id: usize,
    /// Currently running chat streams. We store them here so that we can easier cancel the running
    /// skill if there is an error in the stream. This is much harder to do if we use the normal `ResourceTable`.
    chat_streams: HashMap<ChatStreamId, mpsc::Receiver<anyhow::Result<inference::ChatEvent>>>,
    /// Currently running completion streams. We store them here so that we can easier cancel the running
    /// skill if there is an error in the stream. This is much harder to do if we use the normal `ResourceTable`.
    completion_streams:
        HashMap<CompletionStreamId, mpsc::Receiver<anyhow::Result<CompletionEvent>>>,
}

impl<C> SkillInvocationCtx<C> {
    pub fn new(
        send_rt_err: oneshot::Sender<anyhow::Error>,
        csi_apis: C,
        api_token: String,
    ) -> Self {
        SkillInvocationCtx {
            send_rt_error: Some(send_rt_err),
            csi_apis,
            api_token,
            current_stream_id: 0,
            chat_streams: HashMap::new(),
            completion_streams: HashMap::new(),
        }
    }

    fn next_stream_id<Id>(&mut self) -> Id
    where
        Id: From<usize>,
    {
        self.current_stream_id += 1;
        self.current_stream_id.into()
    }

    /// Never return, we did report the error via the send error channel.
    async fn send_error<T>(&mut self, error: anyhow::Error) -> T {
        self.send_rt_error
            .take()
            .expect("Only one error must be send during skill invocation")
            .send(error)
            .unwrap();
        pending().await
    }
}

#[async_trait]
impl<C> CsiForSkills for SkillInvocationCtx<C>
where
    C: Csi + Send + Sync,
{
    async fn explain(&mut self, requests: Vec<ExplanationRequest>) -> Vec<Explanation> {
        match self
            .csi_apis
            .explain(self.api_token.clone(), requests)
            .await
        {
            Ok(value) => value,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn complete(&mut self, requests: Vec<CompletionRequest>) -> Vec<Completion> {
        match self
            .csi_apis
            .complete(self.api_token.clone(), requests)
            .await
        {
            Ok(value) => value,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn completion_stream_new(&mut self, request: CompletionRequest) -> CompletionStreamId {
        let id = self.next_stream_id();
        let recv = self
            .csi_apis
            .completion_stream(self.api_token.clone(), request)
            .await;
        self.completion_streams.insert(id, recv);
        id
    }

    async fn completion_stream_next(&mut self, id: &CompletionStreamId) -> Option<CompletionEvent> {
        let event = self
            .completion_streams
            .get_mut(id)
            .expect("Stream not found")
            .recv()
            .await
            .transpose();
        match event {
            Ok(event) => event,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn completion_stream_drop(&mut self, id: CompletionStreamId) {
        self.completion_streams.remove(&id);
    }

    async fn chat(&mut self, requests: Vec<ChatRequest>) -> Vec<ChatResponse> {
        match self.csi_apis.chat(self.api_token.clone(), requests).await {
            Ok(value) => value,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn chat_stream_new(&mut self, request: ChatRequest) -> ChatStreamId {
        let id = self.next_stream_id();
        let recv = self
            .csi_apis
            .chat_stream(self.api_token.clone(), request)
            .await;
        self.chat_streams.insert(id, recv);
        id
    }

    async fn chat_stream_next(&mut self, id: &ChatStreamId) -> Option<inference::ChatEvent> {
        let event = self
            .chat_streams
            .get_mut(id)
            .expect("Stream not found")
            .recv()
            .await
            .transpose();
        match event {
            Ok(event) => event,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn chat_stream_drop(&mut self, id: ChatStreamId) {
        self.chat_streams.remove(&id);
    }

    async fn chunk(&mut self, requests: Vec<ChunkRequest>) -> Vec<Vec<Chunk>> {
        match self.csi_apis.chunk(self.api_token.clone(), requests).await {
            Ok(chunks) => chunks,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn select_language(
        &mut self,
        requests: Vec<SelectLanguageRequest>,
    ) -> Vec<Option<Language>> {
        match self.csi_apis.select_language(requests).await {
            Ok(language) => language,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn search(&mut self, requests: Vec<SearchRequest>) -> Vec<Vec<SearchResult>> {
        match self.csi_apis.search(self.api_token.clone(), requests).await {
            Ok(value) => value,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn documents(&mut self, requests: Vec<DocumentPath>) -> Vec<Document> {
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

/// We know that skill metadata will not invoke any csi functions, but still need to provide an implementation of `CsiForSkills`
/// We do not want to panic, as someone could build a component that uses the csi functions inside the metadata function.
/// Therefore, we always send a runtime error which will lead to skill suspension.
struct SkillMetadataCtx {
    send_rt_err: Option<oneshot::Sender<anyhow::Error>>,
}

impl SkillMetadataCtx {
    pub fn new(send_rt_err: oneshot::Sender<anyhow::Error>) -> Self {
        Self {
            send_rt_err: Some(send_rt_err),
        }
    }

    async fn send_error<T>(&mut self) -> T {
        self.send_rt_err
            .take()
            .expect("Only one error must be send during skill invocation")
            .send(anyhow!(
                "This message will be translated and thus never seen by a user"
            ))
            .unwrap();
        pending().await
    }
}

#[async_trait]
impl CsiForSkills for SkillMetadataCtx {
    async fn explain(&mut self, _requests: Vec<ExplanationRequest>) -> Vec<Explanation> {
        self.send_error().await
    }

    async fn complete(&mut self, _requests: Vec<CompletionRequest>) -> Vec<Completion> {
        self.send_error().await
    }

    async fn completion_stream_new(&mut self, _request: CompletionRequest) -> CompletionStreamId {
        self.send_error().await
    }

    async fn completion_stream_next(
        &mut self,
        _id: &CompletionStreamId,
    ) -> Option<CompletionEvent> {
        self.send_error().await
    }

    async fn completion_stream_drop(&mut self, _id: CompletionStreamId) {
        self.send_error::<()>().await;
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

    async fn chat_stream_new(&mut self, _request: ChatRequest) -> ChatStreamId {
        self.send_error().await
    }

    async fn chat_stream_next(&mut self, _id: &ChatStreamId) -> Option<inference::ChatEvent> {
        self.send_error().await
    }

    async fn chat_stream_drop(&mut self, _id: ChatStreamId) {
        self.send_error::<()>().await;
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

#[cfg(test)]
pub mod tests {
    use std::time::Duration;

    use crate::csi::tests::CsiCompleteStub;
    use crate::csi::tests::CsiSaboteur;
    use crate::hardcoded_skills::SkillHello;
    use crate::hardcoded_skills::SkillSaboteur;
    use crate::hardcoded_skills::SkillTellMeAJoke;
    use crate::inference::CompletionParams;
    use crate::inference::{
        ChatParams, Explanation, FinishReason, Granularity, Logprobs, Message, TextScore,
        TokenUsage,
    };
    use crate::namespace_watcher::Namespace;
    use crate::skill_store::tests::SkillStoreStub;
    use crate::skills::{AnySkillManifest, Skill};
    use crate::{
        chunking::ChunkParams,
        csi::tests::{CsiDummy, StubCsi},
        skill_loader::{RegistryConfig, SkillLoader},
        skill_store::SkillStore,
    };
    use anyhow::anyhow;
    use metrics::Label;
    use metrics_util::debugging::DebugValue;
    use metrics_util::debugging::{DebuggingRecorder, Snapshot};
    use serde_json::json;
    use tokio::sync::broadcast;

    use super::*;

    #[tokio::test]
    async fn csi_usage_from_metadata_leads_to_suspension() {
        // Given a skill runtime that always returns a skill that uses the csi from the metadata function
        struct CsiFromMetadataSkill;
        #[async_trait]
        impl Skill for CsiFromMetadataSkill {
            async fn manifest(
                &self,
                _engine: &Engine,
                mut ctx: Box<dyn CsiForSkills + Send>,
            ) -> Result<AnySkillManifest, SkillError> {
                ctx.select_language(vec![SelectLanguageRequest {
                    text: "Hello, good sir!".to_owned(),
                    languages: Vec::new(),
                }])
                .await;
                unreachable!(
                    "The test should never reach this point, as its execution shoud be suspendend"
                )
            }

            async fn run_as_function(
                &self,
                _engine: &Engine,
                _ctx: Box<dyn CsiForSkills + Send>,
                _input: Value,
            ) -> Result<Value, SkillError> {
                unreachable!("This won't be invoked during the test")
            }

            async fn run_as_generator(
                &self,
                _engine: &Engine,
                _ctx: Box<dyn CsiForSkills + Send>,
                _input: Value,
                _sender: mpsc::Sender<StreamEvent>,
            ) -> Result<(), SkillError> {
                unreachable!("This won't be invoked during the test")
            }
        }

        let mut store = SkillStoreStub::new();
        store.with_fetch_response(Some(Arc::new(CsiFromMetadataSkill)));

        let skill_path = SkillPath::local("invoke_csi_from_metadata");
        let engine = Arc::new(Engine::new(false).unwrap());
        let runtime = SkillRuntime::new(engine, CsiSaboteur, store);

        // When metadata for a skill is requested
        let metadata = runtime.api().skill_metadata(skill_path).await;
        runtime.wait_for_shutdown().await;

        // Then the metadata is None
        assert_eq!(
            metadata.unwrap_err().to_string(),
            "The metadata function of the invoked skill is bugged. It is forbidden to invoke any \
            CSI functions from the metadata function, yet the skill does precisely this."
        );
    }

    #[tokio::test]
    async fn forward_explain_response_from_csi() {
        struct SkillDoubleUsingExplain;

        #[async_trait]
        impl Skill for SkillDoubleUsingExplain {
            async fn run_as_function(
                &self,
                _engine: &Engine,
                mut ctx: Box<dyn CsiForSkills + Send>,
                _input: Value,
            ) -> Result<Value, SkillError> {
                let explanation = ctx
                    .explain(vec![ExplanationRequest {
                        prompt: "An apple a day".to_owned(),
                        target: " keeps the doctor away".to_owned(),
                        model: "test-model-name".to_owned(),
                        granularity: Granularity::Auto,
                    }])
                    .await
                    .pop()
                    .unwrap();
                let output = explanation
                    .into_iter()
                    .map(|text_score| json!({"start": text_score.start, "length": text_score.length}))
                    .collect::<Vec<_>>();
                Ok(json!(output))
            }

            async fn manifest(
                &self,
                _engine: &Engine,
                _ctx: Box<dyn CsiForSkills + Send>,
            ) -> Result<AnySkillManifest, SkillError> {
                panic!("Dummy metadata implementation of SkillDoubleUsingExplain")
            }

            async fn run_as_generator(
                &self,
                _engine: &Engine,
                _ctx: Box<dyn CsiForSkills + Send>,
                _input: Value,
                _sender: mpsc::Sender<StreamEvent>,
            ) -> Result<(), SkillError> {
                panic!("Dummy generator implementation of SkillDoubleUsingExplain")
            }
        }

        let skill_path = SkillPath::local("explain");
        let engine = Arc::new(Engine::new(false).unwrap());
        let mut store = SkillStoreStub::new();
        store.with_fetch_response(Some(Arc::new(SkillDoubleUsingExplain)));
        let (send, _) = oneshot::channel();
        let csi = StubCsi::with_explain(|_| {
            Explanation::new(vec![TextScore {
                score: 0.0,
                start: 0,
                length: 2,
            }])
        });
        let skill_ctx = Box::new(SkillInvocationCtx::new(send, csi, "dummy token".to_owned()));

        let runtime = WasmRuntime::new(engine, store);
        let resp = runtime
            .run_function(
                &skill_path,
                json!({"prompt": "An apple a day", "target": " keeps the doctor away"}),
                skill_ctx,
            )
            .await;

        drop(runtime);

        assert_eq!(resp.unwrap(), json!([{"start": 0, "length": 2}]));
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
            .run_function(&SkillPath::dummy(), json!("name"), skill_ctx)
            .await;

        drop(runtime);
        skill_store.wait_for_shutdown().await;

        assert!(resp.is_err());
    }

    #[tokio::test]
    async fn chunk() {
        // Given a skill invocation context with a stub tokenizer provider
        let (send, _) = oneshot::channel();
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

        let mut invocation_ctx = SkillInvocationCtx::new(send, csi, "dummy token".to_owned());

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
        let mut csi = StubCsi::empty();
        csi.set_chunking(|_| Err(anyhow!("Failed to load tokenizer")));
        let mut invocation_ctx = SkillInvocationCtx::new(send, csi, "dummy token".to_owned());

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
        let csi_apis = CsiDummy;
        let executer = SkillRuntime::new(engine, csi_apis, skill_store.api());
        let api = executer.api();

        // When a skill is requested, but it is not listed in the namespace
        let result = api
            .run_function(
                SkillPath::local("my_skill"),
                json!("Any input"),
                "Dummy api token".to_owned(),
            )
            .await;

        drop(api);
        executer.wait_for_shutdown().await;
        skill_store.wait_for_shutdown().await;

        // Then result indicates that the skill is missing
        assert!(matches!(
            result,
            Err(SkillExecutionError::SkillNotConfigured)
        ));
    }

    #[tokio::test]
    async fn skill_runtime_forwards_csi_errors() {
        // Given a skill using csi and a csi that fails
        let mut store = SkillStoreStub::new();
        // Note we are using a skill which actually invokes the csi
        store.with_fetch_response(Some(Arc::new(SkillGreetCompletion)));
        let engine = Arc::new(Engine::new(false).unwrap());
        let runtime = SkillRuntime::new(engine, CsiSaboteur, store);

        // When trying to generate a greeting for Homer using the greet skill
        let result = runtime
            .api()
            .run_function(
                SkillPath::local("greet"),
                json!("Homer"),
                "TOKEN_NOT_REQUIRED".to_owned(),
            )
            .await;

        runtime.wait_for_shutdown().await;

        // Then
        let expectet_error_msg = "The skill could not be executed to completion, something in our \
            runtime is currently \nunavailable or misconfigured. You should try again later, if \
            the situation persists you \nmay want to contact the operaters. Original error:\n\n\
            Test error";
        assert_eq!(result.unwrap_err().to_string(), expectet_error_msg);
    }

    #[tokio::test]
    async fn greeting_skill_should_output_hello() {
        // Given
        let skill = GreetSkill;
        let csi = CsiDummy;
        let engine = Arc::new(Engine::new(false).unwrap());
        let mut store = SkillStoreStub::new();
        store.with_fetch_response(Some(Arc::new(skill)));

        // When
        let runtime = SkillRuntime::new(engine, csi, store);
        let result = runtime
            .api()
            .run_function(
                SkillPath::local("greet"),
                json!(""),
                "TOKEN_NOT_REQUIRED".to_owned(),
            )
            .await;
        runtime.wait_for_shutdown().await;

        // Then
        assert_eq!(result.unwrap(), "Hello");
    }

    #[tokio::test]
    async fn concurrent_skill_execution() {
        // Given
        struct SkillAssertConcurrent {
            send: broadcast::Sender<()>,
        }

        #[async_trait]
        impl Skill for SkillAssertConcurrent {
            async fn run_as_function(
                &self,
                _engine: &Engine,
                _ctx: Box<dyn CsiForSkills + Send>,
                _input: Value,
            ) -> Result<Value, SkillError> {
                let mut recv = self.send.subscribe();
                self.send.send(()).unwrap();
                // Send once await two responses. This way we can only finish any skill if we
                // actually execute them concurrently. Two for the two invocations in this test
                recv.recv().await.unwrap();
                recv.recv().await.unwrap();
                // We finished, lets unblock our counterpart, in case it missed a broadcast
                self.send.send(()).unwrap();
                Ok(json!("Hello"))
            }

            async fn manifest(
                &self,
                _engine: &Engine,
                _ctx: Box<dyn CsiForSkills + Send>,
            ) -> Result<AnySkillManifest, SkillError> {
                panic!("Dummy metadata implementation of Assert concurrency skill")
            }

            async fn run_as_generator(
                &self,
                _engine: &Engine,
                _ctx: Box<dyn CsiForSkills + Send>,
                _input: Value,
                _mpsc: mpsc::Sender<StreamEvent>,
            ) -> Result<(), SkillError> {
                panic!("Dummy generator implementation of Assert concurrency skill")
            }
        }

        let (send, _recv) = broadcast::channel(2);
        let skill = SkillAssertConcurrent { send: send.clone() };
        let engine = Arc::new(Engine::new(false).unwrap());
        let mut store = SkillStoreStub::new();
        store.with_fetch_response(Some(Arc::new(skill)));
        let runtime = SkillRuntime::new(engine, CsiDummy, store);

        // When invoking two skills in parallel
        let token = "TOKEN_NOT_REQUIRED";

        let api_first = runtime.api();
        let first = tokio::spawn(async move {
            api_first
                .run_function(SkillPath::local("any_path"), json!({}), token.to_owned())
                .await
        });
        let api_second = runtime.api();
        let second = tokio::spawn(async move {
            api_second
                .run_function(SkillPath::local("any_path"), json!({}), token.to_owned())
                .await
        });
        let result_first = tokio::time::timeout(Duration::from_secs(1), first).await;
        let result_second = tokio::time::timeout(Duration::from_secs(1), second).await;

        assert!(result_first.is_ok());
        assert!(result_second.is_ok());

        runtime.wait_for_shutdown().await;
    }

    #[tokio::test]
    async fn stream_hello_test() {
        // Given
        let engine = Arc::new(Engine::new(false).unwrap());
        let mut store = SkillStoreStub::new();
        store.with_fetch_response(Some(Arc::new(SkillHello)));
        let runtime = SkillRuntime::new(engine, CsiDummy, store);

        // When
        let mut recv = runtime
            .api()
            .run_stream(
                SkillPath::new(Namespace::new("test-beta").unwrap(), "hello"),
                json!(""),
                "TOKEN_NOT_REQUIRED".to_owned(),
            )
            .await;

        // Then
        assert_eq!(recv.recv().await.unwrap(), StreamEvent::MessageBegin);
        assert_eq!(
            recv.recv().await.unwrap(),
            StreamEvent::MessageAppend {
                text: "H".to_string()
            }
        );
        assert_eq!(
            recv.recv().await.unwrap(),
            StreamEvent::MessageAppend {
                text: "e".to_string()
            }
        );
        assert_eq!(
            recv.recv().await.unwrap(),
            StreamEvent::MessageAppend {
                text: "l".to_string()
            }
        );
        assert_eq!(
            recv.recv().await.unwrap(),
            StreamEvent::MessageAppend {
                text: "l".to_string()
            }
        );
        assert_eq!(
            recv.recv().await.unwrap(),
            StreamEvent::MessageAppend {
                text: "o".to_string()
            }
        );
        assert_eq!(
            recv.recv().await.unwrap(),
            StreamEvent::MessageEnd {
                payload: json!(null)
            }
        );
        assert!(recv.recv().await.is_none());

        // Cleanup
        runtime.wait_for_shutdown().await;
    }

    #[tokio::test]
    async fn stream_saboteur_test() {
        // Given
        let engine = Arc::new(Engine::new(false).unwrap());
        let mut store = SkillStoreStub::new();
        store.with_fetch_response(Some(Arc::new(SkillSaboteur)));
        let runtime = SkillRuntime::new(engine, CsiDummy, store);

        // When
        let mut recv = runtime
            .api()
            .run_stream(
                SkillPath::new(Namespace::new("test-beta").unwrap(), "saboteur"),
                json!(""),
                "TOKEN_NOT_REQUIRED".to_owned(),
            )
            .await;

        // Then
        let expected_error_msg = "The skill you called responded with an error. Maybe you should \
            check your input, if it seems to be correct you may want to contact the skill \
            developer. Error reported by Skill:\n\nSkill is a saboteur";
        assert_eq!(
            recv.recv().await.unwrap(),
            StreamEvent::Error(expected_error_msg.to_string())
        );
        assert!(recv.recv().await.is_none());

        // Cleanup
        runtime.wait_for_shutdown().await;
    }

    #[test]
    fn skill_runtime_metrics_emitted() {
        let skill_path = SkillPath::local("greet");
        let mut store = SkillStoreStub::new();
        store.with_fetch_response(Some(Arc::new(GreetSkill)));
        let engine = Arc::new(Engine::new(false).unwrap());
        let (send, _) = oneshot::channel();
        let msg = RunFunctionMsg {
            skill_path: skill_path.clone(),
            input: json!("Hello"),
            send,
            api_token: "dummy".to_owned(),
        };

        // Metrics requires sync, so all of the async parts are moved into this closure.
        let snapshot = metrics_snapshot(async || {
            let runtime = WasmRuntime::new(engine, store);
            msg.act(CsiDummy, &runtime).await;
            drop(runtime);
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

    #[tokio::test]
    async fn stream_skill_should_emit_error_in_case_of_runtime_error_in_csi() {
        // Given
        let engine = Arc::new(Engine::new(false).unwrap());
        let mut store = SkillStoreStub::new();
        store.with_fetch_response(Some(Arc::new(SkillTellMeAJoke)));
        let runtime = SkillRuntime::new(engine, CsiSaboteur, store);
        let skill_path = SkillPath::new(Namespace::new("test-beta").unwrap(), "tell_me_a_joke");

        // When
        let mut recv = runtime
            .api()
            .run_stream(skill_path, json!({}), "dumm_token".to_owned())
            .await;

        // Then
        let event = recv.recv().await.unwrap();
        let expected_error_msg = "The skill could not be executed to completion, something in our \
            runtime is currently \nunavailable or misconfigured. You should try again later, if \
            the situation persists you \nmay want to contact the operaters. Original error:\n\n\
            Test error";
        assert_eq!(event, StreamEvent::Error(expected_error_msg.to_string()));
    }

    #[tokio::test]
    async fn skill_invocation_ctx_stream_management() {
        let (send, _) = oneshot::channel();
        let completion = Completion {
            text: "text".to_owned(),
            finish_reason: FinishReason::Stop,
            logprobs: vec![],
            usage: TokenUsage {
                prompt: 2,
                completion: 2,
            },
        };
        let resp = completion.clone();
        let csi = StubCsi::with_completion(move |_| resp.clone());
        let mut ctx = SkillInvocationCtx::new(send, csi, "dummy".to_owned());
        let request = CompletionRequest::new("prompt", "model");

        let stream_id = ctx.completion_stream_new(request).await;
        let mut events = vec![];
        while let Some(event) = ctx.completion_stream_next(&stream_id).await {
            events.push(event);
        }
        ctx.completion_stream_drop(stream_id).await;

        assert_eq!(
            events,
            vec![
                CompletionEvent::Delta {
                    text: completion.text,
                    logprobs: completion.logprobs
                },
                CompletionEvent::Finished {
                    finish_reason: completion.finish_reason
                },
                CompletionEvent::Usage {
                    usage: completion.usage
                }
            ]
        );
        assert!(ctx.completion_streams.is_empty());
    }

    #[tokio::test]
    async fn skill_invocation_ctx_chat_stream_management() {
        let (send, _) = oneshot::channel();
        let response = ChatResponse {
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
        };
        let stub_response = response.clone();
        let csi = StubCsi::with_chat(move |_| stub_response.clone());
        let mut ctx = SkillInvocationCtx::new(send, csi, "dummy".to_owned());
        let request = ChatRequest {
            model: "model".to_owned(),
            messages: vec![],
            params: ChatParams::default(),
        };

        let stream_id = ctx.chat_stream_new(request).await;
        let mut events = vec![];
        while let Some(event) = ctx.chat_stream_next(&stream_id).await {
            events.push(event);
        }
        ctx.chat_stream_drop(stream_id).await;

        assert_eq!(
            events,
            vec![
                inference::ChatEvent::MessageStart {
                    role: response.message.role,
                },
                inference::ChatEvent::MessageDelta {
                    content: response.message.content,
                    logprobs: response.logprobs,
                },
                inference::ChatEvent::MessageEnd {
                    finish_reason: response.finish_reason
                },
                inference::ChatEvent::Usage {
                    usage: response.usage
                }
            ]
        );
        assert!(ctx.completion_streams.is_empty());
    }

    fn metrics_snapshot<F: Future<Output = ()>>(f: impl FnOnce() -> F) -> Snapshot {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        metrics::with_local_recorder(&recorder, || runtime.block_on(f()));
        snapshotter.snapshot()
    }

    /// A skill implementation for testing purposes. It sends a greeting to the user.
    struct GreetSkill;

    #[async_trait]
    impl Skill for GreetSkill {
        async fn run_as_function(
            &self,
            _engine: &Engine,
            _ctx: Box<dyn CsiForSkills + Send>,
            _input: Value,
        ) -> Result<Value, SkillError> {
            Ok(json!("Hello"))
        }

        async fn manifest(
            &self,
            _engine: &Engine,
            _ctx: Box<dyn CsiForSkills + Send>,
        ) -> Result<AnySkillManifest, SkillError> {
            panic!("Dummy metadata implementation of Greet Skill")
        }

        async fn run_as_generator(
            &self,
            _engine: &Engine,
            _ctx: Box<dyn CsiForSkills + Send>,
            _input: Value,
            _sender: mpsc::Sender<StreamEvent>,
        ) -> Result<(), SkillError> {
            Err(SkillError::IsFunction)
        }
    }

    /// A test double for a skill. It invokes the csi with a prompt and returns the result.
    struct SkillGreetCompletion;

    #[async_trait]
    impl Skill for SkillGreetCompletion {
        async fn run_as_function(
            &self,
            _engine: &Engine,
            mut ctx: Box<dyn CsiForSkills + Send>,
            _input: Value,
        ) -> Result<Value, SkillError> {
            let mut completions = ctx
                .complete(vec![CompletionRequest {
                    prompt: "Hello".to_owned(),
                    model: "test-model-name".to_owned(),
                    params: CompletionParams {
                        max_tokens: Some(10),
                        temperature: Some(0.5),
                        top_p: Some(1.0),
                        presence_penalty: Some(0.0),
                        frequency_penalty: Some(0.0),
                        stop: Vec::new(),
                        return_special_tokens: true,
                        top_k: None,
                        logprobs: Logprobs::No,
                    },
                }])
                .await;
            let completion = completions.pop().unwrap().text;
            Ok(json!(completion))
        }

        async fn manifest(
            &self,
            _engine: &Engine,
            _ctx: Box<dyn CsiForSkills + Send>,
        ) -> Result<AnySkillManifest, SkillError> {
            panic!("Dummy metadata implementation of Skill Greet Completion")
        }

        async fn run_as_generator(
            &self,
            _engine: &Engine,
            _ctx: Box<dyn CsiForSkills + Send>,
            _input: Value,
            _sender: mpsc::Sender<StreamEvent>,
        ) -> Result<(), SkillError> {
            Err(SkillError::IsFunction)
        }
    }
}
