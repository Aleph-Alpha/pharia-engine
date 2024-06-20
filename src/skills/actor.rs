use crate::inference::{CompleteTextParameters, InferenceApi};

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
    pub fn new(inference_api: InferenceApi) -> Self {
        let (send, recv) = tokio::sync::mpsc::channel::<SkillExecutorMessage>(1);
        let handle = tokio::spawn(async {
            SkillExecutorActor::new(recv, inference_api).run().await;
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

struct SkillExecutorActor {
    inference_api: InferenceApi,
    recv: mpsc::Receiver<SkillExecutorMessage>,
}

impl SkillExecutorActor {
    fn new(recv: mpsc::Receiver<SkillExecutorMessage>, inference_api: InferenceApi) -> Self {
        SkillExecutorActor {
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
                let prompt = format!(
                    "### Instruction:
                Provide a nice greeting for the person utilizing its given name

                ### Input:
                Name: {name}

                ### Response:"
                );
                let params = CompleteTextParameters {
                    prompt,
                    model: "luminous-nextgen-7b".to_owned(),
                    max_tokens: 10,
                };
                let response = self
                    .inference_api
                    .complete_text(params, msg.api_token)
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
        skills::{Skill, SkillExecutor},
    };
    use tokio::{sync::mpsc, task::JoinHandle};

    use wasmtime::{
        component::{Component, Linker},
        Config, Engine, Store,
    };

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
        let executor = SkillExecutor::new(inference_api);
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
    #[test]
    fn greet_skill_component() {
        let engine = Engine::new(Config::new().async_support(true)).unwrap();
        let _store = Store::new(&engine, ());
        let mut linker = Linker::<()>::new(&engine);

        linker
            .instance("csi")
            .unwrap()
            .func_wrap_async(
                "complete_text",
                |_store, (_model, _prompt): (String, String)| {
                    Box::new(async move { Ok(("dummy response",)) })
                },
            )
            .unwrap();
        Component::from_file(&engine, "./skills/greet_skill.wasm").expect("Loading greet-skill component failed. Please run 'cargo build -p greet-skill --target wasm32-wasi' first.");
    }
}
