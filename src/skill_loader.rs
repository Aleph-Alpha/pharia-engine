use std::sync::Arc;

use anyhow::{anyhow, Context};
use tokio::select;
use tokio::sync::{mpsc, oneshot};
use tokio::task::{spawn_blocking, JoinHandle};

use crate::namespace_watcher::Registry;
use crate::registries::{Digest, FileRegistry, OciRegistry, SkillImage, SkillRegistry};
use crate::skills::{Engine, Skill, SkillPath};
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use std::collections::HashMap;
use std::{future::Future, pin::Pin};

// A skill that has been configured and may be fetched and executed.
#[derive(Debug, Clone)]
pub struct ConfiguredSkill {
    skill_path: SkillPath,
    tag: Option<String>,
}

impl ConfiguredSkill {
    pub fn new(skill_path: SkillPath, tag: Option<String>) -> Self {
        Self { skill_path, tag }
    }

    pub fn namespace(&self) -> &str {
        &self.skill_path.namespace
    }

    pub fn name(&self) -> &str {
        &self.skill_path.name
    }

    pub fn tag_or_default(&self) -> &str {
        self.tag.as_deref().unwrap_or("latest")
    }
}

pub enum SkillLoaderMsg {
    Fetch {
        skill: ConfiguredSkill,
        send: oneshot::Sender<anyhow::Result<(Skill, Digest)>>,
    },
    FetchDigest {
        skill: ConfiguredSkill,
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
    pub fn skill_registries(&self) -> HashMap<String, Arc<dyn SkillRegistry + Send + Sync>> {
        self.registries
            .iter()
            .map(|(k, v)| (k.to_owned(), v.into()))
            .collect()
    }
}

impl From<&Registry> for Arc<dyn SkillRegistry + Send + Sync> {
    fn from(val: &Registry) -> Self {
        match val {
            Registry::File { path } => Arc::new(FileRegistry::with_dir(path)),
            Registry::Oci {
                repository,
                registry,
                auth,
            } => Arc::new(OciRegistry::new(
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
        registries: HashMap<String, Arc<dyn SkillRegistry + Send + Sync>>,
    ) -> Self {
        let (sender, recv) = mpsc::channel(1);
        let handle = tokio::spawn(async move {
            let mut actor = SkillLoaderActor::new(recv, engine, registries);
            actor.run().await;
        });
        SkillLoader { sender, handle }
    }

    pub fn api(&self) -> SkillLoaderApi {
        SkillLoaderApi::new(self.sender.clone())
    }

    pub async fn wait_for_shutdown(self) {
        // Drop sender so actor terminates, as soon as all api handles are freed.
        drop(self.sender);
        self.handle.await.unwrap();
    }
}

#[derive(Clone)]
pub struct SkillLoaderApi {
    sender: mpsc::Sender<SkillLoaderMsg>,
}

impl SkillLoaderApi {
    pub fn new(sender: mpsc::Sender<SkillLoaderMsg>) -> Self {
        Self { sender }
    }
}

impl SkillLoaderApi {
    pub async fn fetch(&self, skill: ConfiguredSkill) -> Result<(Skill, Digest), anyhow::Error> {
        let (send, recv) = oneshot::channel();
        self.sender
            .send(SkillLoaderMsg::Fetch { skill, send })
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }

    pub async fn fetch_digest(
        &self,
        skill: ConfiguredSkill,
    ) -> Result<Option<Digest>, anyhow::Error> {
        let (send, recv) = oneshot::channel();
        self.sender
            .send(SkillLoaderMsg::FetchDigest { skill, send })
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }
}

/// A future that fetches a skill and its digest and sends them to the message sender.
type SkillRequest = Pin<Box<dyn Future<Output = ()> + Send>>;

/// Owns access to the registries and provides ready-to-use skills and skill digests.
pub struct SkillLoaderActor {
    receiver: mpsc::Receiver<SkillLoaderMsg>,
    engine: Arc<Engine>,
    registries: HashMap<String, Arc<dyn SkillRegistry + Send + Sync>>,
    running_requests: FuturesUnordered<SkillRequest>,
}

impl SkillLoaderActor {
    pub fn new(
        receiver: mpsc::Receiver<SkillLoaderMsg>,
        engine: Arc<Engine>,
        registries: HashMap<String, Arc<dyn SkillRegistry + Send + Sync>>,
    ) -> Self {
        Self {
            receiver,
            engine,
            registries,
            running_requests: FuturesUnordered::new(),
        }
    }

    pub async fn run(&mut self) {
        loop {
            select! {
                msg = self.receiver.recv() => match msg {
                Some(msg) => self.act(msg),
                // Senders are gone, break out of the loop for shutdown.
                None => break
            },
            // FuturesUnordered will let them run in parallel. It will
            // yield once one of them is completed.
                () = self.running_requests.select_next_some(), if !self.running_requests.is_empty()  => {}
            }
        }
    }

    pub fn registry(&self, namespace: &str) -> Arc<dyn SkillRegistry + Send + Sync> {
        self.registries
            .get(namespace)
            .expect("If skill exists, so must the namespace it resides in.")
            .clone()
    }

    /// Load a skill from the registry and build it to a `Skill`
    async fn fetch<'a>(
        registry: &(dyn SkillRegistry + Send + Sync),
        engine: Arc<Engine>,
        skill: &ConfiguredSkill,
    ) -> anyhow::Result<(Skill, Digest)> {
        let skill_bytes = registry
            .load_skill(skill.name(), skill.tag_or_default())
            .await?;
        let SkillImage { bytes, digest } =
            skill_bytes.ok_or_else(|| anyhow!("Skill {skill:?} configured but not loadable."))?;
        let skill = spawn_blocking(move || Skill::new(engine.as_ref(), bytes))
            .await
            .expect("Spawned linking thread must run to completion without being poisoned.")
            .with_context(|| format!("Failed to initialize {skill:?}."))?;
        Ok((skill, digest))
    }

    /// For each new message, create a future that resolves the message and
    /// push it into the running requests. This allows concurrent execution
    /// of multiple requests.
    fn act(&self, msg: SkillLoaderMsg) {
        match msg {
            SkillLoaderMsg::Fetch { skill, send } => {
                let registry = self.registry(skill.namespace());
                let engine = self.engine.clone();
                let fut = async move {
                    let result = Self::fetch(registry.as_ref(), engine, &skill).await;
                    drop(send.send(result));
                };
                self.running_requests.push(Box::pin(fut));
            }
            SkillLoaderMsg::FetchDigest { skill, send } => {
                let registry = self.registry(skill.namespace());
                let fut = async move {
                    let result = registry
                        .fetch_digest(skill.name(), skill.tag_or_default())
                        .await;
                    drop(send.send(result));
                };
                self.running_requests.push(Box::pin(fut));
            }
        }
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

    impl ConfiguredSkill {
        pub fn without_tag(skill_path: SkillPath) -> Self {
            Self::new(skill_path, None)
        }
    }

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

    #[tokio::test(start_paused = true)]
    async fn test_skill_loader_fetches_multiple_skills_concurrently() {
        // Given a skill loader with two registries, one that never resolves and one that always does
        let engine = Arc::new(Engine::new(false).unwrap());
        let never_resolving =
            Arc::new(NeverResolvingRegistry) as Arc<dyn SkillRegistry + Send + Sync>;
        let ready_registry = Arc::new(ReadyRegistry) as Arc<dyn SkillRegistry + Send + Sync>;
        let mut registries = HashMap::new();
        registries.insert("never-resolving".to_owned(), never_resolving);
        registries.insert("ready".to_owned(), ready_registry);
        let skill_loader = SkillLoader::new(engine, registries);
        let never_resolving_skill_path = SkillPath::new("never-resolving", "dummy");
        let ready_skill_path = SkillPath::new("ready", "dummy");

        // When we fetch the never resolving skill
        let api = skill_loader.api();
        let skill = ConfiguredSkill::new(never_resolving_skill_path, Some("dummy".to_owned()));
        let handle = tokio::spawn(async move {
            drop(api.fetch(skill).await);
        });

        // And waiting 10ms to ensure the message has been received
        sleep(Duration::from_millis(10)).await;

        // Then the other skill can still be fetched
        let skill = ConfiguredSkill::new(ready_skill_path, Some("dummy".to_owned()));
        let result = timeout(Duration::from_millis(5), skill_loader.api().fetch(skill)).await;
        assert!(result.is_ok());
        drop(handle);
    }
}
