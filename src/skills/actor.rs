use crate::{inference::InferenceApi, skills::runtime::Runtime};
use serde::{Deserialize, Serialize};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

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

#[derive(Serialize, Deserialize, Debug)]
pub enum Skill {
    Greet { name: String },
}

impl SkillExecutorApi {
    pub async fn execute_skill(&mut self, skill: Skill, api_token: String) -> String {
        let (send, recv) = oneshot::channel();
        let msg = SkillExecutorMessage {
            send,
            api_token,
            skill,
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
    inference_api: InferenceApi,
    recv: mpsc::Receiver<SkillExecutorMessage>,
}

impl<R: Runtime> SkillExecutorActor<R> {
    fn new(
        runtime: R,
        recv: mpsc::Receiver<SkillExecutorMessage>,
        inference_api: InferenceApi,
    ) -> Self {
        SkillExecutorActor {
            runtime,
            inference_api,
            recv,
        }
    }
    async fn run(&mut self) {
        while let Some(msg) = self.recv.recv().await {
            self.act(msg).await;
        }
    }
    async fn act(&mut self, msg: SkillExecutorMessage) {
        let _ = match msg.skill {
            Skill::Greet { name } => {
                let response = self
                    .runtime
                    .run_greet(name, msg.api_token, &mut self.inference_api)
                    .await;
                msg.send.send(response)
            }
        };
    }
}

struct SkillExecutorMessage {
    skill: Skill,
    send: oneshot::Sender<String>,
    api_token: String,
}

#[cfg(test)]
mod tests {
    use crate::{
        inference::{InferenceApi, InferenceMessage},
        skills::{RustRuntime, Skill, SkillExecutor},
    };
    use tokio::{sync::mpsc, task::JoinHandle};

    struct InferenceStub {
        send: mpsc::Sender<InferenceMessage>,
        join_handle: JoinHandle<()>,
    }

    impl InferenceStub {
        fn new(completion: String) -> Self {
            let (send, mut recv) = mpsc::channel::<InferenceMessage>(1);

            let join_handle = tokio::spawn(async move {
                match recv.recv().await.unwrap() {
                    InferenceMessage::CompleteText { send, .. } => {
                        send.send(completion).unwrap();
                    }
                }
            });

            Self { send, join_handle }
        }

        async fn shutdown(self) {
            drop(self.send);
            self.join_handle.await.unwrap()
        }

        fn api(&self) -> InferenceApi {
            InferenceApi::new(self.send.clone())
        }
    }

    #[tokio::test]
    async fn greeting_skill() {
        // Given
        let inference = InferenceStub::new("Hello".to_owned());
        let inference_api = inference.api();

        // When
        let runtime = RustRuntime::new();
        let executor = SkillExecutor::new(runtime, inference_api);
        let skill = Skill::Greet {
            name: "".to_owned(),
        };
        let result = executor
            .api()
            .execute_skill(skill, "TOKEN_NOT_REQUIRED".to_owned())
            .await;
        executor.shutdown().await;
        inference.shutdown().await;

        // Then
        assert_eq!("Hello", result);
    }
}
