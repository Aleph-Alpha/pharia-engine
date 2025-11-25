use std::{collections::HashMap, future::pending, sync::Arc};

use anyhow::anyhow;
use async_trait::async_trait;
use serde_json::Value;
use thiserror::Error;
use tokio::{
    select,
    sync::{mpsc, oneshot},
};

use crate::{
    chunking::{Chunk, ChunkRequest},
    csi::{ChatStreamId, CompletionStreamId, ContextualCsi, Csi, ToolResult},
    inference::{
        ChatEvent, ChatEventV2, ChatRequest, ChatResponse, ChatResponseV2, Completion,
        CompletionEvent, CompletionRequest, Explanation, ExplanationRequest, InferenceError,
    },
    language_selection::{Language, SelectLanguageRequest},
    logging::TracingContext,
    namespace_watcher::Namespace,
    search::{Document, DocumentPath, SearchRequest, SearchResult},
    skill::{AnySkillManifest, Skill, SkillError, SkillEvent},
    tool::{InvokeRequest, ToolDescription},
    wasm::SkillLoadError,
};

pub struct SkillDriver;

impl SkillDriver {
    pub async fn run_message_stream(
        &self,
        skill: Arc<dyn Skill>,
        input: Value,
        contextual_csi: impl ContextualCsi + Send + Sync + 'static,
        tracing_context: &TracingContext,
        sender: mpsc::Sender<SkillExecutionEvent>,
    ) -> Result<(), SkillExecutionError> {
        let (send_ctx_event, mut recv_ctx_event) = mpsc::channel(1);
        let csi = Box::new(SkillInvocationCtx::new(send_ctx_event, contextual_csi));

        let (send_skill_event, mut recv_skill_event) = mpsc::channel(1);

        let mut execute_skill =
            skill.run_as_message_stream(csi, input, send_skill_event, tracing_context);

        let mut translator = EventTranslator::new();

        let result = loop {
            select! {
                // Controls the polling order. We want to ensure that we poll runtime errors first,
                // then messages, and finally the result of the skill handler. It is suitable to
                // check for the runtime errors first, since we would then error out and terminate
                // anyways. We can not rely on it for correctness, but we poll the before the
                // receiver of events before the handler.
                biased;

                Some(event) = recv_ctx_event.recv() => {
                    match event {
                        SkillCtxEvent::Quit(error) => {
                            let error = SkillExecutionError::RuntimeError(error.to_string());
                            drop(sender.send(SkillExecutionEvent::Error(error.clone())).await);
                            break Err(error);
                        }
                        SkillCtxEvent::ToolBegin { name } => {
                            drop(sender.send(SkillExecutionEvent::ToolBegin { name }).await);
                        }
                        SkillCtxEvent::ToolEnd { name, result } => {
                            drop(sender.send(SkillExecutionEvent::ToolEnd { name, result }).await);
                        }
                    }
                }
                Some(skill_event) = recv_skill_event.recv() => {
                    let execution_event =
                        translator.translate_to_execution_event(skill_event);
                    let maybe_error = execution_event.execution_error().cloned();
                    drop(sender.send(execution_event).await);
                    if let Some(error) = maybe_error {
                        break Err(error);
                    }
                }
                result = &mut execute_skill => {
                    // Skill error to skill execution error
                    let result = result.map_err(Into::<SkillExecutionError>::into);
                    if let Err(err) = &result {
                        drop(sender
                            .send(SkillExecutionEvent::Error(err.clone()))
                            .await);
                    }
                    break result;
                }
            }
        };

        // In case the skill invocation finishes faster than we could extract the last event. I.e.
        // the event is placed in the channel, yet the receiver did not pick it up yet.
        if let Ok(skill_event) = recv_skill_event.try_recv() {
            let execution_event = translator.translate_to_execution_event(skill_event);
            let maybe_error = execution_event.execution_error().cloned();
            drop(sender.send(execution_event).await);
            if let Some(error) = maybe_error {
                return Err(error);
            }
        }

        result
    }

    pub async fn run_function(
        &self,
        skill: Arc<dyn Skill>,
        input: Value,
        contextual_csi: impl ContextualCsi + Send + Sync + 'static,
        tracing_context: &TracingContext,
    ) -> Result<Value, SkillExecutionError> {
        let (send_rt_event, mut recv_rt_event) = mpsc::channel(1);
        let csi = Box::new(SkillInvocationCtx::new(send_rt_event, contextual_csi));
        select! {
            result = skill.run_as_function(csi, input, tracing_context) => result.map_err(Into::into),
            // An error occurred during skill execution. We ignore any other events for running
            // functions for now.
            Some(SkillCtxEvent::Quit(error)) = recv_rt_event.recv() => Err(SkillExecutionError::RuntimeError(error.to_string())),
        }
    }

    pub async fn metadata(
        &self,
        skill: Arc<dyn Skill>,
        tracing_context: &TracingContext,
    ) -> Result<AnySkillManifest, SkillExecutionError> {
        let (send_rt_err, recv_rt_err) = oneshot::channel();
        let ctx = Box::new(SkillMetadataCtx::new(send_rt_err));
        select! {
            result = skill.manifest(ctx, tracing_context) => result.map_err(Into::into),
            // An error occurred during skill execution.
            Ok(_error) = recv_rt_err => Err(SkillExecutionError::CsiUseFromMetadata)
        }
    }
}

