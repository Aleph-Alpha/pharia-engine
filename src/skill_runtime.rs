use std::{
    borrow::Cow,
    future::{pending, Future},
    pin::Pin,
    sync::Arc,
    time::Instant,
};

use anyhow::anyhow;
use async_trait::async_trait;
use futures::{stream::FuturesUnordered, StreamExt};
use opentelemetry::Context;
use serde::Serialize;
use serde_json::Value;
use tokio::{
    select,
    sync::{mpsc, oneshot},
    task::JoinHandle,
};
use tracing::{span, Level};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use utoipa::ToSchema;

use crate::{
    csi::{ChunkRequest, Csi, CsiForSkills},
    inference::{ChatRequest, ChatResponse, Completion, CompletionRequest},
    language_selection::{Language, SelectLanguageRequest},
    search::{Document, DocumentPath, SearchRequest, SearchResult},
    skill_store::SkillStoreApi,
    skills::{Engine, SkillPath},
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
    ) -> Result<Value, ExecuteSkillError> {
        let skill = self.skill_store_api.fetch(skill_path.to_owned()).await?;
        // Unwrap Skill, raise error if it is not existing
        let skill = skill.ok_or(ExecuteSkillError::SkillDoesNotExist)?;
        Ok(skill.run(&self.engine, ctx, input).await?)
    }

    pub async fn metadata(
        &self,
        skill_path: &SkillPath,
        ctx: Box<dyn CsiForSkills + Send>,
    ) -> Result<Option<SkillMetadata>, ExecuteSkillError> {
        let skill = self.skill_store_api.fetch(skill_path.to_owned()).await?;
        // Unwrap Skill, raise error if it is not existing
        let skill = skill.ok_or(ExecuteSkillError::SkillDoesNotExist)?;
        Ok(skill
            .metadata(self.engine.as_ref(), ctx)
            .await
            .map_err(ExecuteSkillError::Other)?)
    }
}

/// Starts and stops the execution of skills as it owns the skill executer actor.
pub struct SkillExecutor {
    send: mpsc::Sender<SkillExecutorMsg>,
    handle: JoinHandle<()>,
}

impl SkillExecutor {
    /// Create a new skill executer with the default web assembly runtime
    pub fn new<C>(engine: Arc<Engine>, csi_apis: C, skill_store_api: SkillStoreApi) -> Self
    where
        C: Csi + Clone + Send + Sync + 'static,
    {
        let runtime = WasmRuntime::new(engine, skill_store_api);
        let (send, recv) = mpsc::channel::<SkillExecutorMsg>(1);
        let handle = tokio::spawn(async {
            SkillExecutorActor::new(runtime, recv, csi_apis).run().await;
        });
        SkillExecutor { send, handle }
    }

    /// Retrieve a handle in order to interact with skills. All handles have to be dropped in order
    /// for [`Self::wait_for_shutdown`] to complete.
    pub fn api(&self) -> SkillExecutorApi {
        SkillExecutorApi::new(self.send.clone())
    }

    pub async fn wait_for_shutdown(self) {
        drop(self.send);
        self.handle.await.unwrap();
    }
}

#[derive(Clone)]
pub struct SkillExecutorApi {
    send: mpsc::Sender<SkillExecutorMsg>,
}

impl SkillExecutorApi {
    pub fn new(send: mpsc::Sender<SkillExecutorMsg>) -> Self {
        Self { send }
    }

