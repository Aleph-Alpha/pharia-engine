use std::{collections::HashMap, future::pending};

use super::{
    runtime::{Csi, Runtime, SkillProvider, WasmRuntime},
    SkillPath,
};

use crate::{
    configuration_observer::NamespaceConfig,
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
    pub fn new(inference_api: InferenceApi, namespaces: &HashMap<String, NamespaceConfig>) -> Self {
        let provider = SkillProvider::new(namespaces);
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

    pub async fn upsert_skill(&self, skill: SkillPath, tag: Option<String>) {
        let msg = SkillExecutorMessage::Upsert { skill, tag };
        self.send
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
    }

    pub async fn remove_skill(&self, skill: SkillPath) {
        let msg = SkillExecutorMessage::Remove { skill };
        self.send
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
    }

    pub async fn execute_skill(
        &self,
        skill_path: SkillPath,
        input: Value,
        api_token: String,
    ) -> Result<Value, ExecuteSkillError> {
        let (send, recv) = oneshot::channel();
        let msg = SkillExecutorMessage::Execute {
            skill_path,
            input,
            send,
            api_token,
        };
        self.send
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap().map_err(ExecuteSkillError::Other)
    }

    pub async fn skills(&self) -> Vec<SkillPath> {
        let (send, recv) = oneshot::channel();
        let msg = SkillExecutorMessage::Skills { send };

        self.send
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }

    pub async fn loaded_skills(&self) -> Vec<SkillPath> {
        let (send, recv) = oneshot::channel();
        let msg = SkillExecutorMessage::CachedSkills { send };

        self.send
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }

    pub async fn drop_from_cache(&self, skill_path: SkillPath) -> bool {
        let (send, recv) = oneshot::channel();
        let msg = SkillExecutorMessage::Uncache { send, skill_path };

        self.send
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExecuteSkillError {
    // #[error(
    //     "The requested skill does not exist. Make sure it is configured in the configuration \
    //     associated with the namespace."
    // )]
    // SkillDoesNotExist,
    #[error(transparent)]
    Other(anyhow::Error)
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
            SkillExecutorMessage::Upsert { skill, tag } => self.runtime.upsert_skill(skill, tag),
            SkillExecutorMessage::Remove { skill } => self.runtime.remove_skill(&skill),
            SkillExecutorMessage::Execute {
                skill_path,
                input,
                send,
                api_token,
            } => {
                let response = self.run_skill(&skill_path, input, api_token).await;
                drop(send.send(response));
            }
            SkillExecutorMessage::Skills { send } => {
                let response = self.runtime.skills().cloned().collect();
                drop(send.send(response));
            }
            SkillExecutorMessage::CachedSkills { send } => {
                let response = self.runtime.loaded_skills().cloned().collect();
                drop(send.send(response));
            }
            SkillExecutorMessage::Uncache { skill_path, send } => {
                let response = self.runtime.invalidate_cached_skill(&skill_path);
                let _ = send.send(response);
            }
        }
    }

    async fn run_skill(
        &mut self,
        skill_path: &SkillPath,
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
            result = self.runtime.run(skill_path, input, ctx) => result,
            Ok(error) = recv_rt_err => Err(error)
        }
    }
}

