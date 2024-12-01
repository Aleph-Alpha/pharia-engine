use std::{
    future::{pending, Future},
    pin::Pin,
    sync::Arc,
};

use async_trait::async_trait;
use futures::{stream::FuturesUnordered, StreamExt};
use opentelemetry::Context;
use serde_json::Value;
use tokio::{
    select,
    sync::{mpsc, oneshot},
    task::JoinHandle,
};
use tracing::{span, Level};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use super::{
    runtime::{CsiForSkills, WasmRuntime},
    Engine, SkillPath,
};

use crate::{
    csi::{chunking::ChunkParams, ChunkRequest, Csi},
    inference::{ChatRequest, ChatResponse, Completion, CompletionRequest},
    language_selection::{Language, SelectLanguageRequest},
    search::{SearchRequest, SearchResult},
    skill_store::SkillStoreApi,
};

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
        let msg = SkillExecutorMsg {
            skill_path,
            input,
            send,
            api_token,
        };
        self.send
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExecuteSkillError {
    #[error(
        "The requested skill does not exist. Make sure it is configured in the configuration \
        associated with the namespace."
    )]
    SkillDoesNotExist,
    #[error(transparent)]
    Other(anyhow::Error),
}

struct SkillExecutorActor<C> {
    runtime: Arc<WasmRuntime>,
    recv: mpsc::Receiver<SkillExecutorMsg>,
    csi_apis: C,
    running_skills: FuturesUnordered<Pin<Box<dyn Future<Output = ()> + Send>>>,
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
            running_skills: FuturesUnordered::new(),
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
                        self.running_skills.push(Box::pin(async move {
                            msg.run_skill(csi_apis, runtime.as_ref()).await;
                        }));
                    },
                    // Senders are gone, break out of the loop for shutdown.
                    None => break
                },
                // FuturesUnordered will let them run in parallel. It will
                // yield once one of them is completed.
                () = self.running_skills.select_next_some(), if !self.running_skills.is_empty()  => {}
            }
        }
    }
}

#[derive(Debug)]
pub struct SkillExecutorMsg {
    pub skill_path: SkillPath,
    pub input: Value,
    pub send: oneshot::Sender<Result<Value, ExecuteSkillError>>,
    pub api_token: String,
}

