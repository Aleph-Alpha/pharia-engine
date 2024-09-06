use std::{collections::HashMap, env, sync::Arc};

use anyhow::{anyhow, Context};
use serde_json::Value;
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use crate::{
    configuration_observer::{NamespaceConfig, Registry},
    registries::{FileRegistry, OciRegistry, SkillRegistry},
    skills::SkillPath,
};

use super::{
    engine::{Engine, Skill},
    CsiForSkills,
};

pub struct SkillProvider {
    known_skills: HashMap<SkillPath, Option<String>>,
    cached_skills: HashMap<SkillPath, Arc<CachedSkill>>,
    // key: Namespace, value: Registry
    skill_registries: HashMap<String, Box<dyn SkillRegistry + Send>>,
    // key: Namespace, value: Error
    invalid_namespaces: HashMap<String, anyhow::Error>,
}

impl SkillProvider {
    pub fn new(namespaces: &HashMap<String, NamespaceConfig>) -> Self {
        let skill_registries = namespaces
            .iter()
            .map(|(k, v)| (k.to_owned(), Self::registry(v)))
            .collect::<HashMap<_, _>>();
        SkillProvider {
            known_skills: HashMap::new(),
            cached_skills: HashMap::new(),
            skill_registries,
            invalid_namespaces: HashMap::new(),
        }
    }

    fn registry(namespace_config: &NamespaceConfig) -> Box<dyn SkillRegistry + Send> {
        match &namespace_config.registry {
            Registry::File { path } => Box::new(FileRegistry::with_dir(path)),
            Registry::Oci {
                repository,
                registry,
            } => {
                drop(dotenvy::dotenv());
                let username = env::var("SKILL_REGISTRY_USER")
                    .expect("SKILL_REGISTRY_USER must be set if OCI registry is used.");
                let password = env::var("SKILL_REGISTRY_PASSWORD")
                    .expect("SKILL_REGISTRY_PASSWORD must be set if OCI registry is used.");
                Box::new(OciRegistry::new(
                    repository.clone(),
                    registry.clone(),
                    username,
                    password,
                ))
            }
        }
    }

    pub fn upsert_skill(&mut self, skill: &SkillPath, tag: Option<String>) {
        if self.known_skills.insert(skill.clone(), tag).is_some() {
            self.invalidate(skill);
        }
    }

    pub fn remove_skill(&mut self, skill: &SkillPath) {
        self.known_skills.remove(skill);
        self.invalidate(skill);
    }

    pub fn skills(&self) -> impl Iterator<Item = &SkillPath> {
        self.known_skills.keys()
    }

    pub fn loaded_skills(&self) -> impl Iterator<Item = &SkillPath> + '_ {
        self.cached_skills.keys()
    }

    pub fn invalidate(&mut self, skill_path: &SkillPath) -> bool {
        self.cached_skills.remove(skill_path).is_some()
    }

    pub fn add_invalid_namespace(&mut self, namespace: String, e: anyhow::Error) {
        self.invalid_namespaces.insert(namespace, e);
    }

    pub fn remove_invalid_namespace(&mut self, namespace: &str) {
        self.invalid_namespaces.remove(namespace);
    }

    /// `Some` if the skill can be successfully loaded, `None` if the skill can not be found
    pub async fn fetch(
        &mut self,
        skill_path: &SkillPath,
        engine: &Engine,
    ) -> anyhow::Result<Option<Arc<CachedSkill>>> {
        if let Some(error) = self.invalid_namespaces.get(&skill_path.namespace) {
            return Err(anyhow!("Invalid namespace: {error}"));
        }

        if !self.cached_skills.contains_key(skill_path) {
            let Some(tag) = self.known_skills.get(skill_path) else {
                return Ok(None);
            };

            let registry = self
                .skill_registries
                .get(&skill_path.namespace)
                .expect("If skill exists, so must the namespace it resides in.");

            let bytes = registry
                .load_skill(&skill_path.name, tag.as_deref().unwrap_or("latest"))
                .await?;
            let bytes = bytes.ok_or_else(|| anyhow!("Sorry, skill {skill_path} not found."))?;
            let skill = CachedSkill::new(engine, bytes)
                .with_context(|| format!("Failed to initialize {skill_path}."))?;
            self.cached_skills
                .insert(skill_path.clone(), Arc::new(skill));
        }
        Ok(Some(
            self.cached_skills
                .get(skill_path)
                .expect("Skill present.")
                .clone(),
        ))
    }
}

