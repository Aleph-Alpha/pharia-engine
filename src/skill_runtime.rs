use std::{
    borrow::Cow,
    future::{Future, pending},
    pin::Pin,
    sync::Arc,
    time::Instant,
};

use anyhow::anyhow;
use async_trait::async_trait;
use futures::{StreamExt, stream::FuturesUnordered};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::{
    select,
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use crate::{
    chunking::{Chunk, ChunkRequest},
    csi::{Csi, CsiForSkills},
    inference::{
        ChatRequest, ChatResponse, Completion, CompletionParams, CompletionRequest, Explanation,
        ExplanationRequest,
    },
    language_selection::{Language, SelectLanguageRequest},
    namespace_watcher::Namespace,
    search::{Document, DocumentPath, SearchRequest, SearchResult},
    skill_store::{SkillStoreApi, SkillStoreError},
    skills::{Engine, SkillMetadata, SkillPath},
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

    pub async fn run_function(
        &self,
        skill_path: &SkillPath,
        input: Value,
        ctx: Box<dyn CsiForSkills + Send>,
    ) -> Result<Value, SkillExecutionError> {
        let skill = self.skill_store_api.fetch(skill_path.to_owned()).await?;
        // Unwrap Skill, raise error if it is not existing
        let skill = skill.ok_or(SkillExecutionError::SkillNotConfigured)?;
        skill
            .run(&self.engine, ctx, input)
            .await
            .map_err(SkillExecutionError::SkillLogicError)
    }

    pub async fn metadata(
        &self,
        skill_path: &SkillPath,
        ctx: Box<dyn CsiForSkills + Send>,
    ) -> Result<Option<SkillMetadata>, SkillExecutionError> {
        let skill = self.skill_store_api.fetch(skill_path.to_owned()).await?;
        // Unwrap Skill, raise error if it is not existing
        let skill = skill.ok_or(SkillExecutionError::SkillNotConfigured)?;
        skill
            .metadata(self.engine.as_ref(), ctx)
            .await
            .map_err(SkillExecutionError::SkillLogicError)
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

    async fn run_chat(
        &self,
        skill_path: SkillPath,
        input: Value,
        api_token: String,
    ) -> mpsc::Receiver<ChatEvent>;

    async fn skill_metadata(
        &self,
        skill_path: SkillPath,
    ) -> Result<Option<SkillMetadata>, SkillExecutionError>;
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

    async fn run_chat(
        &self,
        skill_path: SkillPath,
        input: Value,
        api_token: String,
    ) -> mpsc::Receiver<ChatEvent> {
        let (send, recv) = mpsc::channel::<ChatEvent>(1);

        let msg = RunChatMsg {
            skill_path,
            _input: input,
            send,
            api_token,
        };

        self.send(SkillRuntimeMsg::Chat(msg))
            .await
            .expect("all api handlers must be shutdown before actors");
        recv
    }

    async fn skill_metadata(
        &self,
        skill_path: SkillPath,
    ) -> Result<Option<SkillMetadata>, SkillExecutionError> {
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
    SkillLogicError(#[source] anyhow::Error),
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
    Chat(RunChatMsg),
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
            SkillRuntimeMsg::Chat(msg) => {
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
    pub send: oneshot::Sender<Result<Option<SkillMetadata>, SkillExecutionError>>,
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
pub struct RunChatMsg {
    pub skill_path: SkillPath,
    pub _input: Value,
    pub send: mpsc::Sender<ChatEvent>,
    pub api_token: String,
}

impl RunChatMsg {
    async fn act(
        self,
        csi_apis: impl Csi + Send + Sync + 'static,
        _runtime: &WasmRuntime<impl SkillStoreApi>,
    ) {
        let start = Instant::now();
        let RunChatMsg {
            skill_path,
            _input,
            send,
            api_token,
        } = self;

        let name = skill_path.name.clone();

        let (send_rt_err, recv_rt_err) = oneshot::channel();

        let send_copy = send.clone();

        // Hardcoded domain logic-----------------------------
        let skill_logic = async move {
            if name.eq_ignore_ascii_case("saboteur") {
                Err(SkillExecutionError::SkillLogicError(anyhow!(
                    "Skill is a saboteur"
                )))
            } else if name.eq_ignore_ascii_case("hello") {
                for c in "Hello".chars() {
                    send.send(ChatEvent::Append(c.to_string())).await.unwrap();
                }
                Ok(json!(""))
            } else if name.eq_ignore_ascii_case("tell_me_a_joke") {
                let prompt = "<|begin_of_text|><|start_header_id|>user<|end_header_id|>\n\
                            \n\
                        Tell me a joke!<|eot_id|><|start_header_id|>assistant<|end_header_id|>\n\
                        "
                .to_owned();
                let params = CompletionParams {
                    return_special_tokens: false,
                    max_tokens: Some(300),
                    temperature: Some(0.3),
                    top_k: None,
                    top_p: None,
                    stop: vec![],
                    frequency_penalty: None,
                    presence_penalty: None,
                    logprobs: crate::inference::Logprobs::No,
                };
                let request = CompletionRequest {
                    prompt,
                    model: "llama-3.1-8b-instruct".to_owned(),
                    params,
                };
                match csi_apis.complete(api_token, vec![request]).await {
                    Ok(mut completion) => {
                        send.send(ChatEvent::Append(completion.drain(..).next().unwrap().text))
                            .await
                            .unwrap();
                    }
                    Err(err) => {
                        send_rt_err.send(err).unwrap();
                        pending::<()>().await;
                    }
                }
                Ok(json!(""))
            } else {
                Err(SkillExecutionError::SkillNotConfigured)
            }
        };
        // ---------------------------------------------------

        let response = select! {
            result = skill_logic => result,
            // An error occurred during skill execution.
            Ok(error) = recv_rt_err => Err(SkillExecutionError::RuntimeError(error))
        };

        let label = status_label(&response);

        if let Err(error) = response {
            send_copy
                .send(ChatEvent::Error(error.to_string()))
                .await
                .unwrap();
        };

        let latency = start.elapsed().as_secs_f64();
        let labels = [
            ("namespace", Cow::from(skill_path.namespace.to_string())),
            ("name", Cow::from(skill_path.name)),
            ("status", label.into()),
        ];
        metrics::counter!(SkillRuntimeMetrics::SkillExecutionTotal, &labels).increment(1);
        metrics::histogram!(SkillRuntimeMetrics::SkillExecutionDurationSeconds, &labels)
            .record(latency);
    }
}

fn status_label(result: &Result<Value, SkillExecutionError>) -> String {
    match result {
        Ok(_) => "ok",
        Err(
            SkillExecutionError::SkillLogicError(_)
            | SkillExecutionError::CsiUseFromMetadata
            | SkillExecutionError::SkillNotConfigured
            | SkillExecutionError::MisconfiguredNamespace { .. },
        ) => "logic_error",
        Err(SkillExecutionError::RuntimeError(_)) => "runtime_error",
    }
    .to_owned()
}

/// An event emitted by a chat skill
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatEvent {
    /// Append the internal string to the current message
    Append(String),
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

        let latency = start.elapsed().as_secs_f64();
        let labels = [
            ("namespace", Cow::from(skill_path.namespace.to_string())),
            ("name", Cow::from(skill_path.name)),
            ("status", status_label(&response).into()),
        ];
        metrics::counter!(SkillRuntimeMetrics::SkillExecutionTotal, &labels).increment(1);
        metrics::histogram!(SkillRuntimeMetrics::SkillExecutionDurationSeconds, &labels)
            .record(latency);

        // Error is expected to happen during shutdown. Ignore result.
        drop(send.send(response));
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
    csi_apis: C,
    // How the user authenticates with us
    api_token: String,
}

impl<C> SkillInvocationCtx<C> {
    pub fn new(
        send_rt_err: oneshot::Sender<anyhow::Error>,
        csi_apis: C,
        api_token: String,
    ) -> Self {
        SkillInvocationCtx {
            send_rt_err: Some(send_rt_err),
            csi_apis,
            api_token,
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

    async fn chat(&mut self, requests: Vec<ChatRequest>) -> Vec<ChatResponse> {
        match self.csi_apis.chat(self.api_token.clone(), requests).await {
            Ok(value) => value,
            Err(error) => self.send_error(error).await,
        }
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

#[cfg(test)]
pub mod tests {
    use std::time::Duration;

    use anyhow::{anyhow, bail};
    use metrics::Label;
    use metrics_util::debugging::DebugValue;
    use metrics_util::debugging::{DebuggingRecorder, Snapshot};
    use serde_json::json;
    use test_skills::{
        given_csi_from_metadata_skill, given_invalid_output_skill, given_python_skill_greet_v0_3,
        given_rust_skill_explain, given_rust_skill_greet_v0_2, given_rust_skill_greet_v0_3,
    };
    use tokio::try_join;

    use crate::csi::tests::{CsiCompleteStub, CsiGreetingMock};
    use crate::inference::{Explanation, ExplanationRequest, TextScore};
    use crate::namespace_watcher::Namespace;
    use crate::skill_store::tests::SkillStoreDummy;
    use crate::skills::SkillMetadata;
    use crate::{
        chunking::ChunkParams,
        csi::tests::{CsiDummy, StubCsi},
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
            "The metadata function of the invoked skill is bugged. It is forbidden to invoke any \
            CSI functions from the metadata function, yet the skill does precisely this."
        );
    }

    #[tokio::test]
    async fn skill_metadata_v0_2_is_none() {
        // Given a skill runtime that always returns a v0.2 skill
        let test_skill = given_rust_skill_greet_v0_2();
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
        let test_skill = given_rust_skill_greet_v0_3();
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
        let test_skill = given_rust_skill_explain();
        let skill_path = SkillPath::local("explain");
        let engine = Arc::new(Engine::new(false).unwrap());
        let skill_store =
            SkillStoreStub::new(engine.clone(), test_skill.bytes(), skill_path.clone());
        let (send, _) = oneshot::channel();
        let csi = StubCsi::with_explain(|_| {
            Explanation::new(vec![TextScore {
                score: 0.0,
                start: 0,
                length: 2,
            }])
        });
        let skill_ctx = Box::new(SkillInvocationCtx::new(send, csi, "dummy token".to_owned()));

        let runtime = WasmRuntime::new(engine, skill_store.api());
        let resp = runtime
            .run_function(
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
        let test_skill = given_rust_skill_greet_v0_2();
        let skill_path = SkillPath::local("greet");
        let engine = Arc::new(Engine::new(false).unwrap());
        let skill_store =
            SkillStoreStub::new(engine.clone(), test_skill.bytes(), skill_path.clone());

        let runtime = WasmRuntime::new(engine, skill_store.api());
        let skill_ctx = Box::new(CsiCompleteStub::new(|_| Completion::from_text("Hello")));
        let resp = runtime
            .run_function(&skill_path, json!("name"), skill_ctx)
            .await;

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
            .run_function(&SkillPath::dummy(), json!("name"), skill_ctx)
            .await;

        drop(runtime);
        skill_store.wait_for_shutdown().await;

        assert!(resp.is_err());
    }

    #[tokio::test]
    async fn rust_greeting_skill() {
        let test_skill = given_rust_skill_greet_v0_2();
        let skill_path = SkillPath::local("greet_skill_v0_2");
        let engine = Arc::new(Engine::new(false).unwrap());
        let skill_store =
            SkillStoreStub::new(engine.clone(), test_skill.bytes(), skill_path.clone());
        let skill_ctx = Box::new(CsiGreetingMock);

        let runtime = WasmRuntime::new(engine, skill_store.api());
        let actual = runtime
            .run_function(&skill_path, json!("Homer"), skill_ctx)
            .await
            .unwrap();

        drop(runtime);
        skill_store.wait_for_shutdown().await;

        assert_eq!(actual, "Hello Homer");
    }

    #[tokio::test]
    async fn python_greeting_skill() {
        let test_skill = given_python_skill_greet_v0_3();
        let skill_ctx = Box::new(CsiGreetingMock);
        let skill_path = SkillPath::local("greet");
        let engine = Arc::new(Engine::new(false).unwrap());
        let skill_store =
            SkillStoreStub::new(engine.clone(), test_skill.bytes(), skill_path.clone());

        let runtime = WasmRuntime::new(engine, skill_store.api());

        let actual = runtime
            .run_function(&skill_path, json!("Homer"), skill_ctx)
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
        // Given csi which emits errors for completion request
        let test_skill = given_rust_skill_greet_v0_3();
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
            .run_function(
                SkillPath::local("greet"),
                json!("Homer"),
                "TOKEN_NOT_REQUIRED".to_owned(),
            )
            .await;

        runtime.wait_for_shutdown().await;
        store.wait_for_shutdown().await;

        // Then
        let expectet_error_msg = "The skill could not be executed to completion, something in our \
            runtime is currently \nunavailable or misconfigured. You should try again later, if \
            the situation persists you \nmay want to contact the operaters. Original error:\n\n\
            Test error";
        assert_eq!(result.unwrap_err().to_string(), expectet_error_msg);
    }

    #[tokio::test]
    async fn greeting_skill() {
        // Given
        let test_skill = given_rust_skill_greet_v0_3();
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
            .run_function(
                SkillPath::local("greet"),
                json!(""),
                "TOKEN_NOT_REQUIRED".to_owned(),
            )
            .await;
        runtime.wait_for_shutdown().await;
        store.wait_for_shutdown().await;

        // Then
        assert_eq!(result.unwrap(), "Hello");
    }

    #[tokio::test(start_paused = true)]
    async fn concurrent_skill_execution() {
        // Given
        let test_skill = given_rust_skill_greet_v0_3();
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

        // When executing tw tasks in parallel
        let skill_path = SkillPath::local("greet");
        let input = json!("Homer");
        let token = "TOKEN_NOT_REQUIRED";
        let result = try_join!(
            api.run_function(skill_path.clone(), input.clone(), token.to_owned()),
            api.run_function(skill_path, input, token.to_owned()),
        );

        drop(api);
        runtime.wait_for_shutdown().await;
        inference.wait_for_shutdown().await;
        store.wait_for_shutdown().await;

        // Then they both have completed with the same values
        let (result1, result2) = result.unwrap();
        assert_eq!(result1, result2);
    }

    #[tokio::test]
    async fn chat_hello_test() {
        // Given
        let engine = Arc::new(Engine::new(false).unwrap());
        let runtime = SkillRuntime::new(engine, CsiDummy, SkillStoreDummy);

        // When
        let mut recv = runtime
            .api()
            .run_chat(
                SkillPath::local("hello"),
                json!(""),
                "TOKEN_NOT_REQUIRED".to_owned(),
            )
            .await;

        // Then
        assert_eq!(
            recv.recv().await.unwrap(),
            ChatEvent::Append("H".to_string())
        );
        assert_eq!(
            recv.recv().await.unwrap(),
            ChatEvent::Append("e".to_string())
        );
        assert_eq!(
            recv.recv().await.unwrap(),
            ChatEvent::Append("l".to_string())
        );
        assert_eq!(
            recv.recv().await.unwrap(),
            ChatEvent::Append("l".to_string())
        );
        assert_eq!(
            recv.recv().await.unwrap(),
            ChatEvent::Append("o".to_string())
        );
        assert!(recv.recv().await.is_none());

        // Cleanup
        runtime.wait_for_shutdown().await;
    }

    #[tokio::test]
    async fn chat_saboteur_test() {
        // Given
        let engine = Arc::new(Engine::new(false).unwrap());
        let runtime = SkillRuntime::new(engine, CsiDummy, SkillStoreDummy);

        // When
        let mut recv = runtime
            .api()
            .run_chat(
                SkillPath::local("saboteur"),
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
            ChatEvent::Error(expected_error_msg.to_string())
        );
        assert!(recv.recv().await.is_none());

        // Cleanup
        runtime.wait_for_shutdown().await;
    }

    #[test]
    fn skill_runtime_metrics_emitted() {
        let test_skill = given_rust_skill_greet_v0_3();
        let engine = Arc::new(Engine::new(false).unwrap());
        let csi = StubCsi::with_completion_from_text("Hello");
        let (send, _) = oneshot::channel();
        let skill_path = SkillPath::local("greet");
        let msg = RunFunctionMsg {
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

    #[tokio::test]
    async fn chat_skill_should_emit_error_in_case_of_runtime_error_in_csi() {
        // Given
        let engine = Arc::new(Engine::new(false).unwrap());
        let runtime = SkillRuntime::new(engine, SaboteurCsi, SkillStoreDummy);
        let skill_path = SkillPath::local("tell_me_a_joke");

        // When
        let mut recv = runtime
            .api()
            .run_chat(skill_path, json!({}), "dumm_token".to_owned())
            .await;

        // Then
        let event = recv.recv().await.unwrap();
        let expected_error_msg = "The skill could not be executed to completion, something in our \
            runtime is currently \nunavailable or misconfigured. You should try again later, if \
            the situation persists you \nmay want to contact the operaters. Original error:\n\n\
            Test error";
        assert_eq!(event, ChatEvent::Error(expected_error_msg.to_string()));
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

    /// Maybe we can use the `SkillStoreStub` from `SkillStore::test` instead?
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

        pub fn api(&self) -> mpsc::Sender<SkillStoreMessage> {
            self.send.clone()
        }
    }
}