impl SkillExecutorMsg {
    async fn run_skill(self, csi_apis: impl Csi + Send + Sync + 'static, runtime: &WasmRuntime) {
        let SkillExecutorMsg {
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

#[async_trait]
impl<C> CsiForSkills for SkillInvocationCtx<C>
where
    C: Csi + Send + Sync,
{
    async fn complete_text(&mut self, params: CompletionRequest) -> Completion {
        let span = span!(Level::DEBUG, "complete_text", model = params.model);
        if let Some(context) = self.parent_context.as_ref() {
            span.set_parent(context.clone());
        }
        match self
            .csi_apis
            .complete_text(self.api_token.clone(), params)
            .await
        {
            Ok(value) => value,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn complete_all(&mut self, requests: Vec<CompletionRequest>) -> Vec<Completion> {
        let span = span!(Level::DEBUG, "complete_all", requests_len = requests.len());
        if let Some(context) = self.parent_context.as_ref() {
            span.set_parent(context.clone());
        }
        match self
            .csi_apis
            .complete_all(self.api_token.clone(), requests)
            .await
        {
            Ok(value) => value,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn chat(&mut self, request: ChatRequest) -> ChatResponse {
        let span = span!(Level::DEBUG, "chat", model = request.model);
        if let Some(context) = self.parent_context.as_ref() {
            span.set_parent(context.clone());
        }
        match self.csi_apis.chat(self.api_token.clone(), request).await {
            Ok(value) => value,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn chunk(&mut self, request: ChunkRequest) -> Vec<String> {
        let ChunkParams { model, max_tokens } = &request.params;
        let span = span!(
            Level::DEBUG,
            "chunk",
            text_len = request.text.len(),
            model = model,
            max_tokens = max_tokens,
        );
        if let Some(context) = self.parent_context.as_ref() {
            span.set_parent(context.clone());
        }
        match self.csi_apis.chunk(self.api_token.clone(), request).await {
            Ok(chunks) => chunks,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn select_language(&mut self, request: SelectLanguageRequest) -> Option<Language> {
        let span = span!(
            Level::DEBUG,
            "select_language",
            text_len = request.text.len(),
            languages_len = request.languages.len()
        );
        if let Some(context) = self.parent_context.as_ref() {
            span.set_parent(context.clone());
        }
        match self.csi_apis.select_language(request).await {
            Ok(language) => language,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn search(&mut self, request: SearchRequest) -> Vec<SearchResult> {
        let span = span!(
            Level::DEBUG,
            "search",
            namespace = request.index_path.namespace,
            collection = request.index_path.collection,
            index = request.index_path.index
        );
        if let Some(context) = self.parent_context.as_ref() {
            span.set_parent(context.clone());
        }
        match self.csi_apis.search(self.api_token.clone(), request).await {
            Ok(value) => value,
            Err(error) => self.send_error(error).await,
        }
    }
}

#[cfg(test)]
pub mod tests {
    use std::{fs, time::Duration};

    use super::*;

    use anyhow::{anyhow, bail};
    use serde_json::json;
    use test_skills::given_greet_skill_v0_2;
    use tokio::try_join;

    use crate::{
        csi::tests::{DummyCsi, StubCsi},
        inference::{
            tests::AssertConcurrentClient, ChatRequest, ChatResponse, CompletionRequest, Inference,
        },
        skill_configuration::SkillConfiguration,
        skill_loader::{RegistryConfig, SkillLoader},
        skill_store::{SkillStore, SkillStoreMessage},
        skills::Skill,
    };

    #[tokio::test]
    async fn chunk() {
        // Given a skill invocation context with a stub tokenizer provider
        let (send, _) = oneshot::channel();
        let mut csi = StubCsi::empty();
        csi.set_chunking(|_| Ok(vec!["my_chunk".to_owned()]));

        let mut invocation_ctx = SkillInvocationCtx::new(send, csi, "dummy token".to_owned(), None);

        // When chunking a short text
        let model = "Pharia-1-LLM-7B-control".to_owned();
        let max_tokens = 10;
        let request = ChunkRequest {
            text: "Greet".to_owned(),
            params: ChunkParams { model, max_tokens },
        };
        let chunks = invocation_ctx.chunk(request).await;

        // Then a single chunk is returned
        assert_eq!(chunks, vec!["my_chunk".to_owned()]);
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
            _ = invocation_ctx.chunk(request)  => unreachable!(),
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
        let configuration = SkillConfiguration::new().api();

        let skill_store = SkillStore::new(skill_loader, configuration, Duration::from_secs(10));
        let csi_apis = DummyCsi;
        let executer = SkillExecutor::new(engine, csi_apis, skill_store.api());
        let api = executer.api();

        // When a skill is requested, but it is not listed in the namespace
        let result = api
            .execute_skill(
                SkillPath::new("my_namespace", "my_skill"),
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
                SkillPath::new("test", "greet"),
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
                SkillPath::new("test", "greet"),
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
        let skill_path = SkillPath::new("test", "greet");
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

    #[derive(Clone)]
    struct SaboteurCsi;

    #[async_trait]
    impl Csi for SaboteurCsi {
        async fn complete_text(
            &self,
            _auth: String,
            _request: CompletionRequest,
        ) -> Result<Completion, anyhow::Error> {
            bail!("Test error")
        }

        async fn complete_all(
            &self,
            _auth: String,
            _requests: Vec<CompletionRequest>,
        ) -> Result<Vec<Completion>, anyhow::Error> {
            bail!("Test error")
        }

        async fn chunk(
            &self,
            _auth: String,
            _request: ChunkRequest,
        ) -> Result<Vec<String>, anyhow::Error> {
            bail!("Test error")
        }

        async fn search(
            &self,
            _auth: String,
            _request: SearchRequest,
        ) -> Result<Vec<SearchResult>, anyhow::Error> {
            bail!("Test error")
        }

        async fn chat(
            &self,
            _auth: String,
            _request: ChatRequest,
        ) -> Result<ChatResponse, anyhow::Error> {
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
                            let skill = if skill_path == SkillPath::new("test", "greet") {
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