pub struct CachedSkill {
    skill: Skill,
}

impl CachedSkill {
    pub fn new(engine: &Engine, bytes: impl AsRef<[u8]>) -> anyhow::Result<Self> {
        let skill = engine.instantiate_pre_skill(bytes)?;
        Ok(Self { skill })
    }

    pub async fn run(
        &self,
        engine: &Engine,
        ctx: Box<dyn CsiForSkills + Send>,
        input: Value,
    ) -> anyhow::Result<Value> {
        self.skill.run(engine, ctx, input).await
    }
}

pub struct SkillProviderActorHandle {
    sender: mpsc::Sender<SkillProviderMsg>,
    handle: JoinHandle<()>,
}

impl SkillProviderActorHandle {
    pub fn new(namespaces: &HashMap<String, NamespaceConfig>) -> Self {
        let (sender, recv) = mpsc::channel(1);
        let mut actor = SkillProviderActor::new(recv, namespaces);
        let handle = tokio::spawn(async move {
            actor.run().await;
        });
        SkillProviderActorHandle { sender, handle }
    }

    pub fn api(&self) -> SkillProviderApi {
        SkillProviderApi::new(self.sender.clone())
    }

    pub async fn wait_for_shutdown(self) {
        // Drop sender so actor terminates, as soon as all api handles are freed.
        drop(self.sender);
        self.handle.await.unwrap();
    }
}

pub struct SkillProviderApi {
    sender: mpsc::Sender<SkillProviderMsg>,
}

impl SkillProviderApi {
    pub fn new(sender: mpsc::Sender<SkillProviderMsg>) -> Self {
        SkillProviderApi { sender }
    }

    pub async fn remove(&self, skill_path: SkillPath) {
        let msg = SkillProviderMsg::Remove { skill_path };
        self.sender
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
    }

    pub async fn upsert(&self, skill_path: SkillPath, tag: Option<String>) {
        let msg = SkillProviderMsg::Upsert { skill_path, tag };
        self.sender
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
    }

    /// Report a namespace as erroneous (e.g. in case its configuration is messed up). Set `None`
    /// to communicate that a namespace is no longer erroneous.
    pub async fn set_namespace_error(&self, namespace: String, error: Option<anyhow::Error>) {
        let msg = SkillProviderMsg::SetNamespaceError { namespace, error };
        self.sender.send(msg).await.expect("all api handlers must be shutdown before actors");
    }
}

pub enum SkillProviderMsg {
    Fetch {
        skill_path: SkillPath,
        engine: Arc<Engine>,
        send: oneshot::Sender<Result<Option<Arc<CachedSkill>>, anyhow::Error>>,
    },
    List {
        send: oneshot::Sender<Vec<SkillPath>>,
    },
    Remove {
        skill_path: SkillPath,
    },
    Upsert {
        skill_path: SkillPath,
        tag: Option<String>,
    },
    SetNamespaceError {
        namespace: String,
        error: Option<anyhow::Error>,
    }
}

struct SkillProviderActor {
    receiver: mpsc::Receiver<SkillProviderMsg>,
    provider: SkillProvider,
}

impl SkillProviderActor {
    pub fn new(
        receiver: mpsc::Receiver<SkillProviderMsg>,
        namespaces: &HashMap<String, NamespaceConfig>,
    ) -> Self {
        SkillProviderActor {
            receiver,
            provider: SkillProvider::new(namespaces),
        }
    }

