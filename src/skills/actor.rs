use std::future::pending;

use super::runtime::{Csi, OperatorProvider, Runtime, WasmRuntime};

use crate::{
    configuration_observer::OperatorConfig,
    inference::{Completion, CompletionRequest, InferenceApi},
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
    send: mpsc::Sender<SkillExecutorMessage>,
    handle: JoinHandle<()>,
}

impl SkillExecutor {
    /// Create a new skill executer with the default web assembly runtime
    pub fn new(inference_api: InferenceApi) -> Self {
        let config_str = include_str!("../../config.toml");
        let config = OperatorConfig::from_str(config_str).unwrap();

        let provider = OperatorProvider::new(config);

        let runtime = WasmRuntime::with_provider(provider);
        Self::with_runtime(runtime, inference_api)
    }

    /// You may want use this constructor if you want to use a double runtime for testing
    pub fn with_runtime<R: Runtime + Send + 'static>(
        runtime: R,
        inference_api: InferenceApi,
    ) -> Self {
        let (send, recv) = mpsc::channel::<SkillExecutorMessage>(1);
        let handle = tokio::spawn(async {
            SkillExecutorActor::new(runtime, recv, inference_api)
                .run()
                .await;
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
    send: mpsc::Sender<SkillExecutorMessage>,
}

impl SkillExecutorApi {
    pub fn new(send: mpsc::Sender<SkillExecutorMessage>) -> Self {
        Self { send }
    }

    pub async fn add_skill(&mut self, skill: String) {
        let msg = SkillExecutorMessage::Add { skill };
        self.send
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
    }

    pub async fn remove_skill(&mut self, skill: String) {
        let msg = SkillExecutorMessage::Remove { skill };
        self.send
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
    }

    pub async fn execute_skill(
        &mut self,
        skill: String,
        input: Value,
        api_token: String,
    ) -> anyhow::Result<Value> {
        let (send, recv) = oneshot::channel();
        let msg = SkillExecutorMessage::Execute {
            skill,
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

    pub async fn skills(&mut self) -> Vec<String> {
        let (send, recv) = oneshot::channel();
        let msg = SkillExecutorMessage::Skills { send };

        self.send
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }

    pub async fn drop_from_cache(&mut self, skill: String) -> bool {
        let (send, recv) = oneshot::channel();
        let msg = SkillExecutorMessage::Unload { send, skill };

        self.send
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }
}

struct SkillExecutorActor<R: Runtime> {
    runtime: R,
    recv: mpsc::Receiver<SkillExecutorMessage>,
    inference_api: InferenceApi,
}

impl<R: Runtime> SkillExecutorActor<R> {
    fn new(
        runtime: R,
        recv: mpsc::Receiver<SkillExecutorMessage>,
        inference_api: InferenceApi,
    ) -> Self {
        SkillExecutorActor {
            runtime,
            recv,
            inference_api,
        }
    }

    async fn run(&mut self) {
        while let Some(msg) = self.recv.recv().await {
            self.act(msg).await;
        }
    }

    async fn act(&mut self, msg: SkillExecutorMessage) {
        match msg {
            SkillExecutorMessage::Add { skill } => (),
            SkillExecutorMessage::Remove { skill } => (),
            SkillExecutorMessage::Execute {
                skill,
                input,
                send,
                api_token,
            } => {
                let response = self.run_skill(skill, input, api_token).await;
                drop(send.send(response));
            }
            SkillExecutorMessage::Skills { send } => {
                let response = self.runtime.skills().collect();
                drop(send.send(response));
            }
            SkillExecutorMessage::Unload { skill, send } => {
                let response = self.runtime.invalidate_cached_skill(&skill);
                let _ = send.send(response);
            }
        }
    }

    async fn run_skill(
        &mut self,
        skill: String,
        input: Value,
        api_token: String,
    ) -> anyhow::Result<Value> {
        let (send_rt_err, recv_rt_err) = oneshot::channel();
        let ctx = Box::new(SkillInvocationCtx::new(
            send_rt_err,
            self.inference_api.clone(),
            api_token,
        ));
        select! {
            result = self.runtime.run(&skill, input, ctx) => result,
            Ok(error) = recv_rt_err => Err(error)
        }
    }
}

pub enum SkillExecutorMessage {
    Add {
        skill: String,
    },
    Remove {
        skill: String,
    },
    Execute {
        skill: String,
        input: Value,
        send: oneshot::Sender<anyhow::Result<Value>>,
        api_token: String,
    },
    Skills {
        send: oneshot::Sender<Vec<String>>,
    },
    Unload {
        skill: String,
        send: oneshot::Sender<bool>,
    },
}

/// Implementation of [`Csi`] provided to skills. It is responsible for forwarding the function
/// calls to csi, to the respective drivers and forwarding runtime errors directly to the actor
/// so the User defined code must not worry about accidential complexity.
pub struct SkillInvocationCtx {
    /// This is used to send any runtime error (as opposed to logic error) back to the actor, so it
    /// can drop the future invoking the skill, and report the error appropriately to user and
    /// operator.
    send_rt_err: Option<oneshot::Sender<anyhow::Error>>,
    inference_api: InferenceApi,
    api_token: String,
}

impl SkillInvocationCtx {
    pub fn new(
        send_rt_err: oneshot::Sender<anyhow::Error>,
        inference_api: InferenceApi,
        api_token: String,
    ) -> Self {
        SkillInvocationCtx {
            send_rt_err: Some(send_rt_err),
            inference_api,
            api_token,
        }
    }
}

#[async_trait]
impl Csi for SkillInvocationCtx {
    async fn complete_text(&mut self, params: CompletionRequest) -> Completion {
        match self
            .inference_api
            .complete_text(params, self.api_token.clone())
            .await
        {
            Ok(value) => value,
            Err(error) => {
                self.send_rt_err
                    .take()
                    .expect("Only one error must be send during skill invocation")
                    .send(error)
                    .unwrap();
                // Never return, we did report the error via the send error channel.
                pending().await
            }
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::iter;

    use anyhow::anyhow;
    use serde_json::json;

    use crate::{
        inference::{tests::InferenceStub, CompletionRequest},
        skills::runtime::tests::SaboteurRuntime,
    };

    #[tokio::test]
    async fn inference_error_during_skill_execution() {
        // Given
        // This mock runtime expects that its skills never complete. The futures invoking them must
        // be dropped
        struct MockRuntime;

        impl Runtime for MockRuntime {
            async fn run(
                &mut self,
                _: &str,
                _: Value,
                mut ctx: Box<dyn Csi + Send>,
            ) -> anyhow::Result<Value> {
                ctx.complete_text(CompletionRequest::new(
                    "dummy".to_owned(),
                    "dummy".to_owned(),
                ))
                .await;
                panic!("complete_text must pend forever in case of error")
            }

            fn skills(&self) -> impl Iterator<Item = String> {
                iter::once("Greet".to_owned())
            }
            fn invalidate_cached_skill(&mut self, skill: &str) -> bool {
                skill == "Greet"
            }
        }
        let inference_saboteur = InferenceStub::new(|| Err(anyhow!("Test inference error")));

        // When
        let executer = SkillExecutor::with_runtime(MockRuntime, inference_saboteur.api());
        let mut api = executer.api();
        let result = api
            .execute_skill(
                "Dummy skill name".to_owned(),
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
        let executor = SkillExecutor::with_runtime(runtime, inference.api());

        let result = executor
            .api()
            .execute_skill(
                "greet".to_owned(),
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

        // When
        let executor = SkillExecutor::with_runtime(RustRuntime, inference.api());
        let result = executor
            .api()
            .execute_skill(
                "greet".to_owned(),
                json!(""),
                "TOKEN_NOT_REQUIRED".to_owned(),
            )
            .await;
        executor.wait_for_shutdown().await;
        inference.wait_for_shutdown().await;

        // Then
        assert_eq!(result.unwrap(), "Hello");
    }

    // Tell that `skills` are installed
    pub struct LiarRuntime {
        skills: Vec<String>,
    }

    impl LiarRuntime {
        pub fn new(skills: Vec<String>) -> Self {
            Self { skills }
        }
    }

    impl Runtime for LiarRuntime {
        async fn run(
            &mut self,
            _skill: &str,
            _name: Value,
            _ctx: Box<dyn Csi + Send>,
        ) -> anyhow::Result<Value> {
            panic!("Liar runtime does not run skills")
        }

        fn skills(&self) -> impl Iterator<Item = String> {
            self.skills.iter().map(String::clone)
        }
        fn invalidate_cached_skill(&mut self, skill: &str) -> bool {
            self.skills.iter().any(|s| s == skill)
        }
    }

    #[tokio::test]
    async fn list_skills() {
        // Given a runtime with five skills
        let skills = vec!["First skill".to_owned(), "Second skill".to_owned()];
        let inference = InferenceStub::with_completion("Hello".to_owned());
        let runtime = LiarRuntime::new(skills.clone());

        // When
        let executor = SkillExecutor::with_runtime(runtime, inference.api());
        let result = executor.api().skills().await;

        executor.wait_for_shutdown().await;
        inference.wait_for_shutdown().await;

        // Then
        assert_eq!(result.len(), skills.len());
    }

    #[tokio::test]
    async fn drop_existing_skill() {
        // Given a runtime with the greet skill
        let skills = vec!["haiku_skill".to_owned()];
        let inference = InferenceStub::with_completion("Hello".to_owned());
        let runtime = LiarRuntime::new(skills.clone());

        // When
        let executor = SkillExecutor::with_runtime(runtime, inference.api());
        let result = executor
            .api()
            .drop_from_cache("haiku_skill".to_owned())
            .await;

        executor.wait_for_shutdown().await;
        inference.wait_for_shutdown().await;

        // Then
        assert!(result);
    }

    #[tokio::test]
    async fn drop_non_existing_skill() {
        // Given a runtime with the greet skill
        let skills = vec!["haiku_skill".to_owned()];
        let inference = InferenceStub::with_completion("Hello".to_owned());
        let runtime = LiarRuntime::new(skills.clone());

        // When
        let executor = SkillExecutor::with_runtime(runtime, inference.api());
        let result = executor
            .api()
            .drop_from_cache("a_different_skill".to_owned())
            .await;

        executor.wait_for_shutdown().await;
        inference.wait_for_shutdown().await;

        // Then
        assert!(!result);
    }

    /// Intended as a test double for the production runtime. This implementation features exactly
    /// one hardcoded skill. The skill is called `greet` and it uses `luminous-nextgen-7b` to create
    /// a greeting given a provided name as an input.
    pub struct RustRuntime;

    impl Runtime for RustRuntime {
        async fn run(
            &mut self,
            skill: &str,
            input: Value,
            mut ctx: Box<dyn Csi + Send>,
        ) -> anyhow::Result<Value> {
            assert!(skill == "greet", "RustRuntime only supports greet skill");
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
        fn skills(&self) -> impl Iterator<Item = String> {
            std::iter::once("greet".to_owned())
        }
        fn invalidate_cached_skill(&mut self, skill: &str) -> bool {
            skill == "greet"
        }
    }
}