/// Events emitted by the Skill Invocation Context.
///
/// The caller can choose appropriate action. For example, if an unrecoverable error occurs while
/// resolving a csi call, this event guides the caller to terminate the skill execution.
pub enum SkillCtxEvent {
    /// This is used to send any runtime error (as opposed to logic error) back to the actor, so it
    /// can drop the future invoking the skill, and report the error appropriately to user and
    /// operator.
    Quit(anyhow::Error),
    /// A request for a tool call has been made.
    ToolBegin { name: String },
    /// A tool call has been completed.
    ToolEnd {
        name: String,
        result: Result<(), String>,
    },
}

/// Implementation of [`Csi`] provided to skills. It is responsible for forwarding the function
/// calls to csi, to the respective drivers and forwarding runtime errors directly to the actor
/// so the User defined code must not worry about accidental complexity.
pub struct SkillInvocationCtx<C> {
    /// This channel is used to notify the caller about events that happened around skill
    /// execution.
    send_rt_event: mpsc::Sender<SkillCtxEvent>,
    /// Provides the CSI functionality to Skills while encapsulating knowledge about the
    /// invocation.
    contextual_csi: C,
    /// ID counter for stored streams.
    current_stream_id: usize,
    /// Currently running chat streams. We store them here so that we can easier cancel the running
    /// skill if there is an error in the stream. This is much harder to do if we use the normal
    /// `ResourceTable`.
    chat_streams: HashMap<ChatStreamId, mpsc::Receiver<Result<ChatEvent, InferenceError>>>,
    chat_streams_v2: HashMap<ChatStreamId, mpsc::Receiver<Result<ChatEventV2, InferenceError>>>,
    /// Currently running completion streams. We store them here so that we can easier cancel the
    /// running skill if there is an error in the stream. This is much harder to do if we use
    /// the normal `ResourceTable`.
    completion_streams:
        HashMap<CompletionStreamId, mpsc::Receiver<Result<CompletionEvent, InferenceError>>>,
}

