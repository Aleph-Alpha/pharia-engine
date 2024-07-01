use crate::{inference::InferenceApi, skills::runtime::Runtime};
use anyhow::Error;
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use super::csi::SkillInvocationCtx;

pub struct SkillExecutor {
    send: mpsc::Sender<SkillExecutorMessage>,
    handle: JoinHandle<()>,
}

impl SkillExecutor {
    pub fn new<R: Runtime + Send + 'static>(runtime: R, inference_api: InferenceApi) -> Self {
        let (send, recv) = tokio::sync::mpsc::channel::<SkillExecutorMessage>(1);
        let handle = tokio::spawn(async {
            SkillExecutorActor::new(runtime, recv, inference_api)
                .run()
                .await;
        });
        SkillExecutor { send, handle }
    }
    pub fn api(&self) -> SkillExecutorApi {
        SkillExecutorApi {
            send: self.send.clone(),
        }
    }
    pub async fn shutdown(self) {
        drop(self.send);
        self.handle.await.unwrap();
    }
}

#[derive(Clone)]
pub struct SkillExecutorApi {
    send: mpsc::Sender<SkillExecutorMessage>,
}

impl SkillExecutorApi {
    pub async fn execute_skill(
        &mut self,
        skill: String,
        input: String,
        api_token: String,
    ) -> Result<String, Error> {
        let (send, recv) = oneshot::channel();
        let msg = SkillExecutorMessage {
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
        let ctx = SkillInvocationCtx::new(self.inference_api.clone(), msg.api_token);
        let response = self
            .runtime
            .run(
                &msg.skill,
                msg.input,
                ctx,
            )
            .await;
        drop(msg.send.send(response));
    }
}

struct SkillExecutorMessage {
    skill: String,
    input: String,
    send: oneshot::Sender<Result<String, Error>>,
    api_token: String,
}

#[cfg(test)]
mod tests {
    use crate::{
        inference::tests::InferenceStub,
        skills::{
            tests::{RustRuntime, SaboteurRuntime},
            SkillExecutor,
        },
    };

    #[tokio::test]
    async fn skill_executor_forwards_runtime_errors() {
        let error_msg = "out-of-cheese".to_owned();
        let inference = InferenceStub::with_completion("Hello".to_owned());
        let runtime = SaboteurRuntime::new(error_msg.clone());
        let executor = SkillExecutor::new(runtime, inference.api());

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
        let executor = SkillExecutor::new(runtime, inference.api());
        let result = executor
            .api()
            .execute_skill(
                "greet".to_owned(),
                String::new(),
                "TOKEN_NOT_REQUIRED".to_owned(),
            )
            .await;
        executor.shutdown().await;
        inference.shutdown().await;

        // Then
        assert_eq!(result.unwrap(), "Hello");
    }
}
