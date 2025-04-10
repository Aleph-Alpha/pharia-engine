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
    csi::{ChatStreamId, CompletionStreamId, Csi, CsiForSkills},
    inference::{
        ChatEvent, ChatRequest, ChatResponse, Completion, CompletionEvent, CompletionRequest,
        Explanation, ExplanationRequest, InferenceError,
    },
    language_selection::{Language, SelectLanguageRequest},
    namespace_watcher::Namespace,
    search::{Document, DocumentPath, SearchRequest, SearchResult},
    skills::{AnySkillManifest, Engine, Skill, SkillError, SkillEvent, SkillLoadError},
};

pub struct SkillDriver {
    /// Used to execute skills. We will share the engine with multiple running skills, and skill
    /// provider to convert bytes into executable skills.
    engine: Arc<Engine>,
}

impl SkillDriver {
    pub fn new(engine: Arc<Engine>) -> Self {
        Self { engine }
    }

    pub async fn run_message_stream(
        &self,
        skill: Arc<dyn Skill>,
        input: Value,
        csi: impl Csi + Send + Sync + 'static,
        api_token: String,
        sender: mpsc::Sender<SkillExecutionEvent>,
    ) -> Result<(), SkillExecutionError> {
        let (send_rt_err, mut recv_rt_err) = oneshot::channel();
        let csi_for_skills = Box::new(SkillInvocationCtx::new(send_rt_err, csi, api_token));

        let (send_inner, mut recv_inner) = mpsc::channel(1);

        let mut execute_skill =
            skill.run_as_message_stream(&self.engine, csi_for_skills, input, send_inner);

        let mut translator = EventTranslator::new();

        let result = loop {
            select! {
                // Controls the polling order. We want to ensure that we poll runtime errors first,
                // then messages, and finally the result of the skill handler. It is suitable to
                // check for the runtime errors first, since we would then error out and terminate
                // anyways. We can not rely on it for correctness, but we poll the before the
                // receiver of events before the handler.
                biased;

                Ok(error) = &mut recv_rt_err => {
                    let error = SkillExecutionError::RuntimeError(error.to_string());
                    drop(sender
                        .send(SkillExecutionEvent::Error(error.clone()))
                        .await);
                    break Err(error)
                }
                Some(skill_event) = recv_inner.recv() => {
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
        if let Ok(skill_event) = recv_inner.try_recv() {
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
        csi_apis: impl Csi + Send + Sync + 'static,
        api_token: String,
    ) -> Result<Value, SkillExecutionError> {
        let (send_rt_err, recv_rt_err) = oneshot::channel();
        let csi_for_skills = Box::new(SkillInvocationCtx::new(send_rt_err, csi_apis, api_token));
        select! {
            result = skill.run_as_function(&self.engine, csi_for_skills, input) => result.map_err(Into::into),
            // An error occurred during skill execution.
            Ok(error) = recv_rt_err => Err(SkillExecutionError::RuntimeError(error.to_string()))
        }
    }

    pub async fn metadata(
        &self,
        skill: Arc<dyn Skill>,
    ) -> Result<AnySkillManifest, SkillExecutionError> {
        let (send_rt_err, recv_rt_err) = oneshot::channel();
        let ctx = Box::new(SkillMetadataCtx::new(send_rt_err));
        select! {
            result = skill.manifest(self.engine.as_ref(), ctx) => result.map_err(Into::into),
            // An error occurred during skill execution.
            Ok(_error) = recv_rt_err => Err(SkillExecutionError::CsiUseFromMetadata)
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
    send_rt_error: Option<oneshot::Sender<anyhow::Error>>,
    csi_apis: C,
    // How the user authenticates with us
    api_token: String,
    /// ID counter for stored streams.
    current_stream_id: usize,
    /// Currently running chat streams. We store them here so that we can easier cancel the running
    /// skill if there is an error in the stream. This is much harder to do if we use the normal `ResourceTable`.
    chat_streams: HashMap<ChatStreamId, mpsc::Receiver<Result<ChatEvent, InferenceError>>>,
    /// Currently running completion streams. We store them here so that we can easier cancel the running
    /// skill if there is an error in the stream. This is much harder to do if we use the normal `ResourceTable`.
    completion_streams:
        HashMap<CompletionStreamId, mpsc::Receiver<Result<CompletionEvent, InferenceError>>>,
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
            Err(error) => self.send_error(error.into()).await,
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
            Err(error) => self.send_error(error.into()).await,
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

/// We know that skill metadata will not invoke any csi functions, but still need to provide an
/// implementation of `CsiForSkills` We do not want to panic, as someone could build a component
/// that uses the csi functions inside the metadata function. Therefore, we always send a runtime
/// error which will lead to skill suspension.
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

    async fn chat_stream_next(&mut self, _id: &ChatStreamId) -> Option<ChatEvent> {
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
    /// dropin for classical functions. Might be refined in the future. We anticipate the stop
    /// reason to be very useful for end applications. We also introduce end messages to keep the
    /// door open for multiple messages in a stream.
    MessageEnd { payload: Value },
    /// Append the internal string to the current message
    MessageAppend { text: String },
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
    /// the route to execute a skill, we treat this as a runtime error, but make sure he gets all the
    /// context, because it very likely might be the skill developer who misconfigured the
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
            // We give this the error severity, because this hints to a shortcoming in the
            // envirorment. The operator may need to act.
            SkillExecutionError::SkillLoadError(SkillLoadError::NotSupportedYet(..)) |
            SkillExecutionError::RuntimeError(_) => Level::ERROR,
            // These are bugs in the skill code, or their configuration.
            SkillExecutionError::CsiUseFromMetadata
            | SkillExecutionError::SkillLoadError(_)
            | SkillExecutionError::MessageEndWithoutMessageBegin
            | SkillExecutionError::MessageAppendWithoutMessageBegin
            | SkillExecutionError::MessageBeginWhileMessageActive
            | SkillExecutionError::InvalidOutput(_)
            | SkillExecutionError::MisconfiguredNamespace { .. } => Level::WARN,
            // This could be a wrong configuration, but also just mistyping a skill name. So we log
            // these only as info.
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
    use std::panic;

    use super::*;
    use crate::{
        chunking::ChunkParams,
        csi::tests::{CsiDummy, CsiSaboteur, StubCsi},
        hardcoded_skills::SkillHello,
        inference::{
            ChatParams, CompletionParams, FinishReason, Granularity, Logprobs, Message, TextScore,
            TokenUsage,
        },
        skills::SkillError,
    };
    use anyhow::anyhow;
    use serde_json::json;

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
                CompletionEvent::Append {
                    text: completion.text,
                    logprobs: completion.logprobs
                },
                CompletionEvent::End {
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
                ChatEvent::MessageBegin {
                    role: response.message.role,
                },
                ChatEvent::MessageAppend {
                    content: response.message.content,
                    logprobs: response.logprobs,
                },
                ChatEvent::MessageEnd {
                    finish_reason: response.finish_reason
                },
                ChatEvent::Usage {
                    usage: response.usage
                }
            ]
        );
        assert!(ctx.completion_streams.is_empty());
    }

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

            async fn run_as_message_stream(
                &self,
                _engine: &Engine,
                _ctx: Box<dyn CsiForSkills + Send>,
                _input: Value,
                _sender: mpsc::Sender<SkillEvent>,
            ) -> Result<(), SkillError> {
                unreachable!("This won't be invoked during the test")
            }
        }

        let engine = Arc::new(Engine::new(false, None).unwrap());
        let skill_driver = SkillDriver::new(engine);

        // When metadata for a skill is requested
        let metadata = skill_driver.metadata(Arc::new(CsiFromMetadataSkill)).await;

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

            async fn run_as_message_stream(
                &self,
                _engine: &Engine,
                _ctx: Box<dyn CsiForSkills + Send>,
                _input: Value,
                _sender: mpsc::Sender<SkillEvent>,
            ) -> Result<(), SkillError> {
                panic!("Dummy message stream implementation of SkillDoubleUsingExplain")
            }
        }

        let engine = Arc::new(Engine::new(false, None).unwrap());
        let csi = StubCsi::with_explain(|_| {
            Explanation::new(vec![TextScore {
                score: 0.0,
                start: 0,
                length: 2,
            }])
        });

        let runtime = SkillDriver::new(engine);
        let resp = runtime
            .run_function(
                Arc::new(SkillDoubleUsingExplain),
                json!({"prompt": "An apple a day", "target": " keeps the doctor away"}),
                csi,
                "dummy token".to_owned(),
            )
            .await;

        drop(runtime);

        assert_eq!(resp.unwrap(), json!([{"start": 0, "length": 2}]));
    }

    #[tokio::test]
    async fn should_forward_csi_errors() {
        // Given a skill using csi and a csi that fails
        // Note we are using a skill which actually invokes the csi
        let skill = Arc::new(SkillGreetCompletion);
        let engine = Arc::new(Engine::new(false, None).unwrap());
        let driver = SkillDriver::new(engine);

        // When trying to generate a greeting for Homer using the greet skill
        let result = driver
            .run_function(
                skill,
                json!("Homer"),
                CsiSaboteur,
                "TOKEN_NOT_REQUIRED".to_owned(),
            )
            .await;

        // Then
        let expectet_error_msg = "The skill could not be executed to completion, something in our \
            runtime is currently unavailable or misconfigured. You should try again later, if \
            the situation persists you may want to contact the operators. Original error:\n\n\
            Test error";
        assert_eq!(result.unwrap_err().to_string(), expectet_error_msg);
    }

    struct MessageStreamSkillWithCsi;

    #[async_trait]
    impl Skill for MessageStreamSkillWithCsi {
        async fn run_as_function(
            &self,
            _engine: &Engine,
            _ctx: Box<dyn CsiForSkills + Send>,
            _input: Value,
        ) -> Result<Value, SkillError> {
            Err(SkillError::IsMessageStream)
        }

        async fn manifest(
            &self,
            _engine: &Engine,
            _ctx: Box<dyn CsiForSkills + Send>,
        ) -> Result<AnySkillManifest, SkillError> {
            panic!("Dummy metadata implementation of Skill Greet Completion")
        }

        async fn run_as_message_stream(
            &self,
            _engine: &Engine,
            mut ctx: Box<dyn CsiForSkills + Send>,
            _input: Value,
            _sender: mpsc::Sender<SkillEvent>,
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
                },
            }])
            .await;
            Ok(())
        }
    }

    #[tokio::test]
    async fn should_not_panic_if_receiver_is_dropped() {
        // Given a skill that emits a JSON error
        let engine = Arc::new(Engine::new(false, None).unwrap());
        let driver = SkillDriver::new(engine);

        // When
        let skill = Arc::new(MessageStreamSkillWithCsi);
        let (send, _) = mpsc::channel(1);
        let result = driver
            .run_message_stream(
                skill.clone(),
                json!({}),
                CsiSaboteur,
                "Dummy Token".to_owned(),
                send,
            )
            .await;

        // The result is an error but it doesn't panic
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn should_not_panic_on_skill_error_if_receiver_is_dropped() {
        // Given a skill that emits a JSON error
        let engine = Arc::new(Engine::new(false, None).unwrap());
        let driver = SkillDriver::new(engine);

        // When
        let skill = Arc::new(SkillGreetCompletion);
        let (send, _) = mpsc::channel(1);
        let result = driver
            .run_message_stream(
                skill.clone(),
                json!({}),
                CsiDummy,
                "Dummy Token".to_owned(),
                send,
            )
            .await;

        // The result is an error but it doesn't panic
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn should_not_panic_on_skill_completion_if_receiver_is_dropped() {
        // Given a skill that emits a JSON error
        let engine = Arc::new(Engine::new(false, None).unwrap());
        let driver = SkillDriver::new(engine);

        // When
        let skill = Arc::new(SkillHello);
        let (send, _) = mpsc::channel(1);
        let result = driver
            .run_message_stream(
                skill.clone(),
                json!({}),
                CsiDummy,
                "Dummy Token".to_owned(),
                send,
            )
            .await;

        // The result is ok
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn should_translate_json_errors_emitted_by_message_stream() {
        // Given a skill that emits a JSON error
        let engine = Arc::new(Engine::new(false, None).unwrap());
        let driver = SkillDriver::new(engine);

        // When
        let skill = Arc::new(SkillSaboteurInvalidMessageOutput);
        let (send, mut recv) = mpsc::channel(1);
        let result = driver
            .run_message_stream(
                skill.clone(),
                json!({}),
                CsiDummy,
                "Dummy Token".to_owned(),
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
            async fn run_as_function(
                &self,
                _engine: &Engine,
                _ctx: Box<dyn CsiForSkills + Send>,
                _input: Value,
            ) -> Result<Value, SkillError> {
                panic!("This function should not be called");
            }

            async fn manifest(
                &self,
                _engine: &Engine,
                _ctx: Box<dyn CsiForSkills + Send>,
            ) -> Result<AnySkillManifest, SkillError> {
                panic!("This function should not be called");
            }

            async fn run_as_message_stream(
                &self,
                _engine: &Engine,
                _ctx: Box<dyn CsiForSkills + Send>,
                _input: Value,
                sender: mpsc::Sender<SkillEvent>,
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

        let engine = Arc::new(Engine::new(false, None).unwrap());
        let driver = SkillDriver::new(engine);

        // When
        let skill = Arc::new(BuggyStreamingSkill);
        let (send, mut recv) = mpsc::channel(1);
        let result = driver
            .run_message_stream(
                skill.clone(),
                json!({}),
                CsiDummy,
                "Dummy Token".to_owned(),
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

        async fn run_as_message_stream(
            &self,
            _engine: &Engine,
            _ctx: Box<dyn CsiForSkills + Send>,
            _input: Value,
            _sender: mpsc::Sender<SkillEvent>,
        ) -> Result<(), SkillError> {
            Err(SkillError::IsFunction)
        }
    }

    /// Test double emmiting a syntax error in the message stream.
    struct SkillSaboteurInvalidMessageOutput;

    #[async_trait]
    impl Skill for SkillSaboteurInvalidMessageOutput {
        async fn run_as_function(
            &self,
            _engine: &Engine,
            _ctx: Box<dyn CsiForSkills + Send>,
            _input: Value,
        ) -> Result<Value, SkillError> {
            panic!("Dummy, not invoked in test")
        }

        async fn manifest(
            &self,
            _engine: &Engine,
            _ctx: Box<dyn CsiForSkills + Send>,
        ) -> Result<AnySkillManifest, SkillError> {
            panic!("Dummy, not invoked in test")
        }

        async fn run_as_message_stream(
            &self,
            _engine: &Engine,
            _ctx: Box<dyn CsiForSkills + Send>,
            _input: Value,
            sender: mpsc::Sender<SkillEvent>,
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
}
