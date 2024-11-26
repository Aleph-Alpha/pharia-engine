use std::sync::Arc;

use anyhow::{anyhow, Context};
use tokio::sync::{mpsc, oneshot};
use tokio::task::{spawn_blocking, JoinHandle};

use crate::namespace_watcher::Registry;
use crate::registries::{Digest, FileRegistry, OciRegistry, SkillImage, SkillRegistry};
use crate::skills::{Engine, Skill, SkillPath};

use std::collections::HashMap;

pub enum SkillLoaderMsg {
    Fetch {
        skill_path: SkillPath,
        tag: String,
        send: oneshot::Sender<anyhow::Result<(Skill, Digest)>>,
    },
    FetchDigest {
        skill_path: SkillPath,
        tag: String,
        send: oneshot::Sender<anyhow::Result<Option<Digest>>>,
    },
}

/// Which registry is backing which namespace
pub struct RegistryConfig {
    registries: HashMap<String, Registry>,
}

impl RegistryConfig {
    pub fn new(registries: HashMap<String, Registry>) -> Self {
        Self { registries }
    }

    /// Convert the registry config into a map of namespace to actual skill registry implementations
    pub fn skill_registries(&self) -> HashMap<String, Box<dyn SkillRegistry + Send + Sync>> {
        self.registries
            .iter()
            .map(|(k, v)| (k.to_owned(), v.into()))
            .collect()
    }
}

impl From<&Registry> for Box<dyn SkillRegistry + Send + Sync> {
    fn from(val: &Registry) -> Self {
        match val {
            Registry::File { path } => Box::new(FileRegistry::with_dir(path)),
            Registry::Oci {
                repository,
                registry,
                auth,
            } => Box::new(OciRegistry::new(
                repository.clone(),
                registry.clone(),
                auth.user(),
                auth.password(),
            )),
        }
    }
}

pub struct SkillLoader {
    sender: mpsc::Sender<SkillLoaderMsg>,
    handle: JoinHandle<()>,
}

impl SkillLoader {
    pub fn from_config(engine: Arc<Engine>, registry_config: RegistryConfig) -> Self {
        let registries = registry_config
            .registries
            .iter()
            .map(|(k, v)| (k.to_owned(), v.into()))
            .collect();
        Self::new(engine, registries)
    }

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
    ) -> Result<(Skill, Digest), anyhow::Error> {
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
    ) -> Result<Option<Digest>, anyhow::Error> {
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
    async fn fetch(&self, skill_path: &SkillPath, tag: &str) -> anyhow::Result<(Skill, Digest)> {
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
    ) -> anyhow::Result<Option<Digest>> {
        let registry = self
            .registries
            .get(&skill_path.namespace)
            .expect("If skill exists, so must the namespace it resides in.");
        registry.fetch_digest(&skill_path.name, tag).await
    }
}

#[cfg(test)]
pub mod tests {
    use tokio::time::{sleep, timeout, Duration};

    use crate::{
        namespace_watcher::Registry,
        registries::tests::{NeverResolvingRegistry, ReadyRegistry},
        skills::Engine,
    };

    use super::*;

    impl RegistryConfig {
        pub fn with_file_registry_named_skills(namespace: String) -> Self {
            let registry = Registry::File {
                path: "skills".to_owned(),
            };
            let mut registries = HashMap::new();
            registries.insert(namespace, registry);
            Self::new(registries)
        }

        pub fn with_file_registry(namespace: String, path: String) -> Self {
            let registry = Registry::File { path };
            let mut registries = HashMap::new();
            registries.insert(namespace, registry);
            Self::new(registries)
        }

        pub fn empty() -> Self {
            Self::new(HashMap::new())
        }
    }

    impl SkillLoader {
        /// Skill loader loading skills from a local `skills` directory
        pub fn with_file_registry(engine: Arc<Engine>, namespace: String) -> Self {
            let registry_config = RegistryConfig::with_file_registry_named_skills(namespace);
            SkillLoader::from_config(engine, registry_config)
        }
    }

    #[tokio::test]
    async fn test_skill_loader_fetches_multiple_skills_concurrently() {
        // Given a skill loader with two registries, one that never resolves and one that always does
        let engine = Arc::new(Engine::new(false).unwrap());
        let never_resolving =
            Box::new(NeverResolvingRegistry) as Box<dyn SkillRegistry + Send + Sync>;
        let ready_registry = Box::new(ReadyRegistry) as Box<dyn SkillRegistry + Send + Sync>;
        let mut registries = HashMap::new();
        registries.insert("never-resolving".to_owned(), never_resolving);
        registries.insert("ready".to_owned(), ready_registry);
        let skill_loader = SkillLoader::new(engine, registries);
        let never_resolving_skill_path = SkillPath::new("never-resolving", "dummy");
        let ready_skill_path = SkillPath::new("ready", "dummy");

        // When we fetch the never resolving skill
        let api = skill_loader.api();
        let handle = tokio::spawn(async move {
            drop(
                api.fetch(never_resolving_skill_path, "dummy".to_owned())
                    .await,
            );
        });

        // And waiting 10ms to ensure the message has been received
        sleep(Duration::from_millis(10)).await;

        // Then the other skill can still be fetched
        let result = timeout(
            Duration::from_millis(5),
            skill_loader
                .api()
                .fetch(ready_skill_path, "dummy".to_owned()),
        )
        .await;
        assert!(result.is_ok());
        drop(handle);
    }
}