    pub async fn execute_skill(
        &self,
        skill_path: SkillPath,
        input: Value,
        api_token: String,
    ) -> Result<Value, ExecuteSkillError> {
        let (send, recv) = oneshot::channel();
        let msg = SkillExecutorMsg::ExecuteSkill(ExecuteSkill {
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
    ) -> Result<Option<SkillMetadata>, ExecuteSkillError> {
        let (send, recv) = oneshot::channel();
        let msg = SkillExecutorMsg::SkillMetadata(SkillMetadataRequest { skill_path, send });
        self.send
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }
}

#[derive(ToSchema, Serialize, Debug)]
#[serde(tag = "version")]
pub enum SkillMetadata {
    #[serde(rename = "v1")]
    V1(SkillMetadataV1),
}

#[derive(ToSchema, Serialize, Debug)]
pub struct SkillMetadataV1 {
    pub description: Option<String>,
    pub input_schema: Value,
    pub output_schema: Value,
}

#[derive(Debug, thiserror::Error)]
pub enum ExecuteSkillError {
    #[error(
        "The requested skill does not exist. Make sure it is configured in the configuration \
        associated with the namespace."
    )]
    SkillDoesNotExist,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

struct SkillExecutorActor<C> {
    runtime: Arc<WasmRuntime>,
    recv: mpsc::Receiver<SkillExecutorMsg>,
    csi_apis: C,
    // Can be a skill execution or a skill metadata request
    running_requests: FuturesUnordered<Pin<Box<dyn Future<Output = ()> + Send>>>,
}

impl<C> SkillExecutorActor<C>
where
    C: Csi + Clone + Send + Sync + 'static,
{
    fn new(runtime: WasmRuntime, recv: mpsc::Receiver<SkillExecutorMsg>, csi_apis: C) -> Self {
        SkillExecutorActor {
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

pub enum SkillExecutorMsg {
    ExecuteSkill(ExecuteSkill),
    SkillMetadata(SkillMetadataRequest),
}

impl SkillExecutorMsg {
    async fn act(self, csi_apis: impl Csi + Send + Sync + 'static, runtime: &WasmRuntime) {
        match self {
            SkillExecutorMsg::ExecuteSkill(execute_skill) => {
                execute_skill.act(csi_apis, runtime).await;
            }
            SkillExecutorMsg::SkillMetadata(skill_metadata_request) => {
                skill_metadata_request.act(runtime).await;
            }
        }
    }
}
pub struct SkillMetadataRequest {
    pub skill_path: SkillPath,
    pub send: oneshot::Sender<Result<Option<SkillMetadata>, ExecuteSkillError>>,
}

impl SkillMetadataRequest {
    pub async fn act(self, runtime: &WasmRuntime) {
        let (send_rt_err, recv_rt_err) = oneshot::channel();
        let ctx = Box::new(SkillMetadataCtx::new(send_rt_err));
        let response = select! {
            result = runtime.metadata(&self.skill_path, ctx) => result,
            // An error occurred during skill execution.
            Ok(error) = recv_rt_err => Err(ExecuteSkillError::Other(error))
        };
        drop(self.send.send(response));
    }
}

#[derive(Debug)]
pub struct ExecuteSkill {
    pub skill_path: SkillPath,
    pub input: Value,
    pub send: oneshot::Sender<Result<Value, ExecuteSkillError>>,
    pub api_token: String,
}

impl ExecuteSkill {
    async fn act(self, csi_apis: impl Csi + Send + Sync + 'static, runtime: &WasmRuntime) {
        let start = Instant::now();
        let ExecuteSkill {
            skill_path,
            input,
            send,
            api_token,
        } = self;

        let span = span!(
            Level::DEBUG,
            "execute_skill",
            skill_path = skill_path.to_string(),
        );
        let context = span.context();
        let (send_rt_err, recv_rt_err) = oneshot::channel();
        let ctx = Box::new(SkillInvocationCtx::new(
            send_rt_err,
            csi_apis,
            api_token,
            Some(context),
        ));
        let response = select! {
            result = runtime.run(&skill_path, input, ctx) => result,
            // An error occurred during skill execution.
            Ok(error) = recv_rt_err => Err(ExecuteSkillError::Other(error))
        };

        let latency = start.elapsed().as_secs_f64();
        let labels = [
            ("namespace", Cow::from(skill_path.namespace.to_string())),
            ("name", Cow::from(skill_path.name)),
            (
                "status",
                match response {
                    Ok(_) => "ok",
                    Err(ExecuteSkillError::SkillDoesNotExist) => "not_found",
                    Err(ExecuteSkillError::Other(_)) => "internal_error",
                }
                .into(),
            ),
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
    // For tracing, we wire the skill invocation span context with the CSI spans
    parent_context: Option<Context>,
}

impl<C> SkillInvocationCtx<C> {
    pub fn new(
        send_rt_err: oneshot::Sender<anyhow::Error>,
        csi_apis: C,
        api_token: String,
        parent_context: Option<Context>,
    ) -> Self {
        SkillInvocationCtx {
            send_rt_err: Some(send_rt_err),
            csi_apis,
            api_token,
            parent_context,
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

/// We know that skill metadata will not invoke any csi functions, but still need to provide an implementation of CsiForSkills
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
            .send(anyhow!("Not implemented"))
            .unwrap();
        pending().await
    }
}

#[async_trait]
impl CsiForSkills for SkillMetadataCtx {
    async fn complete(&mut self, _requests: Vec<CompletionRequest>) -> Vec<Completion> {
        self.send_error().await
    }

    async fn chunk(&mut self, _requests: Vec<ChunkRequest>) -> Vec<Vec<String>> {
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

    async fn chunk(&mut self, requests: Vec<ChunkRequest>) -> Vec<Vec<String>> {
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
    use std::{fs, time::Duration};

    use anyhow::{anyhow, bail};
    use metrics::Label;
    use metrics_util::debugging::DebugValue;
    use metrics_util::debugging::{DebuggingRecorder, Snapshot};
    use serde_json::json;
    use test_skills::{given_greet_py_v0_2, given_greet_skill_v0_2};
    use tokio::try_join;

    use crate::csi::tests::{CsiCompleteStub, CsiCounter, CsiGreetingMock};
    use crate::namespace_watcher::Namespace;
    use crate::skill_loader::ConfiguredSkill;
    use crate::{
        csi::{
            chunking::ChunkParams,
            tests::{DummyCsi, StubCsi},
        },
        inference::{
            tests::AssertConcurrentClient, ChatRequest, ChatResponse, CompletionRequest, Inference,
        },
        search::DocumentPath,
        skill_loader::{RegistryConfig, SkillLoader},
        skill_store::{SkillStore, SkillStoreMessage},
        skills::Skill,
    };

    use super::*;

    #[tokio::test]
    async fn greet_skill_component() {
        given_greet_skill_v0_2();
        let skill_path = SkillPath::local("greet_skill_v0_2");
        let skill = ConfiguredSkill::from_path(&skill_path);
        let engine = Arc::new(Engine::new(false).unwrap());
        let skill_loader =
            SkillLoader::with_file_registry(engine.clone(), skill_path.namespace.clone()).api();

        let skill_store = SkillStore::new(skill_loader, Duration::from_secs(10));
        skill_store.api().upsert(skill).await;
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
        given_greet_skill_v0_2();
        let skill_ctx = Box::new(CsiGreetingMock);
        let skill_path = SkillPath::local("greet_skill_v0_2");
        let skill = ConfiguredSkill::from_path(&skill_path);
        let engine = Arc::new(Engine::new(false).unwrap());
        let skill_loader =
            SkillLoader::with_file_registry(engine.clone(), skill_path.namespace.clone()).api();
        let skill_store = SkillStore::new(skill_loader, Duration::from_secs(10));
        skill_store.api().upsert(skill).await;
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
        given_greet_py_v0_2();
        let skill_ctx = Box::new(CsiGreetingMock);
        let skill_path = SkillPath::local("greet-py-v0_2");
        let skill = ConfiguredSkill::from_path(&skill_path);
        let engine = Arc::new(Engine::new(false).unwrap());
        let skill_loader =
            SkillLoader::with_file_registry(engine.clone(), skill_path.namespace.clone()).api();
        let skill_store = SkillStore::new(skill_loader, Duration::from_secs(10));
        skill_store.api().upsert(skill).await;
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
    async fn can_call_pre_instantiated_multiple_times() {
        given_greet_skill_v0_2();
        let skill_ctx = Box::new(CsiCounter::new());
        let skill_path = SkillPath::local("greet_skill_v0_2");
        let skill = ConfiguredSkill::from_path(&skill_path);
        let engine = Arc::new(Engine::new(false).unwrap());
        let skill_loader =
            SkillLoader::with_file_registry(engine.clone(), skill_path.namespace.clone()).api();
        let skill_store = SkillStore::new(skill_loader, Duration::from_secs(10));
        skill_store.api().upsert(skill).await;
        let runtime = WasmRuntime::new(engine, skill_store.api());
        for i in 1..10 {
            let resp = runtime
                .run(&skill_path, json!("Homer"), skill_ctx.clone())
                .await
                .unwrap();
            assert_eq!(resp, json!(i.to_string()));
        }

        drop(runtime);
        skill_store.wait_for_shutdown().await;
    }

    #[tokio::test]
    async fn chunk() {
        // Given a skill invocation context with a stub tokenizer provider
        let (send, _) = oneshot::channel();
        let mut csi = StubCsi::empty();
        csi.set_chunking(|r| Ok(r.into_iter().map(|_| vec!["my_chunk".to_owned()]).collect()));

        let mut invocation_ctx = SkillInvocationCtx::new(send, csi, "dummy token".to_owned(), None);

        // When chunking a short text
        let model = "Pharia-1-LLM-7B-control".to_owned();
        let max_tokens = 10;
        let request = ChunkRequest {
            text: "Greet".to_owned(),
            params: ChunkParams { model, max_tokens },
        };
        let chunks = invocation_ctx.chunk(vec![request]).await;

        // Then a single chunk is returned
        assert_eq!(chunks[0], vec!["my_chunk".to_owned()]);
    }

    #[tokio::test]
    async fn receive_error_if_chunk_failed() {
        // Given a skill invocation context with a saboteur tokenizer provider
        let (send, recv) = oneshot::channel();
        let mut csi = StubCsi::empty();
        csi.set_chunking(|_| Err(anyhow!("Failed to load tokenizer")));
        let mut invocation_ctx = SkillInvocationCtx::new(send, csi, "dummy token".to_owned(), None);

        // When chunking a short text
        let model = "Pharia-1-LLM-7B-control".to_owned();
        let max_tokens = 10;
        let request = ChunkRequest {
            text: "Greet".to_owned(),
            params: ChunkParams { model, max_tokens },
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
        let executer = SkillExecutor::new(engine, csi_apis, skill_store.api());
        let api = executer.api();

        // When a skill is requested, but it is not listed in the namespace
        let result = api
            .execute_skill(
                SkillPath::local("my_skill"),
                json!("Any input"),
                "Dummy api token".to_owned(),
            )
            .await;

        drop(api);
        executer.wait_for_shutdown().await;
        skill_store.wait_for_shutdown().await;

        // Then result indicates that the skill is missing
        assert!(matches!(result, Err(ExecuteSkillError::SkillDoesNotExist)));
    }

    #[tokio::test]
    async fn skill_executor_forwards_csi_errors() {
        // Given csi which emits errors for completion request
        let engine = Arc::new(Engine::new(false).unwrap());
        let store = SkillStoreGreetStub::new(engine.clone());
        let executor = SkillExecutor::new(engine, SaboteurCsi, store.api());

        // When trying to generate a greeting for Homer using the greet skill
        let result = executor
            .api()
            .execute_skill(
                SkillPath::local("greet"),
                json!("Homer"),
                "TOKEN_NOT_REQUIRED".to_owned(),
            )
            .await;

        executor.wait_for_shutdown().await;
        store.wait_for_shutdown().await;

        // Then
        assert_eq!(result.unwrap_err().to_string(), "Test error");
    }

    #[tokio::test]
    async fn greeting_skill() {
        // Given
        let csi = StubCsi::with_completion_from_text("Hello");
        let engine = Arc::new(Engine::new(false).unwrap());
        let store = SkillStoreGreetStub::new(engine.clone());

        // When
        let executor = SkillExecutor::new(engine, csi, store.api());
        let result = executor
            .api()
            .execute_skill(
                SkillPath::local("greet"),
                json!(""),
                "TOKEN_NOT_REQUIRED".to_owned(),
            )
            .await;
        executor.wait_for_shutdown().await;
        store.wait_for_shutdown().await;

        // Then
        assert_eq!(result.unwrap(), "Hello");
    }

    #[tokio::test(start_paused = true)]
    async fn concurrent_skill_execution() {
        // Given
        let engine = Arc::new(Engine::new(false).unwrap());
        let client = AssertConcurrentClient::new(2);
        let inference = Inference::with_client(client);
        let csi = StubCsi::with_completion_from_text("Hello, Homer!");
        let store = SkillStoreGreetStub::new(engine.clone());
        let executor = SkillExecutor::new(engine, csi, store.api());
        let api = executor.api();

        // When executing tw tasks in parallel
        let skill_path = SkillPath::local("greet");
        let input = json!("Homer");
        let token = "TOKEN_NOT_REQUIRED";
        let result = try_join!(
            api.execute_skill(skill_path.clone(), input.clone(), token.to_owned()),
            api.execute_skill(skill_path, input, token.to_owned()),
        );

        drop(api);
        executor.wait_for_shutdown().await;
        inference.wait_for_shutdown().await;
        store.wait_for_shutdown().await;

        // Then they both have completed with the same values
        let (result1, result2) = result.unwrap();
        assert_eq!(result1, result2);
    }

    #[test]
    fn skill_runtime_metrics_emitted() {
        let engine = Arc::new(Engine::new(false).unwrap());
        let csi = StubCsi::with_completion_from_text("Hello");
        let (send, _) = oneshot::channel();
        let skill_path = SkillPath::local("greet");
        let msg = ExecuteSkill {
            skill_path: skill_path.clone(),
            input: json!("Hello"),
            send,
            api_token: "dummy".to_owned(),
        };
        // Metrics requires sync, so all of the async parts are moved into this closure.
        let snapshot = metrics_snapshot(|| async move {
            let store = SkillStoreGreetStub::new(engine.clone());
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
        ) -> anyhow::Result<Vec<Vec<String>>> {
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
    /// Only serves test/greet skill
    pub struct SkillStoreGreetStub {
        send: mpsc::Sender<SkillStoreMessage>,
        join_handle: JoinHandle<()>,
    }

    impl SkillStoreGreetStub {
        pub fn new(engine: Arc<Engine>) -> Self {
            given_greet_skill_v0_2();
            let greet_bytes = fs::read("./skills/greet_skill_v0_2.wasm").unwrap();
            let skill = Skill::new(&engine, greet_bytes.clone()).unwrap();
            let skill = Arc::new(skill);

            let (send, mut recv) = mpsc::channel::<SkillStoreMessage>(1);
            let join_handle = tokio::spawn(async move {
                while let Some(msg) = recv.recv().await {
                    match msg {
                        SkillStoreMessage::Fetch { skill_path, send } => {
                            let skill = if skill_path == SkillPath::local("greet") {
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
