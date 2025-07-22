use anyhow::Context;
use bytesize::ByteSize;
use engine_room::EngineConfig;
use std::sync::Arc;
use tokio::{
    select,
    sync::{mpsc, oneshot},
    task::{JoinHandle, spawn_blocking},
};
use tracing::{error, warn};

use crate::{
    context,
    logging::TracingContext,
    namespace_watcher::{Namespace, Registry, SkillDescription},
    registries::{Digest, FileRegistry, OciRegistry, RegistryError, SkillImage, SkillRegistry},
    skill::{Skill, SkillPath},
    wasm::{Engine, SkillLoadError, load_skill_from_wasm_bytes},
};
use futures::{StreamExt, stream::FuturesUnordered};
use std::{collections::HashMap, future::Future, pin::Pin};
use thiserror::Error;

#[derive(Debug)]
pub enum SkillDescriptionFilterType {
    Chat,
    Programmable,
}

// A skill that has been configured and may be fetched and executed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConfiguredSkill {
    pub namespace: Namespace,
    pub description: SkillDescription,
}

impl std::fmt::Display for ConfiguredSkill {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.description {
            SkillDescription::Programmable { name, tag } => {
                write!(f, "{}/{}:{}", self.namespace, name, tag)
            }
            SkillDescription::Chat { name, version, .. } => {
                write!(f, "{}/{}:{}", self.namespace, name, version)
            }
        }
    }
}

impl ConfiguredSkill {
    pub fn new(namespace: Namespace, description: SkillDescription) -> Self {
        Self {
            namespace,
            description,
        }
    }

