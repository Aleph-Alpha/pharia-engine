use std::{collections::HashMap, env, sync::Arc};

use anyhow::{anyhow, Context};
use tokio::{
    sync::{mpsc, oneshot},
    task::{spawn_blocking, JoinHandle},
};
use tracing::info;

use crate::{
    namespace_watcher::{NamespaceConfig, Registry},
    registries::{FileRegistry, OciRegistry, SkillImage, SkillRegistry},
    skills::{Engine, Skill, SkillPath},
};

/// A wrapper around the Skill to also keep track of which
/// digest was loaded last and when it was last checked.
struct CachedSkill {
    /// Compiled and pre initialized skill
    skill: Arc<Skill>,
    /// Digest of the skill when it was last loaded from the registry
    digest: String,
}

impl CachedSkill {
    fn new(skill: Arc<Skill>, digest: impl Into<String>) -> Self {
        Self {
            skill,
            digest: digest.into(),
        }
    }
}

struct SkillStoreState {
    known_skills: HashMap<SkillPath, Option<String>>,
    cached_skills: HashMap<SkillPath, CachedSkill>,
    // key: Namespace, value: Registry
    skill_registries: HashMap<String, Box<dyn SkillRegistry + Send>>,
    // key: Namespace, value: Error
    invalid_namespaces: HashMap<String, anyhow::Error>,
    engine: Arc<Engine>,
}

impl SkillStoreState {
    pub fn new(engine: Arc<Engine>, namespaces: &HashMap<String, NamespaceConfig>) -> Self {
        let skill_registries = namespaces
            .iter()
            .map(|(k, v)| (k.to_owned(), Self::registry(v)))
            .collect::<HashMap<_, _>>();
        SkillStoreState {
            known_skills: HashMap::new(),
            cached_skills: HashMap::new(),
            skill_registries,
            invalid_namespaces: HashMap::new(),
            engine,
        }
    }

    fn registry(namespace_config: &NamespaceConfig) -> Box<dyn SkillRegistry + Send> {
        match namespace_config.registry() {
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
        info!(
            "New or changed skill: {skill} with tag {}",
            tag.as_deref().unwrap_or("None")
        );
        if self.known_skills.insert(skill.clone(), tag).is_some() {
            self.invalidate(skill);
        }
    }

    pub fn remove_skill(&mut self, skill: &SkillPath) {
        info!("Removed skill: {skill}");
        self.known_skills.remove(skill);
        self.invalidate(skill);
    }

    pub fn skills(&self) -> impl Iterator<Item = &SkillPath> {
        self.known_skills.keys()
    }

    pub fn list_cached_skills(&self) -> impl Iterator<Item = &SkillPath> + '_ {
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
    pub async fn fetch(&mut self, skill_path: &SkillPath) -> anyhow::Result<Option<Arc<Skill>>> {
        if let Some(error) = self.invalid_namespaces.get(&skill_path.namespace) {
            return Err(anyhow!("Invalid namespace: {error}"));
        }

        let skill = if let Some(skill) = self.cached_skills.get(skill_path) {
            skill.skill.clone()
        } else {
            let Some(tag) = self.known_skills.get(skill_path) else {
                return Ok(None);
            };

            let registry = self
                .skill_registries
                .get(&skill_path.namespace)
                .expect("If skill exists, so must the namespace it resides in.");

            let skill_bytes = registry
                .load_skill(&skill_path.name, tag.as_deref().unwrap_or("latest"))
                .await?;
            let SkillImage { bytes, digest } = skill_bytes
                .ok_or_else(|| anyhow!("Skill {skill_path} configured but not loadable."))?;
            let engine = self.engine.clone();
            let skill = spawn_blocking(move || Skill::new(engine.as_ref(), bytes))
                .await
                .expect("Spawend linking thread must run to completion without being poisened.")
                .with_context(|| format!("Failed to initialize {skill_path}."))?;
            let skill = Arc::new(skill);
            self.cached_skills
                .insert(skill_path.clone(), CachedSkill::new(skill.clone(), digest));
            skill
        };

        Ok(Some(skill))
    }
}

pub struct SkillStore {
    sender: mpsc::Sender<SkillProviderMsg>,
    handle: JoinHandle<()>,
}

impl SkillStore {
    pub fn new(engine: Arc<Engine>, namespaces: &HashMap<String, NamespaceConfig>) -> Self {
        let (sender, recv) = mpsc::channel(1);
        let mut actor = SkillProviderActor::new(engine, recv, namespaces);
        let handle = tokio::spawn(async move {
            actor.run().await;
        });
        SkillStore { sender, handle }
    }

    pub fn api(&self) -> SkillStoreApi {
        SkillStoreApi::new(self.sender.clone())
    }

    pub async fn wait_for_shutdown(self) {
        // Drop sender so actor terminates, as soon as all api handles are freed.
        drop(self.sender);
        self.handle.await.unwrap();
    }
}

#[derive(Clone)]
pub struct SkillStoreApi {
    sender: mpsc::Sender<SkillProviderMsg>,
}

impl SkillStoreApi {
    pub fn new(sender: mpsc::Sender<SkillProviderMsg>) -> Self {
        SkillStoreApi { sender }
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
        self.sender
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
    }