impl<C> SkillInvocationCtx<C> {
    pub fn new(send_rt_event: mpsc::Sender<SkillCtxEvent>, contextual_csi: C) -> Self {
        SkillInvocationCtx {
            send_rt_event,
            contextual_csi,
            current_stream_id: 0,
            chat_streams: HashMap::new(),
            chat_streams_v2: HashMap::new(),
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
        self.send_rt_event
            .send(SkillCtxEvent::Quit(error))
            .await
            .unwrap();
        pending().await
    }
}

#[async_trait]
impl<C> Csi for SkillInvocationCtx<C>
where
    C: ContextualCsi + Send + Sync,
{
    async fn explain(&mut self, requests: Vec<ExplanationRequest>) -> Vec<Explanation> {
        match self.contextual_csi.explain(requests).await {
            Ok(value) => value,
            Err(error) => self.send_error(error.into()).await,
        }
    }

    async fn complete(&mut self, requests: Vec<CompletionRequest>) -> Vec<Completion> {
        match self.contextual_csi.complete(requests).await {
            Ok(value) => value,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn completion_stream_new(&mut self, request: CompletionRequest) -> CompletionStreamId {
        let id = self.next_stream_id();
        let recv = self.contextual_csi.completion_stream(request).await;
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
            Err(error) => self.send_error(error.into()).await,
        }
    }

    async fn completion_stream_drop(&mut self, id: CompletionStreamId) {
        self.completion_streams.remove(&id);
    }

    async fn chat(&mut self, requests: Vec<ChatRequest>) -> Vec<ChatResponse> {
        match self.contextual_csi.chat(requests).await {
            Ok(value) => value,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn chat_v2(&mut self, requests: Vec<ChatRequest>) -> Vec<ChatResponseV2> {
        match self.contextual_csi.chat_v2(requests).await {
            Ok(value) => value,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn chat_stream_new(&mut self, request: ChatRequest) -> ChatStreamId {
        let id = self.next_stream_id();
        let recv = self.contextual_csi.chat_stream(request).await;
        self.chat_streams.insert(id, recv);
        id
    }

    async fn chat_stream_new_v2(&mut self, request: ChatRequest) -> ChatStreamId {
        let id = self.next_stream_id();
        let recv = self.contextual_csi.chat_stream_v2(request).await;
        self.chat_streams_v2.insert(id, recv);
        id
    }

    async fn chat_stream_next(&mut self, id: &ChatStreamId) -> Option<ChatEvent> {
        let event = self
            .chat_streams
            .get_mut(id)
            .expect("Stream not found")
            .recv()
            .await
            .transpose();
        match event {
            Ok(event) => event,
            Err(error) => self.send_error(error.into()).await,
        }
    }

    async fn chat_stream_next_v2(&mut self, id: &ChatStreamId) -> Option<ChatEventV2> {
        let event = self
            .chat_streams_v2
            .get_mut(id)
            .expect("Stream not found")
            .recv()
            .await
            .transpose();
        match event {
            Ok(event) => event,
            Err(error) => self.send_error(error.into()).await,
        }
    }

    async fn chat_stream_drop(&mut self, id: ChatStreamId) {
        self.chat_streams.remove(&id);
        self.chat_streams_v2.remove(&id);
    }

    async fn chunk(&mut self, requests: Vec<ChunkRequest>) -> Vec<Vec<Chunk>> {
        match self.contextual_csi.chunk(requests).await {
            Ok(chunks) => chunks,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn select_language(
        &mut self,
        requests: Vec<SelectLanguageRequest>,
    ) -> Vec<Option<Language>> {
        match self.contextual_csi.select_language(requests).await {
            Ok(language) => language,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn search(&mut self, requests: Vec<SearchRequest>) -> Vec<Vec<SearchResult>> {
        match self.contextual_csi.search(requests).await {
            Ok(value) => value,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn documents(&mut self, requests: Vec<DocumentPath>) -> Vec<Document> {
        match self.contextual_csi.documents(requests).await {
            Ok(value) => value,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn document_metadata(&mut self, requests: Vec<DocumentPath>) -> Vec<Option<Value>> {
        match self.contextual_csi.document_metadata(requests).await {
            // We know there will always be exactly one element in the vector
            Ok(value) => value,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn invoke_tool(&mut self, requests: Vec<InvokeRequest>) -> Vec<ToolResult> {
        let names = requests
            .iter()
            .map(|request| request.name.clone())
            .collect::<Vec<_>>();
        for name in &names {
            self.send_rt_event
                .send(SkillCtxEvent::ToolBegin { name: name.clone() })
                .await
                .unwrap();
        }
        // While from a skill point of view, it is fine that all tool results are returned at once,
        // in here we would profit if tool results where queried individually from the contextual
        // csi. This would allow us to notify the caller earlier about the result of a tool call.
        let results = self
            .contextual_csi
            .invoke_tool(requests)
            .await
            .into_iter()
            .map(|result| result.map_err(|error| error.to_string()))
            .collect::<Vec<_>>();

        // The order of the results is guaranteed to be the same as the order of the requests.
        for (name, result) in names.iter().zip(results.iter()) {
            self.send_rt_event
                .send(SkillCtxEvent::ToolEnd {
                    name: name.clone(),
                    result: match result {
                        Ok(_) => Ok(()),
                        Err(error) => Err(error.clone()),
                    },
                })
                .await
                .unwrap();
        }
        results
    }

    async fn list_tools(&mut self) -> Vec<ToolDescription> {
        match self.contextual_csi.list_tools().await {
            Ok(value) => value,
            Err(error) => self.send_error(error).await,
        }
    }
}

/// We know that skill metadata will not invoke any CSI functions, but still need to provide an
/// implementation of `Csi` We do not want to panic, as someone could build a component that uses
/// the csi functions inside the metadata function. Therefore, we always send a runtime error which
/// will lead to skill suspension.
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
impl Csi for SkillMetadataCtx {
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

    async fn chat_v2(&mut self, _requests: Vec<ChatRequest>) -> Vec<ChatResponseV2> {
        self.send_error().await
    }

    async fn chat_stream_new(&mut self, _request: ChatRequest) -> ChatStreamId {
        self.send_error().await
    }

    async fn chat_stream_new_v2(&mut self, _request: ChatRequest) -> ChatStreamId {
        self.send_error().await
    }

    async fn chat_stream_next(&mut self, _id: &ChatStreamId) -> Option<ChatEvent> {
        self.send_error().await
    }

    async fn chat_stream_next_v2(&mut self, _id: &ChatStreamId) -> Option<ChatEventV2> {
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

    async fn invoke_tool(&mut self, _request: Vec<InvokeRequest>) -> Vec<ToolResult> {
        self.send_error().await
    }

    async fn list_tools(&mut self) -> Vec<ToolDescription> {
        self.send_error().await
    }
}

/// Emitted from executing message stream skills. This differs from [`SkillEvent`] in the way that
/// it has stronger guarantees. E.g. every [`Self::MessageEnd`] is prefaced by a
/// [`Self::MessageBegin`]. In addition to this, it also accounts for runtime errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkillExecutionEvent {
    /// Send at the beginning of each message, currently carries no information. May be used in the
    /// future to communicate the role. Can also be useful to the UI to communicate that it's about
    /// time to start rendering that speech bubble.
    MessageBegin,
    /// Send at the end of each message. Can carry an arbitrary payload, to make messages more of a
    /// drop-in for classical functions. Might be refined in the future. We anticipate the stop
    /// reason to be very useful for end applications. We also introduce end messages to keep the
    /// door open for multiple messages in a stream.
    MessageEnd { payload: Value },
    /// A chunk of reasoning generated by the skill.
    Reasoning { text: String },
    /// Append the internal string to the current message
    MessageAppend { text: String },
    /// The Skill has requested a tool call.
    ToolBegin { name: String },
    /// A tool call has been completed. Can be success or error.
    ToolEnd {
        name: String,
        result: Result<(), String>,
    },
    /// An error occurred during skill execution. This kind of error can happen after streaming has
    /// started
    Error(SkillExecutionError),
}

impl SkillExecutionEvent {
    pub fn execution_error(&self) -> Option<&SkillExecutionError> {
        if let SkillExecutionEvent::Error(e) = self {
            Some(e)
        } else {
            None
        }
    }
}

/// Translates [`SkillEvent`]s emitted by skills to [`SkillExecutionEvent`]s. It also keeps track
/// of message begin and end in order to detect invalid state transitions.
struct EventTranslator {
    /// `true` if the stream is in between a begin and end message event. A message is active after
    /// a begin and becomes inactive after an end event.
    message_active: bool,
}

impl EventTranslator {
    fn new() -> Self {
        Self {
            message_active: false,
        }
    }

    /// Output the associated [`SkillExecutionEvent`] for a given [`SkillEvent`]. In case the
    /// [`SkillEvent`] indicates an error, we also return the [`SkillExecutionError`]. The semantics
    /// for this is to log the error and stop processing the stream. Technically the
    /// [`SkillExecutionEvent`] would contain the information of the error as well, but due to
    /// errors not being [`Clone`] we just need two instances of it. One for the operator, and one
    /// for the user.
    fn translate_to_execution_event(&mut self, source: SkillEvent) -> SkillExecutionEvent {
        match (source, self.message_active) {
            (SkillEvent::MessageBegin, false) => {
                self.message_active = true;
                SkillExecutionEvent::MessageBegin
            }
            (SkillEvent::MessageBegin, true) => {
                SkillExecutionEvent::Error(SkillExecutionError::MessageBeginWhileMessageActive)
            }
            (SkillEvent::MessageEnd { payload }, true) => {
                self.message_active = false;
                SkillExecutionEvent::MessageEnd { payload }
            }
            (SkillEvent::MessageEnd { .. }, false) => {
                SkillExecutionEvent::Error(SkillExecutionError::MessageEndWithoutMessageBegin)
            }
            (SkillEvent::MessageAppend { text }, true) => {
                SkillExecutionEvent::MessageAppend { text }
            }
            (SkillEvent::MessageAppend { .. }, false) => {
                SkillExecutionEvent::Error(SkillExecutionError::MessageAppendWithoutMessageBegin)
            }
            (SkillEvent::InvalidBytesInPayload { message }, _) => {
                SkillExecutionEvent::Error(SkillExecutionError::InvalidOutput(message.clone()))
            }
            (SkillEvent::Reasoning { text }, true) => SkillExecutionEvent::Reasoning { text },
            (SkillEvent::Reasoning { .. }, false) => {
                SkillExecutionEvent::Error(SkillExecutionError::ReasoningWithoutMessageBegin)
            }
        }
    }
}

/// Errors which may prevent a skill from executing to completion successfully.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SkillExecutionError {
    #[error("The skill could not be loaded. Original error:\n\n{0}")]
    SkillLoadError(#[from] SkillLoadError),
    #[error(
        "The metadata function of the invoked skill is bugged. It is forbidden to invoke any CSI \
        functions from the metadata function, yet the skill does precisely this."
    )]
    CsiUseFromMetadata,
    /// A skill name is not mentioned in the namespace and therefore it is not served. This is a
    /// logic error. Yet it does not originate in the skill code itself. It could be an error in the
    /// request by the user, or a missing configuration at the side of the skill developer.
    #[error(
        "Sorry, we could not find the skill you requested in its namespace. This can have three \
        causes:\n\n\
        1. You sent the wrong skill name.\n\
        2. You sent the wrong namespace.\n\
        3. The skill is not configured in the namespace you requested. You may want to check the \
        namespace configuration."
    )]
    SkillNotConfigured,
    /// Skill Logic errors are logic errors which are reported by the skill code itself. These may
    /// be due to bugs in the skill code, or invalid user input, we will not be able to tell. For
    /// the operator these are both user errors. The skill user and developer are often the same
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
    #[error(
        "The skill inserted a message end into the stream, which has not been preceded by a \
        message begin. This is a bug in the skill. Please contact its developer."
    )]
    MessageEndWithoutMessageBegin,
    #[error(
        "The skill inserted a reasoning message into the stream, which has not been preceded by a \
        message begin. This is a bug in the skill. Please contact its developer."
    )]
    ReasoningWithoutMessageBegin,
    #[error(
        "The skill inserted a message append into the stream, which has not been preceded by a \
        message begin. This is a bug in the skill. Please contact its developer."
    )]
    MessageAppendWithoutMessageBegin,
    #[error(
        "The skill inserted a message begin into while the previous message has not been ended \
        yet. This is a bug in the skill. Please contact its developer."
    )]
    MessageBeginWhileMessageActive,
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
        "The skill could not be executed to completion, something in our runtime is currently \
        unavailable or misconfigured. You should try again later, if the situation persists you may \
        want to contact the operators. Original error:\n\n{0}"
    )]
    RuntimeError(String),
    /// This happens if a configuration for an individual namespace is broken. For the user calling
    /// the route to execute a skill, we treat this as a runtime error, but make sure he gets all
    /// the context, because it very likely might be the skill developer who misconfigured the
    /// namespace. For the operator team, operating all of Pharia Kernel we treat this as a logic
    /// error, because there is nothing wrong about the Kernel installation or inference, or network
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
    #[error(
        "The skill is designed to stream output. Please invoke it via the /message-stream endpoint."
    )]
    IsMessageStream,
}

