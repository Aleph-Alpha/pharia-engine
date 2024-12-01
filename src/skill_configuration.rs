use std::collections::HashMap;

use crate::skills::SkillPath;

use crate::skill_loader::ConfiguredSkill;
use anyhow::anyhow;
use itertools::Itertools;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::info;

pub enum SkillConfigurationMsg {
    List {
        send: oneshot::Sender<Vec<SkillPath>>,
    },
    Remove {
        skill_path: SkillPath,
    },
    Upsert {
        skill: ConfiguredSkill,
    },
    SetNamespaceError {
        namespace: String,
        error: Option<anyhow::Error>,
    },
    Tag {
        skill_path: SkillPath,
        send: oneshot::Sender<anyhow::Result<Option<String>>>,
    },
}

pub struct SkillConfiguration {
    sender: mpsc::Sender<SkillConfigurationMsg>,
    handle: JoinHandle<()>,
}

impl SkillConfiguration {
    pub fn new() -> Self {
        let (sender, recv) = mpsc::channel(1);
        let handle = tokio::spawn(async move {
            let mut actor = SkillConfigurationActor::new(recv);
            actor.run().await;
        });
        Self { sender, handle }
    }
    pub fn api(&self) -> SkillConfigurationApi {
        SkillConfigurationApi::new(self.sender.clone())
    }
    pub async fn wait_for_shutdown(self) {
        drop(self.sender);
        self.handle.await.unwrap();
    }
}

pub struct SkillConfigurationApi {
    sender: mpsc::Sender<SkillConfigurationMsg>,
}

impl SkillConfigurationApi {
    pub fn new(sender: mpsc::Sender<SkillConfigurationMsg>) -> Self {
        Self { sender }
    }
    pub async fn skills(&self) -> Vec<SkillPath> {
        let (send, recv) = oneshot::channel();
        let msg = SkillConfigurationMsg::List { send };
        self.sender
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }
    pub async fn upsert(&self, skill: ConfiguredSkill) {
        let msg = SkillConfigurationMsg::Upsert { skill };
        self.sender
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
    }
    pub async fn remove(&self, skill_path: SkillPath) {
        let msg = SkillConfigurationMsg::Remove { skill_path };
        self.sender
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
    }
    pub async fn set_namespace_error(&self, namespace: String, error: Option<anyhow::Error>) {
        let msg = SkillConfigurationMsg::SetNamespaceError { namespace, error };
        self.sender
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
    }
    pub async fn tag(&self, skill_path: &SkillPath) -> anyhow::Result<Option<String>> {
        let (send, recv) = oneshot::channel();
        let msg = SkillConfigurationMsg::Tag {
            skill_path: skill_path.clone(),
            send,
        };
        self.sender
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }
}

pub struct SkillConfigurationActor {
    receiver: mpsc::Receiver<SkillConfigurationMsg>,
    known_skills: HashMap<SkillPath, String>,
    invalid_namespaces: HashMap<String, anyhow::Error>,
}

impl SkillConfigurationActor {
    pub fn new(receiver: mpsc::Receiver<SkillConfigurationMsg>) -> Self {
        Self {
            receiver,
            known_skills: HashMap::new(),
            invalid_namespaces: HashMap::new(),
        }
    }

    pub async fn run(&mut self) {
        while let Some(msg) = self.receiver.recv().await {
            self.act(msg);
        }
    }

    pub fn act(&mut self, msg: SkillConfigurationMsg) {
        match msg {
            SkillConfigurationMsg::Upsert { skill } => self.upsert_skill(skill),
            SkillConfigurationMsg::Remove { skill_path } => self.remove_skill(&skill_path),
            SkillConfigurationMsg::SetNamespaceError { namespace, error } => {
                if let Some(error) = error {
                    self.add_invalid_namespace(namespace, error);
                } else {
                    self.remove_invalid_namespace(&namespace);
                }
            }
            SkillConfigurationMsg::List { send } => {
                drop(send.send(self.skills().cloned().collect()));
            }
            SkillConfigurationMsg::Tag { skill_path, send } => {
                drop(send.send(self.tag(&skill_path)));
            }
        }
    }

    pub fn upsert_skill(&mut self, skill: ConfiguredSkill) {
        info!("New or changed skill: {skill}");
        let skill_path = SkillPath::new(&skill.namespace, &skill.name);
        self.known_skills.insert(skill_path.clone(), skill.tag);
    }
    pub fn remove_skill(&mut self, skill_path: &SkillPath) {
        info!("Removed skill: {skill_path}");
        self.known_skills.remove(skill_path);
    }
    /// All configured skills, sorted by namespace and name
    pub fn skills(&self) -> impl Iterator<Item = &SkillPath> {
        self.known_skills.keys().sorted_by(|a, b| {
            a.namespace
                .cmp(&b.namespace)
                .then_with(|| a.name.cmp(&b.name))
        })
    }
    pub fn add_invalid_namespace(&mut self, namespace: String, e: anyhow::Error) {
        self.invalid_namespaces.insert(namespace, e);
    }
    pub fn remove_invalid_namespace(&mut self, namespace: &str) {
        self.invalid_namespaces.remove(namespace);
    }
    /// Return the registered tag for a given skill
    pub fn tag(&self, skill_path: &SkillPath) -> anyhow::Result<Option<String>> {
        if let Some(error) = self.invalid_namespaces.get(&skill_path.namespace) {
            return Err(anyhow!("Invalid namespace: {error}"));
        }
        Ok(self.known_skills.get(skill_path).map(ToOwned::to_owned))
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    pub struct StubSkillConfiguration {
        send: mpsc::Sender<SkillConfigurationMsg>,
        #[allow(dead_code)]
        handle: JoinHandle<()>,
    }

    impl StubSkillConfiguration {
        pub fn new(
            mut handle: impl FnMut(SkillConfigurationMsg) + Send + 'static,
        ) -> StubSkillConfiguration {
            let (send, mut recv) = mpsc::channel(1);
            let handle = tokio::spawn(async move {
                while let Some(msg) = recv.recv().await {
                    handle(msg);
                }
            });
            Self { send, handle }
        }

        pub fn api(&self) -> SkillConfigurationApi {
            SkillConfigurationApi::new(self.send.clone())
        }

        pub fn all_skills_allowed() -> StubSkillConfiguration {
            StubSkillConfiguration::new(|msg| match msg {
                SkillConfigurationMsg::Tag { send, .. } => {
                    drop(send.send(Ok(Some("latest".to_owned()))));
                }
                _ => unreachable!(),
            })
        }
    }

    impl SkillConfiguration {
        pub async fn with_skills(skills: Vec<ConfiguredSkill>) -> Self {
            let configuration = Self::new();
            for skill in skills {
                configuration.api().upsert(skill).await;
            }
            configuration
        }
        pub async fn with_skill(skill: ConfiguredSkill) -> Self {
            Self::with_skills(vec![skill]).await
        }
    }
}