    pub fn path(&self) -> SkillPath {
        match &self.description {
            SkillDescription::Programmable { name, .. } | SkillDescription::Chat { name, .. } => {
                SkillPath::new(self.namespace.clone(), name)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProgrammableSkill {
    pub namespace: Namespace,
    pub name: String,
    pub tag: String,
}

impl std::fmt::Display for ProgrammableSkill {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}:{}", self.namespace, self.name, self.tag)
    }
}

impl ProgrammableSkill {
    pub fn new(namespace: Namespace, name: String, tag: String) -> Self {
        Self {
            namespace,
            name,
            tag,
        }
    }
}

#[derive(Debug, Error, Clone)]
pub enum SkillFetchError {
    #[error(transparent)]
    RegistryError(#[from] RegistryError),
    #[error(transparent)]
    SkillLoadingError(#[from] SkillLoadError),
    #[error("Skill {0} not found in registry.")]
    SkillNotFound(ProgrammableSkill),
}

/// A skill that has been fetched, loaded, and is ready to use.
#[derive(Clone)]
pub struct LoadedSkill {
    /// The Skill that is ready to use in the Skill Runtime.
    /// If this is a Wasm skill, it is already compiled and linked to the engine.
    pub skill: Arc<dyn Skill>,
    /// The digest at the time of loading from the loader.
    pub digest: Digest,
    /// How big the skill was in the registry. Used as a proxy for roughly how large a skill was
    /// and can be used downstream for things like cache eviction.
    pub size_loaded_from_registry: ByteSize,
}

impl LoadedSkill {
    pub fn new(skill: Arc<dyn Skill>, digest: Digest, size_loaded_from_registry: ByteSize) -> Self {
        Self {
            skill,
            digest,
            size_loaded_from_registry,
        }
    }
}

pub enum SkillLoaderMsg {
    Fetch {
        skill: ProgrammableSkill,
        tracing_context: TracingContext,
        send: oneshot::Sender<Result<LoadedSkill, SkillFetchError>>,
    },
    FetchDigest {
        skill: ProgrammableSkill,
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
    pub fn from_config(
        engine_config: EngineConfig,
        registry_config: RegistryConfig,
    ) -> anyhow::Result<Self> {
        let registries = registry_config
            .registries
            .iter()
            .map(|(k, v)| (k.to_owned(), v.into()))
            .collect();
        Self::new(engine_config, registries)
    }

    pub fn new(
        engine_config: EngineConfig,
        registries: HashMap<Namespace, Arc<dyn SkillRegistry + Send + Sync>>,
    ) -> anyhow::Result<Self> {
        let engine = Engine::new(engine_config)
            .context("engine creation failed")
            .inspect_err(|e| error!(target: "pharia-kernel::skill-loader", "{e:#}"))?;

        let (sender, recv) = mpsc::channel(1);
        let handle = tokio::spawn(async move {
            let mut actor = SkillLoaderActor::new(recv, engine, registries);
            actor.run().await;
        });
        Ok(SkillLoader { sender, handle })
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
        skill: ProgrammableSkill,
        tracing_context: TracingContext,
    ) -> impl Future<Output = Result<LoadedSkill, SkillFetchError>> + Send;

    fn fetch_digest(
        &self,
        skill: ProgrammableSkill,
    ) -> impl Future<Output = Result<Option<Digest>, RegistryError>> + Send;
}

impl SkillLoaderApi for mpsc::Sender<SkillLoaderMsg> {
    async fn fetch(
        &self,
        skill: ProgrammableSkill,
        tracing_context: TracingContext,
    ) -> Result<LoadedSkill, SkillFetchError> {
        let (send, recv) = oneshot::channel();
        self.send(SkillLoaderMsg::Fetch {
            skill,
            tracing_context,
            send,
        })
        .await
        .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }

    async fn fetch_digest(
        &self,
        skill: ProgrammableSkill,
    ) -> Result<Option<Digest>, RegistryError> {
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
        engine: Engine,
        registries: HashMap<Namespace, Arc<dyn SkillRegistry + Send + Sync>>,
    ) -> Self {
        Self {
            receiver,
            engine: Arc::new(engine),
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
        skill: &ProgrammableSkill,
        tracing_context: TracingContext,
    ) -> Result<LoadedSkill, SkillFetchError> {
        let registry_context =
            context!(&tracing_context, "pharia-kernel::skill-loader", "download");
        let skill_bytes = registry
            .load_skill(&skill.name, &skill.tag, registry_context)
            .await?;
        let SkillImage { bytes, digest } = skill_bytes.ok_or_else(|| {
            warn!(parent: tracing_context.span(), "Skill not found in registry.");
            SkillFetchError::SkillNotFound(skill.clone())
        })?;
        let size_loaded_from_registry = ByteSize(bytes.len() as u64);
        let skill_name = skill.to_string();
        let skill = spawn_blocking(move || {
            let load_context =
                context!(&tracing_context, "pharia-kernel::skill-loader", "compile",);
            load_skill_from_wasm_bytes(engine, bytes, load_context).inspect_err(|e| {
                // While most context of errors is known exactly where they occur, we do want to
                // include the skill name as part of the error messages, which is why we log this
                // only here.
                error!(parent: tracing_context.span(), "Failed to load skill {skill_name}: {e}");
            })
        })
        .await
        .expect("Spawned linking thread must run to completion without being poisoned.")?;
        Ok(LoadedSkill::new(
            skill.into(),
            digest,
            size_loaded_from_registry,
        ))
    }

    /// For each new message, create a future that resolves the message and
    /// push it into the running requests. This allows concurrent execution
    /// of multiple requests.
    fn act(&self, msg: SkillLoaderMsg) {
        match msg {
            SkillLoaderMsg::Fetch {
                skill,
                tracing_context,
                send,
            } => {
                let registry = self.registry(&skill.namespace);
                let engine = self.engine.clone();
                self.running_requests.push(Box::pin(async move {
                    let result =
                        Self::fetch(registry.as_ref(), engine, &skill, tracing_context).await;
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
        skill::SkillPath,
    };

    use super::*;

    impl ConfiguredSkill {
        pub fn from_path(skill_path: &SkillPath) -> Self {
            Self::new(
                skill_path.namespace.clone(),
                SkillDescription::Programmable {
                    name: skill_path.name.clone(),
                    tag: "latest".to_owned(),
                },
            )
        }
    }

    impl ProgrammableSkill {
        pub fn from_path(skill_path: &SkillPath) -> Self {
            Self::new(
                skill_path.namespace.clone(),
                skill_path.name.clone(),
                "latest".to_owned(),
            )
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
        pub fn with_file_registry(namespace: Namespace) -> Self {
            let registry_config = RegistryConfig::with_file_registry_named_skills(namespace);
            SkillLoader::from_config(EngineConfig::default(), registry_config).unwrap()
        }

        pub fn with_registries(
            registries: HashMap<Namespace, Arc<dyn SkillRegistry + Send + Sync>>,
        ) -> Self {
            SkillLoader::new(EngineConfig::default(), registries).unwrap()
        }

        pub fn from_registry_config(registry_config: RegistryConfig) -> Self {
            SkillLoader::from_config(EngineConfig::default(), registry_config).unwrap()
        }
    }

    #[tokio::test(start_paused = true)]
    async fn test_skill_loader_fetches_multiple_skills_concurrently() {
        // Given a skill loader with two registries, one that never resolves and one that always does
        let mut registries = HashMap::new();

        let never_resolving = Namespace::new("never-resolving").unwrap();
        let never_resolving_registry =
            Arc::new(NeverResolvingRegistry) as Arc<dyn SkillRegistry + Send + Sync>;
        registries.insert(never_resolving.clone(), never_resolving_registry);

        let ready = Namespace::new("ready").unwrap();
        let ready_registry = Arc::new(ReadyRegistry) as Arc<dyn SkillRegistry + Send + Sync>;
        registries.insert(ready.clone(), ready_registry);

        let skill_loader = SkillLoader::with_registries(registries);
        let never_resolving_skill_path = SkillPath::new(never_resolving, "dummy");
        let ready_skill_path = SkillPath::new(ready, "dummy");

        // When we fetch the never resolving skill
        let api = skill_loader.api();
        let skill = ProgrammableSkill::from_path(&never_resolving_skill_path);
        let handle = tokio::spawn(async move {
            drop(api.fetch(skill, TracingContext::dummy()).await);
        });

        // And waiting 10ms to ensure the message has been received
        sleep(Duration::from_millis(10)).await;

        // Then the other skill can still be fetched
        let skill = ProgrammableSkill::from_path(&ready_skill_path);
        let result = timeout(
            Duration::from_millis(5),
            skill_loader.api().fetch(skill, TracingContext::dummy()),
        )
        .await;
        assert!(result.is_ok());
        drop(handle);
    }
}