#[derive(Debug)]
pub enum SkillExecutorMessage {
    Upsert {
        skill: SkillPath,
        tag: Option<String>,
    },
    Remove {
        skill: SkillPath,
    },
    Execute {
        skill_path: SkillPath,
        input: Value,
        send: oneshot::Sender<anyhow::Result<Value>>,
        api_token: String,
    },
    Skills {
        send: oneshot::Sender<Vec<SkillPath>>,
    },
    CachedSkills {
        send: oneshot::Sender<Vec<SkillPath>>,
    },
    Uncache {
        skill_path: SkillPath,
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
    use std::{collections::HashSet, iter};

    use anyhow::anyhow;
    use serde_json::json;

    use crate::{
        inference::{tests::InferenceStub, CompletionRequest},
        skills::runtime::tests::SaboteurRuntime,
        OperatorConfig,
    };

    #[tokio::test]
    async fn inference_error_during_skill_execution() {
        // Given
        // This mock runtime expects that its skills never complete. The futures invoking them must
        // be dropped
        struct MockRuntime {
            skill_path: SkillPath,
        }

        impl Runtime for MockRuntime {
            async fn run(
                &mut self,
                _: &SkillPath,
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

            fn upsert_skill(&mut self, _skill: SkillPath, _tag: Option<String>) {
                panic!("does not add new skill")
            }

            fn remove_skill(&mut self, _skill: &SkillPath) {
                panic!("does not remove skill")
            }

            fn skills(&self) -> impl Iterator<Item = &SkillPath> {
                iter::empty()
            }

            fn loaded_skills(&self) -> impl Iterator<Item = &SkillPath> {
                iter::once(&self.skill_path)
            }

            fn invalidate_cached_skill(&mut self, skill_path: &SkillPath) -> bool {
                skill_path == &self.skill_path
            }
        }
        let inference_saboteur = InferenceStub::new(|| Err(anyhow!("Test inference error")));

        // When
        let skill_path = SkillPath::dummy();
        let runtime = MockRuntime { skill_path };
        let executer = SkillExecutor::with_runtime(runtime, inference_saboteur.api());
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
        let executor = SkillExecutor::with_runtime(runtime, inference.api());

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

        // When
        let runtime = RustRuntime::with_greet_skill();
        let executor = SkillExecutor::with_runtime(runtime, inference.api());
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

    // Tell that `skills` are installed
    pub struct LiarRuntime {
        skills: HashSet<SkillPath>,
    }

    impl LiarRuntime {
        pub fn new(skills: &[String]) -> Self {
            Self {
                skills: skills.iter().map(|s| SkillPath::from_str(s)).collect(),
            }
        }
    }

    impl Runtime for LiarRuntime {
        async fn run(
            &mut self,
            _skill_path: &SkillPath,
            _name: Value,
            _ctx: Box<dyn Csi + Send>,
        ) -> anyhow::Result<Value> {
            panic!("Liar runtime does not run skills")
        }

        fn upsert_skill(&mut self, skill: SkillPath, _tag: Option<String>) {
            self.skills.insert(skill);
        }

        fn remove_skill(&mut self, skill: &SkillPath) {
            self.skills.remove(skill);
        }

        fn skills(&self) -> impl Iterator<Item = &SkillPath> {
            self.skills.iter()
        }

        fn loaded_skills(&self) -> impl Iterator<Item = &SkillPath> {
            self.skills.iter()
        }

        fn invalidate_cached_skill(&mut self, skill_path: &SkillPath) -> bool {
            self.skills.iter().any(|s| s == skill_path)
        }
    }

    #[tokio::test]
    async fn list_skills() {
        // Given a runtime with five skills
        let skills = ["First skill".to_owned(), "Second skill".to_owned()];
        let inference = InferenceStub::with_completion("Hello".to_owned());
        let runtime = LiarRuntime::new(&skills);

        // When
        let executor = SkillExecutor::with_runtime(runtime, inference.api());
        let result = executor.api().loaded_skills().await;

        executor.wait_for_shutdown().await;
        inference.wait_for_shutdown().await;

        // Then
        assert_eq!(result.len(), skills.len());
    }

    #[tokio::test]
    async fn drop_existing_skill() {
        // Given a runtime with the greet skill
        let skills = ["haiku_skill".to_owned()];
        let inference = InferenceStub::with_completion("Hello".to_owned());
        let runtime = LiarRuntime::new(&skills);

        // When
        let executor = SkillExecutor::with_runtime(runtime, inference.api());
        let result = executor
            .api()
            .drop_from_cache(SkillPath::from_str("haiku_skill"))
            .await;

        executor.wait_for_shutdown().await;
        inference.wait_for_shutdown().await;

        // Then
        assert!(result);
    }

    #[tokio::test]
    async fn drop_non_existing_skill() {
        // Given a runtime with the greet skill
        let skills = ["haiku_skill".to_owned()];
        let inference = InferenceStub::with_completion("Hello".to_owned());
        let runtime = LiarRuntime::new(&skills);

        // When
        let executor = SkillExecutor::with_runtime(runtime, inference.api());
        let result = executor
            .api()
            .drop_from_cache(SkillPath::from_str("a_different_skill"))
            .await;

        executor.wait_for_shutdown().await;
        inference.wait_for_shutdown().await;

        // Then
        assert!(!result);
    }

    impl SkillExecutor {
        pub fn with_wasm_runtime() -> Self {
            let namespaces = OperatorConfig::empty().namespaces;
            let provider = SkillProvider::new(&namespaces);
            let runtime = WasmRuntime::with_provider(provider);
            let inference = InferenceStub::with_completion("Hello".to_owned());
            SkillExecutor::with_runtime(runtime, inference.api())
        }
    }
    #[tokio::test]
    async fn executor_api_add_skills() {
        // Given a skill executor api
        let api = SkillExecutor::with_wasm_runtime().api();

        // When adding a skill
        let skill_path_1 = SkillPath::dummy();
        api.upsert_skill(skill_path_1.clone(), None).await;
        let skill_path_2 = SkillPath::dummy();
        api.upsert_skill(skill_path_2.clone(), None).await;

        // Then the skills is listed by the skill executor api
        let skills = api.skills().await;
        assert_eq!(skills.len(), 2);
        assert!(skills.contains(&skill_path_1));
        assert!(skills.contains(&skill_path_2));
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
            mut ctx: Box<dyn Csi + Send>,
        ) -> anyhow::Result<Value> {
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

        fn upsert_skill(&mut self, _skill: SkillPath, _tag: Option<String>) {
            panic!("RustRuntime does not add skill")
        }

        fn remove_skill(&mut self, _skill: &SkillPath) {
            panic!("RustRuntime does not remove skill")
        }

        fn skills(&self) -> impl Iterator<Item = &SkillPath> {
            std::iter::empty()
        }

        fn loaded_skills(&self) -> impl Iterator<Item = &SkillPath> {
            std::iter::once(&self.skill_path)
        }

        fn invalidate_cached_skill(&mut self, skill_path: &SkillPath) -> bool {
            skill_path == &self.skill_path
        }
    }
}