    /// Fetch an exeutable skill
    pub async fn fetch(&self, skill_path: SkillPath) -> Result<Option<Arc<Skill>>, anyhow::Error> {
        let (send, recv) = oneshot::channel();
        let msg = SkillProviderMsg::Fetch { skill_path, send };
        self.sender
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }

    /// List all skills which are currently cached and can be executed without fetching the wasm
    /// component from an OCI
    pub async fn list_cached(&self) -> Vec<SkillPath> {
        let (send, recv) = oneshot::channel();
        let msg = SkillProviderMsg::ListCached { send };
        self.sender
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }

    /// List all skills from all namespaces
    pub async fn list(&self) -> Vec<SkillPath> {
        let (send, recv) = oneshot::channel();
        let msg = SkillProviderMsg::List { send };
        self.sender
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }

    /// Drops a skill from the cache in case it has been cached before. `true` if the skill has been
    /// in the cache before, `false` otherwise .
    pub async fn invalidate_cache(&self, skill_path: SkillPath) -> bool {
        let (send, recv) = oneshot::channel();
        let msg = SkillProviderMsg::InvalidateCache { skill_path, send };
        self.sender
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }
}

pub enum SkillProviderMsg {
    Fetch {
        skill_path: SkillPath,
        send: oneshot::Sender<Result<Option<Arc<Skill>>, anyhow::Error>>,
    },
    List {
        send: oneshot::Sender<Vec<SkillPath>>,
    },
    ListCached {
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
    },
    InvalidateCache {
        skill_path: SkillPath,
        send: oneshot::Sender<bool>,
    },
}

struct SkillProviderActor {
    receiver: mpsc::Receiver<SkillProviderMsg>,
    provider: SkillStoreState,
}

impl SkillProviderActor {
    pub fn new(
        engine: Arc<Engine>,
        receiver: mpsc::Receiver<SkillProviderMsg>,
        namespaces: &HashMap<String, NamespaceConfig>,
    ) -> Self {
        SkillProviderActor {
            receiver,
            provider: SkillStoreState::new(engine, namespaces),
        }
    }

    pub async fn run(&mut self) {
        while let Some(msg) = self.receiver.recv().await {
            self.act(msg).await;
        }
    }

    pub async fn act(&mut self, msg: SkillProviderMsg) {
        match msg {
            SkillProviderMsg::Fetch { skill_path, send } => {
                let result = self.provider.fetch(&skill_path).await;
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
            SkillProviderMsg::ListCached { send } => {
                drop(send.send(self.provider.list_cached_skills().cloned().collect()));
            }
            SkillProviderMsg::InvalidateCache { skill_path, send } => {
                let _ = send.send(self.provider.invalidate(&skill_path));
            }
        }
    }
}

#[cfg(test)]
pub mod tests {

    use test_skills::given_greet_skill;

    use super::*;

    pub use super::SkillProviderMsg;

    pub fn dummy_skill_provider_api() -> SkillStoreApi {
        let (send, _recv) = mpsc::channel(1);
        SkillStoreApi::new(send)
    }

    impl SkillStoreState {
        fn with_namespace_and_skill(engine: Arc<Engine>, skill_path: &SkillPath) -> Self {
            let registry = Registry::File {
                path: "skills".to_owned(),
            };
            let ns_cfg = NamespaceConfig::TeamOwned {
                config_url: "file://namespace.toml".to_owned(),
                config_access_token_env_var: None,
                registry,
            };
            let mut namespaces = HashMap::new();
            namespaces.insert(skill_path.namespace.clone(), ns_cfg);

            let mut provider = SkillStoreState::new(engine, &namespaces);
            provider.upsert_skill(skill_path, None);
            provider
        }
    }