impl SkillExecutionError {
    /// The severity with which this error is reported to the operator.
    ///
    /// Anything which is wrong with our runtime (i.e. resources, external services, network, etc.)
    /// is only fixable by the operator. Therefore we report any failure due to these as errors.
    /// Anything which is wrong with the skill itself, might not be fixable by the operator, but
    /// still compromises the functionality of the system. Also in some situtations the operator
    /// might be the skill developer, or at least on the same team. Therefore we report these as
    /// warnings.
    /// Anything which can be caused by invalid user input over our HTTP interface, might neither be
    /// in the power of operator or developer to fix. We report these as info, in order to be not to
    /// noisy.
    pub fn tracing_level(&self) -> tracing::Level {
        use tracing::Level;
        match self {
            SkillExecutionError::SkillLoadError(SkillLoadError::NotSupportedYet(..)) |
                    SkillExecutionError::RuntimeError(_) => Level::ERROR,
            SkillExecutionError::CsiUseFromMetadata
                    | SkillExecutionError::SkillLoadError(_)
                    | SkillExecutionError::MessageEndWithoutMessageBegin
                    | SkillExecutionError::ReasoningWithoutMessageBegin
                    | SkillExecutionError::MessageAppendWithoutMessageBegin
                    | SkillExecutionError::MessageBeginWhileMessageActive
                    | SkillExecutionError::InvalidOutput(_)
                    | SkillExecutionError::MisconfiguredNamespace { .. } => Level::WARN,
            SkillExecutionError::SkillNotConfigured
                    | SkillExecutionError::InvalidInput(_)
                    | SkillExecutionError::IsMessageStream
                    | SkillExecutionError::IsFunction
                    // Some of these are bugs, but as long as we do not strictly distinguish those from
                    // invalid input, let's reduce false positives and report them as info.
                    | SkillExecutionError::UserCode(..) => Level::INFO,
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
            SkillError::RuntimeError(error) => Self::RuntimeError(error.to_string()),
            SkillError::IsFunction => Self::IsFunction,
            SkillError::IsMessageStream => Self::IsMessageStream,
        }
    }
}

