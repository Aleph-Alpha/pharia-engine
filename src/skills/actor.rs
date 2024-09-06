use std::future::pending;

use super::{
    runtime::{CsiForSkills, Runtime, SkillProviderApi, WasmRuntime},
    SkillPath,
};

use crate::{
    csi::{ChunkRequest, Csi as _, CsiApis},
    inference::{Completion, CompletionRequest},
    language_selection::Language,
};
use async_trait::async_trait;
use serde_json::Value;
use tokio::{
    select,
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

/// Starts and stops the execution of skills as it owns the skill executer actor.
pub struct SkillExecutor {
    send: mpsc::Sender<SkillExecutorMsg>,
    handle: JoinHandle<()>,
}

impl SkillExecutor {
    /// Create a new skill executer with the default web assembly runtime
    pub fn new(csi_apis: CsiApis, skill_provider: SkillProviderApi) -> Self {
        let runtime = WasmRuntime::new(skill_provider);
        Self::with_runtime(runtime, csi_apis)
    }

    /// You may want use this constructor if you want to use a double runtime for testing
    pub fn with_runtime<R: Runtime + Send + 'static>(runtime: R, csi_apis: CsiApis) -> Self {
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

struct SkillExecutorActor<R: Runtime> {
    runtime: R,
    recv: mpsc::Receiver<SkillExecutorMsg>,
    csi_apis: CsiApis,
}

impl<R> SkillExecutorActor<R>
where
    R: Runtime,
{
    fn new(runtime: R, recv: mpsc::Receiver<SkillExecutorMsg>, csi_apis: CsiApis) -> Self {
        SkillExecutorActor {
            runtime,
            recv,
            csi_apis,
        }
    }

    async fn run(&mut self) {
        while let Some(msg) = self.recv.recv().await {
            self.act(msg).await;
        }
    }

    async fn act(&mut self, msg: SkillExecutorMsg) {
        let SkillExecutorMsg {
            skill_path,
            input,
            send,
            api_token,
        } = msg;

        let response = self.run_skill(&skill_path, input, api_token).await;
        let result = send.send(response);
        // Error is expected to happen during shutdown. Ignore result.
        drop(result);
    }

    async fn run_skill(
        &mut self,
        skill_path: &SkillPath,
        input: Value,
        api_token: String,
    ) -> Result<Value, ExecuteSkillError> {
        let (send_rt_err, recv_rt_err) = oneshot::channel();
        let ctx = Box::new(SkillInvocationCtx::new(
            send_rt_err,
            self.csi_apis.clone(),
            api_token,
        ));
        select! {
            result = self.runtime.run(skill_path, input, ctx) => result,
            // An error occurred during skill execution.
            Ok(error) = recv_rt_err => Err(ExecuteSkillError::Other(error))
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

/// Implementation of [`Csi`] provided to skills. It is responsible for forwarding the function
/// calls to csi, to the respective drivers and forwarding runtime errors directly to the actor
/// so the User defined code must not worry about accidential complexity.
pub struct SkillInvocationCtx {
    /// This is used to send any runtime error (as opposed to logic error) back to the actor, so it
    /// can drop the future invoking the skill, and report the error appropriately to user and
    /// operator.
    send_rt_err: Option<oneshot::Sender<anyhow::Error>>,
    csi_apis: CsiApis,
    // How the user authenticates with us
    api_token: String,
}

impl SkillInvocationCtx {
    pub fn new(
        send_rt_err: oneshot::Sender<anyhow::Error>,
        csi_apis: CsiApis,
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

#[async_trait]
impl CsiForSkills for SkillInvocationCtx {
    async fn complete_text(&mut self, params: CompletionRequest) -> Completion {
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
        match self
            .csi_apis
            .complete_all(self.api_token.clone(), requests)
            .await
        {
            Ok(value) => value,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn chunk(&mut self, request: ChunkRequest) -> Vec<String> {
        match self.csi_apis.chunk(self.api_token.clone(), request).await {
            Ok(chunks) => chunks,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn select_language(
        &mut self,
        text: String,
        languages: Vec<Language>,
    ) -> Option<Language> {
        self.csi_apis.select_language(text, languages).await
    }
}

#[cfg(test)]
pub mod tests {
    use std::collections::HashMap;

    use super::*;

    use anyhow::anyhow;
    use serde_json::json;

    use crate::{
        csi::tests::dummy_csi_apis,
        inference::{tests::InferenceStub, CompletionRequest},
        skills::{runtime::tests::SaboteurRuntime, SkillProvider},
        tokenizers::{tests::FakeTokenizers, TokenizersApi, TokenizersMsg},
    };

    #[tokio::test]
    async fn chunk() {
        // Given a skill invocation context with a stub tokenizer provider
        let (send, _) = oneshot::channel();
        let tokenizers = FakeTokenizers::new();
        let csi_apis = CsiApis {
            tokenizers: tokenizers.api(),
            ..dummy_csi_apis()
        };
        let mut invocation_ctx = SkillInvocationCtx::new(send, csi_apis, "dummy token".to_owned());

        // When chunking a short text
        let model = "Pharia-1-LLM-7B-control".to_owned();
        let max_tokens = 10;
        let request = ChunkRequest {
            text: "Greet".to_owned(),
            model,
            max_tokens,
        };
        let chunks = invocation_ctx.chunk(request).await;

        drop(invocation_ctx);
        tokenizers.shutdown().await;

        // Then a single chunk is returned
        assert_eq!(chunks.len(), 1);
    }

    #[tokio::test]
    async fn receive_error_if_chunk_failed() {
        // Given a skill invocation context with a saboteur tokenizer provider
        let (send, recv) = oneshot::channel();
        let (send_tokenizer, mut recv_tokenizer) = mpsc::channel(1);
        let tokenizers = TokenizersApi::new(send_tokenizer);
        tokio::spawn(async move {
            let TokenizersMsg::TokenizerByModel { send, .. } = recv_tokenizer.recv().await.unwrap();
            send.send(Err(anyhow!("Failed to load tokenizer")))
        });
        let csi_apis = CsiApis {
            tokenizers,
            ..dummy_csi_apis()
        };
        let mut invocation_ctx = SkillInvocationCtx::new(send, csi_apis, "dummy token".to_owned());

        // When chunking a short text
        let model = "Pharia-1-LLM-7B-control".to_owned();
        let max_tokens = 10;
        let request = ChunkRequest {
            text: "Greet".to_owned(),
            model,
            max_tokens,
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
        let namespaces = HashMap::new();
        let skill_provider = SkillProvider::new(&namespaces);
        let csi_apis = dummy_csi_apis();
        let executer = SkillExecutor::new(csi_apis, skill_provider.api());
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
        skill_provider.wait_for_shutdown().await;

        // Then result indictaes that the skill is missing
        assert!(matches!(result, Err(ExecuteSkillError::SkillDoesNotExist)));
    }

    #[tokio::test]
    async fn inference_error_during_skill_execution() {
        // Given
        // This mock runtime expects that its skills never complete. The futures invoking them must
        // be dropped
        struct MockRuntime {}

        impl Runtime for MockRuntime {
            async fn run(
                &mut self,
                _: &SkillPath,
                _: Value,
                mut ctx: Box<dyn CsiForSkills + Send>,
            ) -> Result<Value, ExecuteSkillError> {
                ctx.complete_text(CompletionRequest::new(
                    "dummy".to_owned(),
                    "dummy".to_owned(),
                ))
                .await;
                panic!("complete_text must pend forever in case of error")
            }
        }
        let inference_saboteur = InferenceStub::new(|_| Err(anyhow!("Test inference error")));
        let csi_apis = CsiApis {
            inference: inference_saboteur.api(),
            ..dummy_csi_apis()
        };

        // When
        let runtime = MockRuntime {};
        let executer = SkillExecutor::with_runtime(runtime, csi_apis);
        let api = executer.api();
        let another_skill_path = SkillPath::dummy();
        let result = api
            .execute_skill(
                another_skill_path,
                json!("Dummy input"),
                "Dummy api token".to_owned(),
            )
            .await;

        // Then
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert_eq!(error.to_string(), "Test inference error");
    }

    #[tokio::test]
    async fn skill_executor_forwards_runtime_errors() {
        let error_msg = "out-of-cheese".to_owned();
        let inference = InferenceStub::with_completion("Hello".to_owned());
        let runtime = SaboteurRuntime::new(error_msg.clone());
        let csi_apis = CsiApis {
            inference: inference.api(),
            ..dummy_csi_apis()
        };
        let executor = SkillExecutor::with_runtime(runtime, csi_apis);

        let result = executor
            .api()
            .execute_skill(
                SkillPath::dummy(),
                json!(""),
                "TOKEN_NOT_REQUIRED".to_owned(),
            )
            .await;

        assert_eq!(result.unwrap_err().to_string(), error_msg);
    }

    #[tokio::test]
    async fn greeting_skill() {
        // Given
        let inference = InferenceStub::with_completion("Hello".to_owned());
        let csi_apis = CsiApis {
            inference: inference.api(),
            ..dummy_csi_apis()
        };

        // When
        let runtime = RustRuntime::with_greet_skill();
        let executor = SkillExecutor::with_runtime(runtime, csi_apis);
        let result = executor
            .api()
            .execute_skill(
                SkillPath::new("local", "greet"),
                json!(""),
                "TOKEN_NOT_REQUIRED".to_owned(),
            )
            .await;
        executor.wait_for_shutdown().await;
        inference.wait_for_shutdown().await;

        // Then
        assert_eq!(result.unwrap(), "Hello");
    }

    /// Intended as a test double for the production runtime. This implementation features exactly
    /// one hardcoded skill. The skill is called `greet` in the `local` namespace and it uses
    /// `luminous-nextgen-7b` to create a greeting given a provided name as an input.
    pub struct RustRuntime {
        skill_path: SkillPath,
    }

    impl RustRuntime {
        pub fn with_greet_skill() -> Self {
            let skill_path = SkillPath::new("local", "greet");
            Self { skill_path }
        }
    }

    impl Runtime for RustRuntime {
        async fn run(
            &mut self,
            skill_path: &SkillPath,
            input: Value,
            mut ctx: Box<dyn CsiForSkills + Send>,
        ) -> Result<Value, ExecuteSkillError> {
            assert!(
                skill_path == &self.skill_path,
                "RustRuntime only supports {} skill",
                self.skill_path
            );
            let prompt = format!(
                "### Instruction:
                Provide a nice greeting for the person utilizing its given name

                ### Input:
                Name: {input}

                ### Response:"
            );
            let request = CompletionRequest::new(prompt, "luminous-nextgen-7b".to_owned());
            Ok(json!(ctx.complete_text(request).await.text))
        }
    }
}