    #[tokio::test]
    async fn skill_component_is_in_config() {
        let skill_path = SkillPath::dummy();
        let engine = Arc::new(Engine::new(false).unwrap());
        let mut provider = SkillStoreState::with_namespace_and_skill(engine, &skill_path);

        let result = provider.fetch(&skill_path).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn skill_component_not_in_config() {
        let skill_path = SkillPath::dummy();
        let engine = Arc::new(Engine::new(false).unwrap());
        let mut provider = SkillStoreState::with_namespace_and_skill(engine, &skill_path);

        let result = provider
            .fetch(&SkillPath::new(&skill_path.namespace, "non_existing_skill"))
            .await;

        assert!(matches!(result, Ok(None)));
    }

    #[tokio::test]
    async fn namespace_not_in_config() {
        let skill_path = SkillPath::dummy();
        let engine = Arc::new(Engine::new(false).unwrap());
        let mut provider = SkillStoreState::with_namespace_and_skill(engine, &skill_path);

        let result = provider
            .fetch(&SkillPath::new("non_existing_namespace", &skill_path.name))
            .await;

        assert!(matches!(result, Ok(None)));
    }

    #[tokio::test]
    async fn cached_skill_removed() {
        // Given one cached skill
        given_greet_skill();
        let skill_path = SkillPath::new("local", "greet_skill");
        let engine = Arc::new(Engine::new(false).unwrap());
        let mut provider = SkillStoreState::with_namespace_and_skill(engine, &skill_path);
        provider.fetch(&skill_path).await.unwrap();

        // When we remove the skill
        provider.remove_skill(&skill_path);

        // Then the skill is no longer cached
        assert!(provider.list_cached_skills().next().is_none());
    }

    #[tokio::test]
    async fn should_error_if_fetching_skill_from_invalid_namespace() {
        // given a skill in an invalid namespace
        let skill_path = SkillPath::new("local", "greet_skill");
        let engine = Arc::new(Engine::new(false).unwrap());
        let mut provider = SkillStoreState::with_namespace_and_skill(engine, &skill_path);
        provider.add_invalid_namespace(skill_path.namespace.clone(), anyhow!(""));

        // when fetching the skill
        let result = provider.fetch(&skill_path).await;

        // then it returns an error
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn should_only_cache_skills_that_have_been_fetched() {
        given_greet_skill();
        // Given local is a configured namespace, backed by a file repository with "greet_skill"
        // and "greet-py"
        let engine = Arc::new(Engine::new(false).unwrap());
        let skill_provider = SkillStore::new(engine, &local_namespace());
        skill_provider
            .api()
            .upsert(SkillPath::new("local", "greet_skill"), None)
            .await;
        skill_provider
            .api()
            .upsert(SkillPath::new("local", "greet-py"), None)
            .await;

        // When fetching "greet_skill" but not "greet-py"
        skill_provider
            .api()
            .fetch(SkillPath::new("local", "greet_skill"))
            .await
            .unwrap();
        // and listing all chached skills
        let cached_skills = skill_provider.api().list_cached().await;

        // Then only "greet_skill" will appear in that list, but not "greet-py"
        assert_eq!(cached_skills, vec![SkillPath::new("local", "greet_skill")]);

        // Cleanup
        skill_provider.wait_for_shutdown().await;
    }

    #[tokio::test]
    async fn should_list_skills_that_have_been_added() {
        // Given an empty provider
        let engine = Arc::new(Engine::new(false).unwrap());
        let skill_provider = SkillStore::new(engine, &local_namespace());
        let api = skill_provider.api();

        // When adding a skill
        api.upsert(SkillPath::new("local", "one"), None).await;
        api.upsert(SkillPath::new("local", "two"), None).await;
        let skills = api.list().await;

        // Then the skills is listed by the skill executor api
        assert_eq!(skills.len(), 2);
        assert!(skills.contains(&SkillPath::new("local", "one")));
        assert!(skills.contains(&SkillPath::new("local", "two")));

        // Cleanup
        drop(api);
        skill_provider.wait_for_shutdown().await;
    }

    #[tokio::test]
    async fn should_remove_invalidated_skill_from_cache() {
        // Given one cached "greet_skill"
        given_greet_skill();
        let greet_skill = SkillPath::new("local", "greet_skill");
        let engine = Arc::new(Engine::new(false).unwrap());
        let skill_provider = SkillStore::new(engine, &local_namespace());
        let api = skill_provider.api();
        api.upsert(greet_skill.clone(), None).await;
        api.fetch(greet_skill.clone()).await.unwrap();

        // When we invalidate "greet_skill"
        let skill_had_been_in_cache = api.invalidate_cache(greet_skill.clone()).await;

        // Then greet skill is no longer listed in the cache, but of course still available in the
        // list of all skills
        assert!(skill_had_been_in_cache);
        assert!(api.list_cached().await.is_empty());
        assert_eq!(api.list().await, vec![greet_skill]);
    }

    #[tokio::test]
    async fn invalidation_of_an_uncached_skill() {
        // Given one "greet_skill" which is not in cache
        let greet_skill = SkillPath::new("local", "greet_skill");
        let engine = Arc::new(Engine::new(false).unwrap());
        let skill_provider = SkillStore::new(engine, &local_namespace());
        let api = skill_provider.api();
        api.upsert(greet_skill.clone(), None).await;

        // When we invalidate "greet_skill"
        let skill_had_been_in_cache = api.invalidate_cache(greet_skill.clone()).await;

        // Then greet skill is of course still available in the list of all skills. The return value
        // indicates that greet skill never had been in the cache to begin with
        assert!(!skill_had_been_in_cache);
        assert_eq!(api.list().await, vec![greet_skill]);
    }

    /// Namespace named local backed by a file registry with "skills" directory
    fn local_namespace() -> HashMap<String, NamespaceConfig> {
        let namespace_cfg = NamespaceConfig::TeamOwned {
            config_url: "file://namespace.toml".to_owned(),
            config_access_token_env_var: None,
            registry: Registry::File {
                path: "./skills".to_owned(),
            },
        };
        std::iter::once(("local".to_owned(), namespace_cfg)).collect()
    }
}