#[cfg(test)]
mod test {
    use std::{panic, time::Duration};

    use super::*;
    use crate::{
        chunking::ChunkParams,
        csi::CsiError,
        hardcoded_skills::SkillHello,
        inference::{
            ChatParams, CompletionParams, FinishReason, Granularity, Logprobs, TextScore,
            TokenUsage,
        },
        skill::{BoxedCsi, Skill, SkillError},
        tool::{ToolError, ToolOutput},
    };
    use anyhow::{anyhow, bail};
    use double_trait::Dummy;
    use serde_json::json;

    #[tokio::test]
    async fn chunk_result_is_forwarded() {
        // Given a skill invocation context with a stub csi provider
        struct ContextualCsiStub;
        impl ContextualCsi for ContextualCsiStub {
            async fn chunk(&self, _requests: Vec<ChunkRequest>) -> anyhow::Result<Vec<Vec<Chunk>>> {
                Ok(vec![vec![Chunk {
                    text: "my_chunk".to_owned(),
                    byte_offset: 0,
                    character_offset: None,
                }]])
            }
        }

        let (send, _) = mpsc::channel(1);
        let mut invocation_ctx = SkillInvocationCtx::new(send, ContextualCsiStub);

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
        struct ContextualCsiSaboteur;
        impl ContextualCsi for ContextualCsiSaboteur {
            async fn chunk(&self, _requests: Vec<ChunkRequest>) -> anyhow::Result<Vec<Vec<Chunk>>> {
                Err(anyhow!("Failed to load tokenizer"))
            }
        }

        let (send, mut recv) = mpsc::channel(1);
        let mut invocation_ctx = SkillInvocationCtx::new(send, ContextualCsiSaboteur);

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
        let error = select! {
            Some(event) = recv.recv() => match event {
                SkillCtxEvent::Quit(error) => error,
                SkillCtxEvent::ToolBegin { .. } | SkillCtxEvent::ToolEnd { .. } => unreachable!(),
            },
            _ = invocation_ctx.chunk(vec![request])  => unreachable!(),
        };

        // Then receive the error from saboteur tokenizer provider
        assert_eq!(error.to_string(), "Failed to load tokenizer");
    }

