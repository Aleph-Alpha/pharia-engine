use std::future::pending;

use super::runtime::{Csi, Runtime, WasmRuntime};

use crate::inference::{CompleteTextParameters, InferenceApi};
use anyhow::Error;
use async_trait::async_trait;
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
        Self::with_runtime(WasmRuntime::new(), inference_api)
    }

    /// You may want use this constructor if you want to use a double runtime for testing
    pub fn with_runtime<R: Runtime + Send + 'static>(
        runtime: R,
        inference_api: InferenceApi,
    ) -> Self {
        let (send, recv) = tokio::sync::mpsc::channel::<SkillExecutorMessage>(1);
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

    pub async fn execute_skill(
        &mut self,
        skill: String,
        input: String,
        api_token: String,
    ) -> Result<String, Error> {
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
        let msg = SkillExecutorMessage::Drop { send, skill };

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
                let response = self.runtime.skills().map(str::to_owned).collect();
                drop(send.send(response));
            }
            SkillExecutorMessage::Drop { skill, send } => {
                let response = self.runtime.invalidate_cached_skill(&skill);
                let _ = send.send(response);
            }
        }
    }

    async fn run_skill(
        &mut self,
        skill: String,
        input: String,
        api_token: String,
    ) -> Result<String, Error> {
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
    Execute {
        skill: String,
        input: String,
        send: oneshot::Sender<Result<String, Error>>,
        api_token: String,
    },
    Skills {
        send: oneshot::Sender<Vec<String>>,
    },
    Drop {
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
    send_rt_err: Option<oneshot::Sender<Error>>,
    inference_api: InferenceApi,
    api_token: String,
}

impl SkillInvocationCtx {
    pub fn new(
        send_rt_err: oneshot::Sender<Error>,
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
    async fn complete_text(&mut self, params: CompleteTextParameters) -> String {
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

    use anyhow::{anyhow, Error};

    use crate::{
        inference::{tests::InferenceStub, CompleteTextParameters},
        skills::runtime::tests::{RustRuntime, SaboteurRuntime},
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
                _: String,
                mut ctx: Box<dyn Csi + Send>,
            ) -> Result<String, Error> {
                ctx.complete_text(CompleteTextParameters {
                    prompt: "dummy".to_owned(),
                    model: "dummy".to_owned(),
                    max_tokens: 128,
                })
                .await;
                panic!("complete_text must pend forever in case of error")
            }

            fn skills(&self) -> impl Iterator<Item = &str> {
                iter::once("Greet")
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
                "Dummy input".to_owned(),
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
                String::new(),
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
        let runtime = RustRuntime::new();
        let executor = SkillExecutor::with_runtime(runtime, inference.api());
        let result = executor
            .api()
            .execute_skill(
                "greet".to_owned(),
                String::new(),
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
            _name: String,
            _ctx: Box<dyn Csi + Send>,
        ) -> Result<String, Error> {
            panic!("Liar runtime does not run skills")
        }

        fn skills(&self) -> impl Iterator<Item = &str> {
            self.skills.iter().map(String::as_ref)
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
}
