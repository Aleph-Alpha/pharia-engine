use std::sync::Arc;

use tokio::select;
use tokio::sync::{mpsc, oneshot};
use tokio::task::{JoinHandle, spawn_blocking};

use crate::namespace_watcher::{Namespace, Registry};
use crate::registries::{
    Digest, FileRegistry, OciRegistry, RegistryError, SkillImage, SkillRegistry,
};
use crate::skills::{Engine, LoadSkillError, Skill, SkillPath, load_skill_from_wasm_bytes};
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use std::collections::HashMap;
use std::{future::Future, pin::Pin};

// A skill that has been configured and may be fetched and executed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConfiguredSkill {
    pub namespace: Namespace,
    pub name: String,
    pub tag: String,
}

impl std::fmt::Display for ConfiguredSkill {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}:{}", self.namespace, self.name, self.tag)
    }
}

impl ConfiguredSkill {
    pub fn new(namespace: Namespace, name: impl Into<String>, tag: impl Into<String>) -> Self {
        Self {
            namespace,
            name: name.into(),
            tag: tag.into(),
        }
    }

    pub fn path(&self) -> SkillPath {
        SkillPath::new(self.namespace.clone(), &self.name)
    }
}

#[derive(Debug, thiserror::Error, Clone)]
pub enum SkillFetchError {
    #[error(transparent)]
    RegistryError(#[from] RegistryError),
    #[error(transparent)]
    SkillLoadingError(#[from] LoadSkillError),
    #[error("Skill {0} not found in registry.")]
    SkillNotFound(ConfiguredSkill),
}

type SkillAndDigest = (Box<dyn Skill>, Digest);

pub enum SkillLoaderMsg {
    Fetch {
        skill: ConfiguredSkill,
        send: oneshot::Sender<Result<SkillAndDigest, SkillFetchError>>,
    },
    FetchDigest {
        skill: ConfiguredSkill,
        send: oneshot::Sender<Result<Option<Digest>, RegistryError>>,
    },
}

/// Which registry is backing which namespace
pub struct RegistryConfig {
    registries: HashMap<Namespace, Registry>,
}

impl RegistryConfig {
    pub fn new(registries: HashMap<Namespace, Registry>) -> Self {
        Self { registries }
    }

    /// Convert the registry config into a map of namespace to actual skill registry implementations
    pub fn skill_registries(&self) -> HashMap<Namespace, Arc<dyn SkillRegistry + Send + Sync>> {
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
                registry,
                base_repository,
                user,
                password,
            } => Arc::new(OciRegistry::new(
                registry.clone(),
                base_repository.clone(),
                user.clone(),
                password.clone(),
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
        registries: HashMap<Namespace, Arc<dyn SkillRegistry + Send + Sync>>,
    ) -> Self {
        let (sender, recv) = mpsc::channel(1);
        let handle = tokio::spawn(async move {
            let mut actor = SkillLoaderActor::new(recv, engine, registries);
            actor.run().await;
        });
        SkillLoader { sender, handle }
    }

    pub fn api(&self) -> mpsc::Sender<SkillLoaderMsg> {
        self.sender.clone()
    }

    pub async fn wait_for_shutdown(self) {
        // Drop sender so actor terminates, as soon as all api handles are freed.
        drop(self.sender);
        self.handle.await.unwrap();
    }
}

pub trait SkillLoaderApi {
    fn fetch(
        &self,
        skill: ConfiguredSkill,
    ) -> impl Future<Output = Result<(Box<dyn Skill>, Digest), SkillFetchError>> + Send;

    fn fetch_digest(
        &self,
        skill: ConfiguredSkill,
    ) -> impl Future<Output = Result<Option<Digest>, RegistryError>> + Send;
}

impl SkillLoaderApi for mpsc::Sender<SkillLoaderMsg> {
    async fn fetch(
        &self,
        skill: ConfiguredSkill,
    ) -> Result<(Box<dyn Skill>, Digest), SkillFetchError> {
        let (send, recv) = oneshot::channel();
        self.send(SkillLoaderMsg::Fetch { skill, send })
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }

    async fn fetch_digest(&self, skill: ConfiguredSkill) -> Result<Option<Digest>, RegistryError> {
        let (send, recv) = oneshot::channel();
        self.send(SkillLoaderMsg::FetchDigest { skill, send })
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
    registries: HashMap<Namespace, Arc<dyn SkillRegistry + Send + Sync>>,
    running_requests: FuturesUnordered<SkillRequest>,
}

impl SkillLoaderActor {
    pub fn new(
        receiver: mpsc::Receiver<SkillLoaderMsg>,
        engine: Arc<Engine>,
        registries: HashMap<Namespace, Arc<dyn SkillRegistry + Send + Sync>>,
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

    pub fn registry(&self, namespace: &Namespace) -> Arc<dyn SkillRegistry + Send + Sync> {
        self.registries
            .get(namespace)
            .expect("If skill exists, so must the namespace it resides in.")
            .clone()
    }

    /// Load a skill from the registry and build it to a `Skill`
    async fn fetch(
        registry: &(dyn SkillRegistry + Send + Sync),
        engine: Arc<Engine>,
        skill: &ConfiguredSkill,
    ) -> Result<(Box<dyn Skill>, Digest), SkillFetchError> {
        let skill_bytes = registry.load_skill(&skill.name, &skill.tag).await?;
        let SkillImage { bytes, digest } =
            skill_bytes.ok_or_else(|| SkillFetchError::SkillNotFound(skill.clone()))?;
        let skill = spawn_blocking(move || load_skill_from_wasm_bytes(engine.as_ref(), bytes))
            .await
            .expect("Spawned linking thread must run to completion without being poisoned.")?;
        Ok((skill, digest))
    }

    /// For each new message, create a future that resolves the message and
    /// push it into the running requests. This allows concurrent execution
    /// of multiple requests.
    fn act(&self, msg: SkillLoaderMsg) {
        match msg {
            SkillLoaderMsg::Fetch { skill, send } => {
                let registry = self.registry(&skill.namespace);
                let engine = self.engine.clone();
                self.running_requests.push(Box::pin(async move {
                    let result = Self::fetch(registry.as_ref(), engine, &skill).await;
                    drop(send.send(result));
                }));
            }
            SkillLoaderMsg::FetchDigest { skill, send } => {
                let registry = self.registry(&skill.namespace);
                self.running_requests.push(Box::pin(async move {
                    let result = registry.fetch_digest(&skill.name, &skill.tag).await;
                    drop(send.send(result));
                }));
            }
        }
    }
}

#[cfg(test)]
pub mod tests {
    use tokio::time::{Duration, sleep, timeout};

    use crate::{
        namespace_watcher::Registry,
        registries::tests::{NeverResolvingRegistry, ReadyRegistry},
        skills::{Engine, SkillPath},
    };

    use super::*;

    impl ConfiguredSkill {
        pub fn from_path(skill_path: &SkillPath) -> Self {
            Self::new(skill_path.namespace.clone(), &skill_path.name, "latest")
        }
    }

    impl RegistryConfig {
        pub fn with_file_registry_named_skills(namespace: Namespace) -> Self {
            let registry = Registry::File {
                path: "skills".to_owned(),
            };
            let mut registries = HashMap::new();
            registries.insert(namespace, registry);
            Self::new(registries)
        }

        pub fn with_file_registry(namespace: Namespace, path: String) -> Self {
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
        pub fn with_file_registry(engine: Arc<Engine>, namespace: Namespace) -> Self {
            let registry_config = RegistryConfig::with_file_registry_named_skills(namespace);
            SkillLoader::from_config(engine, registry_config)
        }
    }

    #[tokio::test(start_paused = true)]
    async fn test_skill_loader_fetches_multiple_skills_concurrently() {
        // Given a skill loader with two registries, one that never resolves and one that always does
        let engine = Arc::new(Engine::new(false).unwrap());
        let mut registries = HashMap::new();

        let never_resolving = Namespace::new("never-resolving").unwrap();
        let never_resolving_registry =
            Arc::new(NeverResolvingRegistry) as Arc<dyn SkillRegistry + Send + Sync>;
        registries.insert(never_resolving.clone(), never_resolving_registry);

        let ready = Namespace::new("ready").unwrap();
        let ready_registry = Arc::new(ReadyRegistry) as Arc<dyn SkillRegistry + Send + Sync>;
        registries.insert(ready.clone(), ready_registry);

        let skill_loader = SkillLoader::new(engine, registries);
        let never_resolving_skill_path = SkillPath::new(never_resolving, "dummy");
        let ready_skill_path = SkillPath::new(ready, "dummy");

        // When we fetch the never resolving skill
        let api = skill_loader.api();
        let skill = ConfiguredSkill::from_path(&never_resolving_skill_path);
        let handle = tokio::spawn(async move {
            drop(api.fetch(skill).await);
        });

        // And waiting 10ms to ensure the message has been received
        sleep(Duration::from_millis(10)).await;

        // Then the other skill can still be fetched
        let skill = ConfiguredSkill::from_path(&ready_skill_path);
        let result = timeout(Duration::from_millis(5), skill_loader.api().fetch(skill)).await;
        assert!(result.is_ok());
        drop(handle);
    }
}