    #[tokio::test]
    async fn skill_invocation_ctx_stream_management() {
        struct ContextualCsiStub;
        impl ContextualCsi for ContextualCsiStub {
            async fn completion_stream(
                &self,
                _request: CompletionRequest,
            ) -> mpsc::Receiver<Result<CompletionEvent, InferenceError>> {
                let (sender, receiver) = mpsc::channel(1);
                tokio::spawn(async move {
                    sender
                        .send(Ok(CompletionEvent::Append {
                            text: "text".to_owned(),
                            logprobs: vec![],
                        }))
                        .await
                        .unwrap();
                    sender
                        .send(Ok(CompletionEvent::End {
                            finish_reason: FinishReason::Stop,
                        }))
                        .await
                        .unwrap();
                    sender
                        .send(Ok(CompletionEvent::Usage {
                            usage: TokenUsage {
                                prompt: 2,
                                completion: 2,
                            },
                        }))
                        .await
                        .unwrap();
                });
                receiver
            }
        }
        let (send, _) = mpsc::channel(1);
        let mut ctx = SkillInvocationCtx::new(send, ContextualCsiStub);
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
                CompletionEvent::Append {
                    text: "text".to_owned(),
                    logprobs: vec![]
                },
                CompletionEvent::End {
                    finish_reason: FinishReason::Stop
                },
                CompletionEvent::Usage {
                    usage: TokenUsage {
                        prompt: 2,
                        completion: 2
                    }
                }
            ]
        );
        assert!(ctx.completion_streams.is_empty());
    }

    #[tokio::test]
    async fn skill_invocation_ctx_chat_stream_management() {
        struct ContextualCsiStub;
        impl ContextualCsi for ContextualCsiStub {
            async fn chat_stream(
                &self,
                _request: ChatRequest,
            ) -> mpsc::Receiver<Result<ChatEvent, InferenceError>> {
                let (sender, receiver) = mpsc::channel(1);
                tokio::spawn(async move {
                    sender
                        .send(Ok(ChatEvent::MessageBegin {
                            role: "assistant".to_owned(),
                        }))
                        .await
                        .unwrap();
                    sender
                        .send(Ok(ChatEvent::MessageAppend {
                            content: "Hello".to_owned(),
                            logprobs: vec![],
                        }))
                        .await
                        .unwrap();
                    sender
                        .send(Ok(ChatEvent::MessageEnd {
                            finish_reason: FinishReason::Stop,
                        }))
                        .await
                        .unwrap();
                    sender
                        .send(Ok(ChatEvent::Usage {
                            usage: TokenUsage {
                                prompt: 1,
                                completion: 1,
                            },
                        }))
                        .await
                        .unwrap();
                });
                receiver
            }
        }
        let (send, _) = mpsc::channel(1);
        let mut ctx = SkillInvocationCtx::new(send, ContextualCsiStub);
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
                ChatEvent::MessageBegin {
                    role: "assistant".to_owned(),
                },
                ChatEvent::MessageAppend {
                    content: "Hello".to_owned(),
                    logprobs: vec![],
                },
                ChatEvent::MessageEnd {
                    finish_reason: FinishReason::Stop
                },
                ChatEvent::Usage {
                    usage: TokenUsage {
                        prompt: 1,
                        completion: 1
                    }
                }
            ]
        );
        assert!(ctx.chat_streams.is_empty());
    }

    #[tokio::test]
    async fn csi_usage_from_metadata_leads_to_suspension() {
        // Given a skill runtime that always returns a skill that uses the csi from the metadata
        // function
        struct CsiFromMetadataSkill;

        #[async_trait]
        impl Skill for CsiFromMetadataSkill {
            async fn manifest(
                &self,
                mut ctx: BoxedCsi,
                _tracing_context: &TracingContext,
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
        }

        // When metadata for a skill is requested
        let metadata = SkillDriver
            .metadata(Arc::new(CsiFromMetadataSkill), &TracingContext::dummy())
            .await;

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
                mut ctx: BoxedCsi,
                _input: Value,
                _tracing_context: &TracingContext,
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
        }

        struct ContextualCsiStub;
        impl ContextualCsi for ContextualCsiStub {
            async fn explain(
                &self,
                _requests: Vec<ExplanationRequest>,
            ) -> Result<Vec<Explanation>, CsiError> {
                Ok(vec![Explanation::new(vec![TextScore {
                    score: 0.0,
                    start: 0,
                    length: 2,
                }])])
            }
        }

        let resp = SkillDriver
            .run_function(
                Arc::new(SkillDoubleUsingExplain),
                json!({"prompt": "An apple a day", "target": " keeps the doctor away"}),
                ContextualCsiStub,
                &TracingContext::dummy(),
            )
            .await;

        // drop(runtime);

        assert_eq!(resp.unwrap(), json!([{"start": 0, "length": 2}]));
    }

    #[tokio::test]
    async fn should_forward_csi_errors() {
        struct ContextualCsiSaboteur;
        impl ContextualCsi for ContextualCsiSaboteur {
            async fn complete(
                &self,
                _requests: Vec<CompletionRequest>,
            ) -> anyhow::Result<Vec<Completion>> {
                bail!("Test error")
            }
        }

        // Given a skill using csi and a csi that fails
        // Note we are using a skill which actually invokes the csi
        let skill = Arc::new(SkillGreetCompletion);

        // When trying to generate a greeting for Homer using the greet skill
        let result = SkillDriver
            .run_function(
                skill,
                json!("Homer"),
                ContextualCsiSaboteur,
                &TracingContext::dummy(),
            )
            .await;

        // Then
        let expected_error_msg = "The skill could not be executed to completion, something in our \
            runtime is currently unavailable or misconfigured. You should try again later, if \
            the situation persists you may want to contact the operators. Original error:\n\n\
            Test error";
        assert_eq!(result.unwrap_err().to_string(), expected_error_msg);
    }

    struct MessageStreamSkillWithCsi;

    #[async_trait]
    impl Skill for MessageStreamSkillWithCsi {
        async fn run_as_message_stream(
            &self,
            mut ctx: BoxedCsi,
            _input: Value,
            _sender: mpsc::Sender<SkillEvent>,
            _tracing_context: &TracingContext,
        ) -> Result<(), SkillError> {
            ctx.complete(vec![CompletionRequest {
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
                    echo: false,
                },
            }])
            .await;
            Ok(())
        }
    }

    #[tokio::test]
    async fn csi_errors_cause_message_stream_to_stop_with_error() {
        // Given a skill that calls a failing function on the CSI
        struct CsiSaboteur;

        impl ContextualCsi for CsiSaboteur {
            async fn complete(
                &self,
                _requests: Vec<CompletionRequest>,
            ) -> anyhow::Result<Vec<Completion>> {
                bail!("Out of cheese.")
            }
        }

        let skill = Arc::new(MessageStreamSkillWithCsi);

        // When
        let (send, _) = mpsc::channel(1);
        let result = SkillDriver
            .run_message_stream(
                skill.clone(),
                json!({}),
                CsiSaboteur,
                &TracingContext::dummy(),
                send,
            )
            .await;

        // The result is an error but it doesn't panic
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn message_stream_skill_error_is_forwarded_as_error() {
        // Given a skill that returns a user error when run as message stream
        let skill = Arc::new(SkillGreetCompletion);

        // When running it as a message stream
        let (send, _) = mpsc::channel(1);
        let result = SkillDriver
            .run_message_stream(
                skill.clone(),
                json!({}),
                Dummy,
                &TracingContext::dummy(),
                send,
            )
            .await;

        // Then the result is an error
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn message_stream_skill_completes_successfully() {
        // Given a message stream skill that does not use the CSI
        let skill = Arc::new(SkillHello);

        // When running it
        let (send, _) = mpsc::channel(1);
        let result = SkillDriver
            .run_message_stream(
                skill.clone(),
                json!({}),
                Dummy,
                &TracingContext::dummy(),
                send,
            )
            .await;

        // Then the result is ok
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn should_translate_json_errors_emitted_by_message_stream() {
        // Given a skill that emits a JSON error
        let skill = Arc::new(SkillSaboteurInvalidMessageOutput);

        // When
        let (send, mut recv) = mpsc::channel(1);
        let result = SkillDriver
            .run_message_stream(
                skill.clone(),
                json!({}),
                Dummy,
                &TracingContext::dummy(),
                send,
            )
            .await;

        // Then
        assert_eq!(
            SkillExecutionEvent::Error(SkillExecutionError::InvalidOutput(
                "Test error parsing JSON".to_owned()
            )),
            recv.recv().await.unwrap()
        );

        assert!(matches!(
            result,
            Err(SkillExecutionError::InvalidOutput(..))
        ));
    }

    #[tokio::test]
    async fn should_insert_error_if_message_stream_emits_message_end_without_message_start() {
        // Given a skill that emits a message_end without a message_start
        struct BuggyStreamingSkill;

        #[async_trait]
        impl Skill for BuggyStreamingSkill {
            async fn run_as_message_stream(
                &self,
                _ctx: BoxedCsi,
                _input: Value,
                sender: mpsc::Sender<SkillEvent>,
                _tracing_context: &TracingContext,
            ) -> Result<(), SkillError> {
                sender
                    .send(SkillEvent::MessageEnd {
                        payload: json!(null),
                    })
                    .await
                    .unwrap();
                Ok(())
            }
        }

        // When
        let skill = Arc::new(BuggyStreamingSkill);
        let (send, mut recv) = mpsc::channel(1);
        let result = SkillDriver
            .run_message_stream(
                skill.clone(),
                json!({}),
                Dummy,
                &TracingContext::dummy(),
                send,
            )
            .await;

        // Then
        assert_eq!(
            SkillExecutionEvent::Error(SkillExecutionError::MessageEndWithoutMessageBegin),
            recv.recv().await.unwrap()
        );
        assert!(matches!(
            result,
            Err(SkillExecutionError::MessageEndWithoutMessageBegin)
        ));
    }

    /// A test double for a skill. It invokes the csi with a prompt and returns the result.
    struct SkillGreetCompletion;

    #[async_trait]
    impl Skill for SkillGreetCompletion {
        async fn run_as_function(
            &self,
            mut ctx: BoxedCsi,
            _input: Value,
            _tracing_context: &TracingContext,
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
                        echo: false,
                    },
                }])
                .await;
            let completion = completions.pop().unwrap().text;
            Ok(json!(completion))
        }

        async fn run_as_message_stream(
            &self,
            _ctx: BoxedCsi,
            _input: Value,
            _sender: mpsc::Sender<SkillEvent>,
            _tracing_context: &TracingContext,
        ) -> Result<(), SkillError> {
            Err(SkillError::IsFunction)
        }
    }

    /// Test double emiting a syntax error in the message stream.
    struct SkillSaboteurInvalidMessageOutput;

    #[async_trait]
    impl Skill for SkillSaboteurInvalidMessageOutput {
        async fn run_as_message_stream(
            &self,
            _ctx: BoxedCsi,
            _input: Value,
            sender: mpsc::Sender<SkillEvent>,
            _tracing_context: &TracingContext,
        ) -> Result<(), SkillError> {
            sender
                .send(SkillEvent::InvalidBytesInPayload {
                    message: "Test error parsing JSON".to_owned(),
                })
                .await
                .unwrap();
            Ok(())
        }
    }

    #[tokio::test]
    async fn skill_with_tool_call_event_is_executed_normally() {
        // Given a skill that does a tool call
        struct SkillWithToolCall;

        #[async_trait]
        impl Skill for SkillWithToolCall {
            async fn run_as_function(
                &self,
                mut ctx: BoxedCsi,
                _input: Value,
                _tracing_context: &TracingContext,
            ) -> Result<Value, SkillError> {
                ctx.invoke_tool(vec![InvokeRequest {
                    name: "test-tool".to_owned(),
                    arguments: vec![],
                }])
                .await;
                Ok(json!({}))
            }
        }

        // And given a stub csi
        struct ContextualCsiStub;
        impl ContextualCsi for ContextualCsiStub {
            async fn invoke_tool(
                &self,
                _requests: Vec<InvokeRequest>,
            ) -> Vec<Result<ToolOutput, ToolError>> {
                vec![Ok(ToolOutput::empty())]
            }
        }

        // When
        let skill = Arc::new(SkillWithToolCall);
        let result = SkillDriver
            .run_function(
                skill,
                json!({}),
                ContextualCsiStub,
                &TracingContext::dummy(),
            )
            .await;

        // Then
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn tool_call_event_are_forwarded_for_message_stream() {
        // Given a skill that does a tool call
        struct SkillWithToolCall;

        #[async_trait]
        impl Skill for SkillWithToolCall {
            async fn run_as_message_stream(
                &self,
                mut ctx: BoxedCsi,
                _input: Value,
                _sender: mpsc::Sender<SkillEvent>,
                _tracing_context: &TracingContext,
            ) -> Result<(), SkillError> {
                ctx.invoke_tool(vec![InvokeRequest {
                    name: "test-tool".to_owned(),
                    arguments: vec![],
                }])
                .await;
                // sleep for a short while to give the invoker the chance to receive the tool event
                tokio::time::sleep(Duration::from_millis(10)).await;
                Ok(())
            }
        }

        // And given a stub csi
        struct ContextualCsiStub;
        impl ContextualCsi for ContextualCsiStub {
            async fn invoke_tool(
                &self,
                _requests: Vec<InvokeRequest>,
            ) -> Vec<Result<ToolOutput, ToolError>> {
                vec![Ok(ToolOutput::empty())]
            }
        }

        // When running the skill as message stream
        let (send, mut recv) = mpsc::channel(1);
        let skill = Arc::new(SkillWithToolCall);

        let task = tokio::task::spawn(async move {
            SkillDriver
                .run_message_stream(
                    skill,
                    json!({}),
                    ContextualCsiStub,
                    &TracingContext::dummy(),
                    send,
                )
                .await
        });

        // Then we receive a tool call event
        let mut events = Vec::new();
        while let Some(event) = recv.recv().await {
            events.push(event);
        }
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            SkillExecutionEvent::ToolBegin { name } if name == "test-tool"
        ));
        assert!(matches!(
            &events[1],
            SkillExecutionEvent::ToolEnd { name, result } if name == "test-tool" && result.is_ok()
        ));
        assert!(task.await.is_ok());
    }

    #[tokio::test]
    async fn tool_end_event_is_produced_for_each_tool_call() {
        // Given a stub csi
        struct StubCsi;
        impl ContextualCsi for StubCsi {
            async fn invoke_tool(
                &self,
                requests: Vec<InvokeRequest>,
            ) -> Vec<Result<ToolOutput, ToolError>> {
                requests
                    .iter()
                    .map(|request| match request.name.as_str() {
                        "count-the-fish" => Ok(ToolOutput::empty()),
                        "catch-a-fish" => {
                            Err(ToolError::ToolExecution("No fish caught.".to_owned()))
                        }
                        _ => unreachable!(),
                    })
                    .collect()
            }
        }

        // Given a Skill invocation ctx
        let (send, mut recv) = mpsc::channel(1);
        let mut ctx = SkillInvocationCtx::new(send, StubCsi);

        // When spawning an invoke tool call into a tokio task
        let task = tokio::task::spawn(async move {
            ctx.invoke_tool(vec![
                InvokeRequest {
                    name: "count-the-fish".to_owned(),
                    arguments: vec![],
                },
                InvokeRequest {
                    name: "catch-a-fish".to_owned(),
                    arguments: vec![],
                },
            ])
            .await
        });

        // Then we receive two tool started and two tool ended events
        let mut events = Vec::new();
        while let Some(event) = recv.recv().await {
            events.push(event);
        }
        assert_eq!(events.len(), 4);
        assert!(
            matches!(&events[0], SkillCtxEvent::ToolBegin { name } if name == "count-the-fish")
        );
        assert!(matches!(&events[1], SkillCtxEvent::ToolBegin { name } if name == "catch-a-fish"));
        assert!(
            matches!(&events[2], SkillCtxEvent::ToolEnd { name, result } if name == "count-the-fish" && result.is_ok())
        );
        assert!(
            matches!(&events[3], SkillCtxEvent::ToolEnd { name, result: Err(result) } if name == "catch-a-fish" && result == "No fish caught.")
        );
        assert!(task.await.is_ok());
    }
}
