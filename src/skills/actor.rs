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
                let ctx = Box::new(SkillInvocationCtx::new(
                    self.inference_api.clone(),
                    api_token,
                ));
                let response = self.runtime.run(&skill, input, ctx).await;
                drop(send.send(response));
            }
            SkillExecutorMessage::Skills { send } => {
                let response = self.runtime.skills().map(str::to_owned).collect();
                drop(send.send(response));
            }
        }
    }
}

enum SkillExecutorMessage {
    Execute {
        skill: String,
        input: String,
        send: oneshot::Sender<Result<String, Error>>,
        api_token: String,
    },
    Skills {
        send: oneshot::Sender<Vec<String>>,
    },
}

#[cfg(test)]
pub mod tests {
    use anyhow::Error;

    use crate::{
        inference::tests::InferenceStub,
        skills::{
            csi::Csi,
            runtime::Runtime,
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
    }

    #[tokio::test]
    async fn list_skills() {
        // Given a runtime with five skills
        let skills = vec!["First skill".to_owned(), "Second skill".to_owned()];
        let inference = InferenceStub::with_completion("Hello".to_owned());
        let runtime = LiarRuntime::new(skills.clone());

        // When
        let executor = SkillExecutor::new(runtime, inference.api());
        let result = executor.api().skills().await;

        executor.shutdown().await;
        inference.shutdown().await;

        // Then
        assert_eq!(result.len(), skills.len());
    }
}
