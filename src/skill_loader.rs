use std::sync::Arc;

use anyhow::{anyhow, Context};
use tokio::sync::{mpsc, oneshot};
use tokio::task::{spawn_blocking, JoinHandle};

use crate::registries::{SkillImage, SkillRegistry};
use crate::skills::{Engine, Skill, SkillPath};

use std::collections::HashMap;

pub enum SkillLoaderMsg {
    Fetch {
        skill_path: SkillPath,
        tag: String,
        send: oneshot::Sender<anyhow::Result<(Skill, String)>>,
    },
    FetchDigest {
        skill_path: SkillPath,
        tag: String,
        send: oneshot::Sender<anyhow::Result<Option<String>>>,
    },
}

pub struct SkillLoader {
    sender: mpsc::Sender<SkillLoaderMsg>,
    handle: JoinHandle<()>,
}

impl SkillLoader {
    pub fn new(
        engine: Arc<Engine>,
        registries: HashMap<String, Box<dyn SkillRegistry + Send + Sync>>,
    ) -> Self {
        let (sender, recv) = mpsc::channel(1);
        let mut actor = SkillLoaderActor::new(recv, engine, registries);
        let handle = tokio::spawn(async move {
            actor.run().await;
        });
        SkillLoader { sender, handle }
    }

    pub fn api(&self) -> SkillLoaderApi {
        SkillLoaderApi {
            sender: self.sender.clone(),
        }
    }

    pub async fn wait_for_shutdown(self) {
        // Drop sender so actor terminates, as soon as all api handles are freed.
        drop(self.sender);
        self.handle.await.unwrap();
    }
}
pub struct SkillLoaderApi {
    sender: mpsc::Sender<SkillLoaderMsg>,
}

impl SkillLoaderApi {
    pub async fn fetch(
        &self,
        skill_path: SkillPath,
        tag: String,
    ) -> Result<(Skill, String), anyhow::Error> {
        let (send, recv) = oneshot::channel();
        self.sender
            .send(SkillLoaderMsg::Fetch {
                skill_path,
                tag,
                send,
            })
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }

    pub async fn fetch_digest(
        &self,
        skill_path: SkillPath,
        tag: String,
    ) -> Result<Option<String>, anyhow::Error> {
        let (send, recv) = oneshot::channel();
        self.sender
            .send(SkillLoaderMsg::FetchDigest {
                skill_path,
                tag,
                send,
            })
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }
}

/// Owns access to the registries and provides ready-to-use skills and skill digests.
pub struct SkillLoaderActor {
    receiver: mpsc::Receiver<SkillLoaderMsg>,
    engine: Arc<Engine>,
    registries: HashMap<String, Box<dyn SkillRegistry + Send + Sync>>,
}

impl SkillLoaderActor {
    pub fn new(
        receiver: mpsc::Receiver<SkillLoaderMsg>,
        engine: Arc<Engine>,
        registries: HashMap<String, Box<dyn SkillRegistry + Send + Sync>>,
    ) -> Self {
        Self {
            receiver,
            engine,
            registries,
        }
    }

    pub async fn run(&mut self) {
        while let Some(msg) = self.receiver.recv().await {
            self.act(msg).await;
        }
    }

    async fn act(&self, msg: SkillLoaderMsg) {
        match msg {
            SkillLoaderMsg::Fetch {
                skill_path,
                tag,
                send,
            } => {
                let result = self.fetch(&skill_path, &tag).await;
                drop(send.send(result));
            }
            SkillLoaderMsg::FetchDigest {
                skill_path,
                tag,
                send,
            } => {
                let result = self.fetch_digest(&skill_path, &tag).await;
                drop(send.send(result));
            }
        }
    }

    /// Load a skill from the registry and build it to a `Skill`
    async fn fetch(&self, skill_path: &SkillPath, tag: &str) -> anyhow::Result<(Skill, String)> {
        let registry = self
            .registries
            .get(&skill_path.namespace)
            .expect("If skill exists, so must the namespace it resides in.");

        let skill_bytes = registry.load_skill(&skill_path.name, tag).await?;
        let SkillImage { bytes, digest } = skill_bytes
            .ok_or_else(|| anyhow!("Skill {skill_path} configured but not loadable."))?;
        let engine = self.engine.clone();
        let skill = spawn_blocking(move || Skill::new(engine.as_ref(), bytes))
            .await
            .expect("Spawned linking thread must run to completion without being poisoned.")
            .with_context(|| format!("Failed to initialize {skill_path}."))?;
        Ok((skill, digest))
    }

    /// Fetch the digest for a skill from the registry
    pub async fn fetch_digest(
        &self,
        skill_path: &SkillPath,
        tag: &str,
    ) -> anyhow::Result<Option<String>> {
        let registry = self
            .registries
            .get(&skill_path.namespace)
            .expect("If skill exists, so must the namespace it resides in.");
        registry.fetch_digest(&skill_path.name, tag).await
    }
}

#[cfg(test)]
pub mod tests {
    use crate::namespace_watcher::{NamespaceConfig, Registry};

    use super::*;

    impl SkillLoader {
        /// Skill loader loading skills from a local
        /// `skills` directory
        pub fn with_file_registry(engine: Arc<Engine>, namespace: String) -> Self {
            let registry = Registry::File {
                path: "skills".to_owned(),
            };
            let ns_cfg = NamespaceConfig::TeamOwned {
                config_url: "file://namespace.toml".to_owned(),
                config_access_token_env_var: None,
                registry,
            };

            let mut registries = HashMap::new();
            registries.insert(namespace, (&ns_cfg).into());
            SkillLoader::new(engine, registries)
        }

        /// Skill loader with a registry called `local` loading skills from a local `skills` directory
        pub fn with_file_registry_named_local(engine: Arc<Engine>) -> Self {
            SkillLoader::with_file_registry(engine, "local".to_owned())
        }
    }
}
