use crate::skills::runtime::Runtime;
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
    pub fn new<R: Runtime + Send + 'static>(runtime: R) -> Self {
        let (send, recv) = tokio::sync::mpsc::channel::<SkillExecutorMessage>(1);
        let handle = tokio::spawn(async {
            SkillExecutorActor::new(runtime, recv).run().await;
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
    recv: mpsc::Receiver<SkillExecutorMessage>,
}

impl<R: Runtime> SkillExecutorActor<R> {
    fn new(runtime: R, recv: mpsc::Receiver<SkillExecutorMessage>) -> Self {
        SkillExecutorActor { runtime, recv }
    }
    async fn run(&mut self) {
        while let Some(msg) = self.recv.recv().await {
            self.act(msg).await;
        }
    }
    async fn act(&mut self, msg: SkillExecutorMessage) {
        let _ = match msg.skill {
            Skill::Greet { name } => {
                let response = self.runtime.run_greet(name, msg.api_token).await;
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
        inference::tests::InferenceStub,
        skills::{RustRuntime, Skill, SkillExecutor},
    };

    #[tokio::test]
    async fn greeting_skill() {
        // Given
        let inference = InferenceStub::new("Hello".to_owned());

        // When
        let runtime = RustRuntime::new(inference.api());
        let executor = SkillExecutor::new(runtime);
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