    pub async fn run(&mut self) {
        while let Some(msg) = self.receiver.recv().await {
            self.act(msg).await;
        }
    }

    pub async fn act(&mut self, msg: SkillProviderMsg) {
        match msg {
            SkillProviderMsg::Fetch {
                skill_path,
                engine,
                send,
            } => {
                let result = self.provider.fetch(&skill_path, &engine).await;
                drop(send.send(result));
            }
            SkillProviderMsg::List { send } => {
                drop(send.send(self.provider.skills().cloned().collect()));
            }
            SkillProviderMsg::Remove { skill_path } => {
                self.provider.remove_skill(&skill_path);
            }
            SkillProviderMsg::Upsert { skill_path, tag } => {
                self.provider.upsert_skill(&skill_path, tag);
            }
            SkillProviderMsg::SetNamespaceError { namespace, error } => {
                if let Some(error) = error {
                    self.provider.add_invalid_namespace(namespace, error);
                } else {
                    self.provider.remove_invalid_namespace(&namespace);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    impl SkillProvider {
        fn with_namespace_and_skill(skill_path: &SkillPath) -> Self {
            let registry = Registry::File {
                path: "skills".to_owned(),
            };
            let ns_cfg = NamespaceConfig {
                config_url: "file://namespace.toml".to_owned(),
                config_access_token_env_var: None,
                registry,
            };
            let mut namespaces = HashMap::new();
            namespaces.insert(skill_path.namespace.clone(), ns_cfg);

            let mut provider = SkillProvider::new(&namespaces);
            provider.upsert_skill(skill_path, None);
            provider
        }
    }

    #[tokio::test]
    async fn skill_component_is_in_config() {
        let skill_path = SkillPath::dummy();
        let mut provider = SkillProvider::with_namespace_and_skill(&skill_path);
        let engine = Engine::new().unwrap();

        let result = provider.fetch(&skill_path, &engine).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn skill_component_not_in_config() {
        let skill_path = SkillPath::dummy();
        let mut provider = SkillProvider::with_namespace_and_skill(&skill_path);
        let engine = Engine::new().unwrap();

        let result = provider
            .fetch(
                &SkillPath::new(&skill_path.namespace, "non_existing_skill"),
                &engine,
            )
            .await;

        assert!(matches!(result, Ok(None)));
    }

    #[tokio::test]
    async fn namespace_not_in_config() {
        let skill_path = SkillPath::dummy();
        let mut provider = SkillProvider::with_namespace_and_skill(&skill_path);
        let engine = Engine::new().unwrap();

        let result = provider
            .fetch(
                &SkillPath::new("non_existing_namespace", &skill_path.name),
                &engine,
            )
            .await;

        assert!(matches!(result, Ok(None)));
    }

    #[tokio::test]
    async fn cached_skill_removed() {
        // given one cached skill
        let skill_path = SkillPath::new("local", "greet_skill");
        let mut provider = SkillProvider::with_namespace_and_skill(&skill_path);
        let engine = Engine::new().unwrap();
        provider.fetch(&skill_path, &engine).await.unwrap();

        // when we remove the skill
        provider.remove_skill(&skill_path);

        // then the skill is no longer cached
        assert!(provider.loaded_skills().next().is_none());
    }

    #[tokio::test]
    async fn error_fetching_skill_in_invalid_namespace() {
        // given a skill in an invalid namespace
        let skill_path = SkillPath::new("local", "greet_skill");
        let mut provider = SkillProvider::with_namespace_and_skill(&skill_path);
        provider.add_invalid_namespace(
            skill_path.namespace.clone(),
            anyhow!(""),
        );
        let engine = Engine::new().unwrap();

        // when fetching the skill
        let result = provider.fetch(&skill_path, &engine).await;

        // then it returns an error
        assert!(result.is_err());
    }
}
