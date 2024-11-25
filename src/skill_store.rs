use anyhow::{anyhow, Context};
use futures::stream::FuturesUnordered;
use std::{collections::HashMap, sync::Arc, time::Duration};
use std::{future::Future, pin::Pin};
use tokio::{
    select,
    sync::{mpsc, oneshot},
    task::{spawn_blocking, JoinHandle},
    time::{sleep_until, Instant},
};
use tracing::{error, info};

use crate::{
    namespace_watcher::{NamespaceConfig, Registry},
    registries::{FileRegistry, OciRegistry, SkillImage, SkillRegistry},
    skills::{Engine, Skill, SkillPath},
};
use futures::StreamExt;

/// A wrapper around the Skill to also keep track of which
/// digest was loaded last and when it was last checked.
struct CachedSkill {
    /// Compiled and pre initialized skill
    skill: Arc<Skill>,
    /// Digest of the skill when it was last loaded from the registry
    digest: String,
    /// When we last checked the digest
    digest_validated: Instant,
}

impl CachedSkill {
    fn new(skill: Arc<Skill>, digest: impl Into<String>) -> Self {
        Self {
            skill,
            digest: digest.into(),
            digest_validated: Instant::now(),
        }
    }
}

struct SkillStoreState {
    known_skills: HashMap<SkillPath, Option<String>>,
    cached_skills: HashMap<SkillPath, CachedSkill>,
    // key: Namespace, value: Registry
    skill_registries: HashMap<String, Box<dyn SkillRegistry + Send + Sync>>,
    // key: Namespace, value: Error
    invalid_namespaces: HashMap<String, anyhow::Error>,
    engine: Arc<Engine>,
}

impl SkillStoreState {
    pub fn new(
        engine: Arc<Engine>,
        skill_registries: HashMap<String, Box<dyn SkillRegistry + Send + Sync>>,
    ) -> Self {
        SkillStoreState {
            known_skills: HashMap::new(),
            cached_skills: HashMap::new(),
            skill_registries,
            invalid_namespaces: HashMap::new(),
            engine,
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

    /// Fetch a skill. If the skill is present in the cache, return it. Otherwise, fetch it from the
    /// registry and cache it.
    pub async fn fetch(&mut self, skill_path: &SkillPath) -> anyhow::Result<Option<Arc<Skill>>> {
        if let Some(skill) = self.cached_skill(skill_path) {
            return Ok(Some(skill));
        }
        if let Some(tag) = self.tag(skill_path)? {
            let (skill, digest) = self.load_and_build(skill_path, tag).await?;
            let skill = Arc::new(skill);
            self.insert(skill_path.clone(), skill.clone(), digest);
            Ok(Some(skill))
        } else {
            Ok(None)
        }
    }

    /// `Some` if the skill is present in the cache, `None` if not
    pub fn cached_skill(&self, skill_path: &SkillPath) -> Option<Arc<Skill>> {
        self.cached_skills
            .get(skill_path)
            .map(|skill| skill.skill.clone())
    }

    /// Load a skill from the registry and build it to a `Skill`
    pub async fn load_and_build(
        &self,
        skill_path: &SkillPath,
        tag: &str,
    ) -> anyhow::Result<(Skill, String)> {
        let registry = self
            .skill_registries
            .get(&skill_path.namespace)
            .expect("If skill exists, so must the namespace it resides in.");

        let skill_bytes = registry.load_skill(&skill_path.name, tag).await?;
        let SkillImage { bytes, digest } = skill_bytes
            .ok_or_else(|| anyhow!("Skill {skill_path} configured but not loadable."))?;
        let engine = self.engine.clone();
        let skill = spawn_blocking(move || Skill::new(engine.as_ref(), bytes))
            .await
            .expect("Spawend linking thread must run to completion without being poisened.")
            .with_context(|| format!("Failed to initialize {skill_path}."))?;
        Ok((skill, digest))
    }

    /// Insert a skill into the cache.
    pub fn insert(&mut self, skill_path: SkillPath, skill: Arc<Skill>, digest: String) {
        self.cached_skills
            .insert(skill_path, CachedSkill::new(skill.clone(), digest));
    }

    /// Return to corresponding registry for a given skill path
    fn registry_for_skill(&self, skill_path: &SkillPath) -> Option<&(impl SkillRegistry + use<>)> {
        self.skill_registries.get(&skill_path.namespace)
    }

    /// Return the registered tag for a given skill
    fn tag(&self, skill_path: &SkillPath) -> anyhow::Result<Option<&str>> {
        if let Some(error) = self.invalid_namespaces.get(&skill_path.namespace) {
            return Err(anyhow!("Invalid namespace: {error}"));
        }
        Ok(self
            .known_skills
            .get(skill_path)
            .map(|o| o.as_deref().unwrap_or("latest")))
    }

    /// Retrieve the oldest digest validation timestamp. So, the one we would need to refresh the soonest.
    /// If there are no cached skills, it will return `None`.
    fn oldest_digest(&self) -> Option<(SkillPath, Instant)> {
        self.cached_skills
            .iter()
            .min_by_key(|(_, c)| c.digest_validated)
            .map(|(skill_path, cached_skill)| (skill_path.clone(), cached_skill.digest_validated))
    }

    /// Compares the digest in the cache with the digest behind the corresponding tag in the registry.
    /// If the digest behind the tag has changed, remove the cache entry. Otherwise, upate the validation time.
    async fn validate_digest(&mut self, skill_path: SkillPath) -> anyhow::Result<()> {
        let CachedSkill { digest, .. } = self
            .cached_skills
            .get(&skill_path)
            .ok_or_else(|| anyhow!("Missing cached skill for {skill_path}"))?;
        let registry = self
            .registry_for_skill(&skill_path)
            .expect("Missing registry for skill");
        let tag = self
            .tag(&skill_path)
            .expect("Bad namespace configuration for skill")
            .expect("Missing tag for skill");

        match registry.fetch_digest(&skill_path.name, tag).await {
            // There is a new digest behind the tag, delete the cache entry
            Ok(Some(new_digest)) if &new_digest != digest => {
                self.cached_skills.remove(&skill_path);
                Ok(())
            }
            other_case => {
                let result = match other_case {
                    // Errors fetching the digest
                    Ok(None) => Err(anyhow!("Missing digest for skill {skill_path}")),
                    Err(e) => Err(e),
                    // Digest has not changed
                    _ => Ok(()),
                };
                // Always update the digest_validated time so we don't loop over errors constantly.
                self.cached_skills
                    .entry(skill_path)
                    .and_modify(|c| c.digest_validated = Instant::now());

                result
            }
        }
    }

    /// Finds the next cached skill we should refresh to see if the backing digest has changed,
    /// in which case it will remove it from the cache or update the timestamp for when it needs to
    /// be checked again.
    ///
    /// It will check only after the update interval has passed since it was last checked.
    ///
    /// This will return immediately if there are no skills to check.
    async fn refresh_oldest_digest(&mut self, update_interval: Duration) -> anyhow::Result<()> {
        // Find the next skill we should refresh
        let Some((skill_path, last_checked)) = self.oldest_digest() else {
            return Ok(());
        };
        // Wait until is it time to refresh
        sleep_until(last_checked + update_interval).await;
        self.validate_digest(skill_path).await
    }
}

pub struct SkillStore {
    sender: mpsc::Sender<SkillProviderMsg>,
    handle: JoinHandle<()>,
}

impl SkillStore {
    pub fn new(
        engine: Arc<Engine>,
        skill_registries: HashMap<String, Box<dyn SkillRegistry + Send + Sync>>,
        digest_update_interval: Duration,
    ) -> Self {
        let (sender, recv) = mpsc::channel(1);
        let mut actor =
            SkillStoreActor::new(engine, recv, skill_registries, digest_update_interval);
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

struct SkillStoreActor {
    receiver: mpsc::Receiver<SkillProviderMsg>,
    provider: SkillStoreState,
    digest_update_interval: Duration,
    running_requests: FuturesUnordered<Pin<Box<dyn Future<Output = ()> + Send>>>,
}

impl From<&NamespaceConfig> for Box<dyn SkillRegistry + Send + Sync> {
    fn from(val: &NamespaceConfig) -> Self {
        match val.registry() {
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

impl SkillStoreActor {
    pub fn new(
        engine: Arc<Engine>,
        receiver: mpsc::Receiver<SkillProviderMsg>,
        skill_registries: HashMap<String, Box<dyn SkillRegistry + Send + Sync>>,
        digest_update_interval: Duration,
    ) -> Self {
        SkillStoreActor {
            receiver,
            provider: SkillStoreState::new(engine, skill_registries),
            digest_update_interval,
            running_requests: FuturesUnordered::new(),
        }
    }

    pub async fn run(&mut self) {
        loop {
            select! {
                msg = self.receiver.recv() => match msg {
                    Some(msg) => self.act(msg).await,
                    // Senders are gone, break out of the loop for shutdown.
                    None => break
                },
                // FuturesUnordered will let them run in parallel. It will
                // yield once one of them is completed.
                () = self.running_requests.select_next_some(), if !self.running_requests.is_empty()  => {}
                // While we are waiting on messages, refresh the digests behind the tags. If a new message
                // comes in while we are mid-request, it will get cancelled, but since we want to prioritize
                // processing messages, doing some duplicate requests is ok.
                //
                // The if expressions makes sure we only poll if there are cached skills so that `select`
                // will only wait on messages until we have a cache.
                result = self.provider.refresh_oldest_digest(self.digest_update_interval), if self.provider.list_cached_skills().next().is_some() => {
                    if let Err(e) = result {
                        error!("Error refreshing digest: {e}");
                    }
                },
            }
        }
    }

    pub async fn act(&mut self, msg: SkillProviderMsg) {
        match msg {
            SkillProviderMsg::Fetch { skill_path, send } => {
                drop(send.send(self.provider.fetch(&skill_path).await));
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

    use std::fs;

    use tempfile::TempDir;
    use test_skills::{given_chat_skill, given_greet_skill};
    use tokio::time::sleep;

    use crate::namespace_watcher::tests::SkillDescription;
    use crate::registries::tests::NeverResolvingRegistry;

    use super::*;

    pub use super::SkillProviderMsg;
    use tokio::time::{self, Duration};

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

            let mut registries = HashMap::new();
            registries.insert(skill_path.namespace.clone(), (&ns_cfg).into());

            let mut provider = SkillStoreState::new(engine, registries);
            provider.upsert_skill(skill_path, None);
            provider
        }
    }

    #[tokio::test]
    async fn loading_skill_does_not_block_skill_store() {
        // Given a skill store with a registry that never resolves
        let registry = Box::new(NeverResolvingRegistry) as Box<dyn SkillRegistry + Send + Sync>;
        let skill_path = SkillPath::new("local", "greet_skill");
        let registries = std::iter::once(("local".to_owned(), registry)).collect();
        let engine = Arc::new(Engine::new(false).unwrap());
        let skill_store = SkillStore::new(engine, registries, Duration::from_secs(10)).api();

        // When fetching a skill which is in the known skills
        skill_store.upsert(skill_path.clone(), None).await;
        let cloned_skill_store = skill_store.clone();
        let handle = tokio::spawn(async move {
            let result = cloned_skill_store.fetch(skill_path).await;
            assert!(result.unwrap().is_none());
        });

        // and waiting 10ms to ensure the fetch has been received
        sleep(Duration::from_millis(10)).await;

        // Then the skill store can still be served other requests within reasonable time
        let result = time::timeout(Duration::from_millis(20), skill_store.list()).await;
        assert_eq!(result.unwrap().len(), 1);

        drop(handle);
    }

    #[tokio::test]
    async fn skill_component_is_in_config() {
        let skill_path = SkillPath::dummy();
        let engine = Arc::new(Engine::new(false).unwrap());
        let mut provider = SkillStoreState::with_namespace_and_skill(engine, &skill_path);

        let result = provider.fetch(&skill_path).await;

        assert!(result.is_err());
    }

    #[test]
    fn skill_component_not_in_config() {
        let skill_path = SkillPath::dummy();
        let engine = Arc::new(Engine::new(false).unwrap());
        let provider = SkillStoreState::with_namespace_and_skill(engine, &skill_path);

        let result = provider.tag(&SkillPath::new(&skill_path.namespace, "non_existing_skill"));
        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn namespace_not_in_config() {
        let skill_path = SkillPath::dummy();
        let engine = Arc::new(Engine::new(false).unwrap());
        let provider = SkillStoreState::with_namespace_and_skill(engine, &skill_path);

        let result = provider.tag(&SkillPath::new("non_existing_namespace", &skill_path.name));

        assert!(matches!(result, Ok(None)));
    }

    #[tokio::test]
    async fn cached_skill_removed() {
        // Given one cached skill
        given_greet_skill();
        let skill_path = SkillPath::new("local", "greet_skill");
        let engine = Arc::new(Engine::new(false).unwrap());
        let mut provider = SkillStoreState::with_namespace_and_skill(engine, &skill_path);
        provider.fetch(&skill_path).await.unwrap().unwrap();

        // When we remove the skill
        provider.remove_skill(&skill_path);

        // Then the skill is no longer cached
        assert!(provider.list_cached_skills().next().is_none());
    }

    #[test]
    fn should_error_if_fetching_skill_from_invalid_namespace() {
        // given a skill in an invalid namespace
        let skill_path = SkillPath::new("local", "greet_skill");
        let engine = Arc::new(Engine::new(false).unwrap());
        let mut provider = SkillStoreState::with_namespace_and_skill(engine, &skill_path);
        provider.add_invalid_namespace(skill_path.namespace.clone(), anyhow!(""));

        // when fetching the tag
        let result = provider.tag(&skill_path);

        // then it returns an error
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn should_only_cache_skills_that_have_been_fetched() {
        given_greet_skill();
        // Given local is a configured namespace, backed by a file repository with "greet_skill"
        // and "greet-py"
        let engine = Arc::new(Engine::new(false).unwrap());
        let skill_provider = SkillStore::new(engine, local_registries(), Duration::from_secs(10));
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
        let skill_provider = SkillStore::new(engine, local_registries(), Duration::from_secs(10));
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
        let skill_provider = SkillStore::new(engine, local_registries(), Duration::from_secs(10));
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
        let skill_provider = SkillStore::new(engine, local_registries(), Duration::from_secs(10));
        let api = skill_provider.api();
        api.upsert(greet_skill.clone(), None).await;

        // When we invalidate "greet_skill"
        let skill_had_been_in_cache = api.invalidate_cache(greet_skill.clone()).await;

        // Then greet skill is of course still available in the list of all skills. The return value
        // indicates that greet skill never had been in the cache to begin with
        assert!(!skill_had_been_in_cache);
        assert_eq!(api.list().await, vec![greet_skill]);
        // Cleanup
        drop(api);
        skill_provider.wait_for_shutdown().await;
    }

    /// Namespace named local backed by a file registry with "skills" directory
    fn local_registries() -> HashMap<String, Box<dyn SkillRegistry + Send + Sync>> {
        let namespace_cfg = NamespaceConfig::TeamOwned {
            config_url: "file://namespace.toml".to_owned(),
            config_access_token_env_var: None,
            registry: Registry::File {
                path: "./skills".to_owned(),
            },
        };
        let registry = (&namespace_cfg).into();
        std::iter::once(("local".to_owned(), registry)).collect()
    }

    #[tokio::test]
    async fn invalidate_cache_skills_after_digest_change() -> anyhow::Result<()> {
        // Given one cached "greet_skill"
        let (registries, temp_dir) = tmp_registries_with_skill()?;
        let greet_skill = SkillPath::new("local", "greet_skill");
        let engine = Arc::new(Engine::new(false)?);
        let mut skill_provider = SkillStoreState::new(engine, registries);
        skill_provider.upsert_skill(&greet_skill, None);
        skill_provider.fetch(&greet_skill).await?;
        assert_eq!(
            skill_provider.list_cached_skills().collect::<Vec<_>>(),
            vec![&greet_skill]
        );

        // When we update "greet_skill"s contents and we clear out expired skills
        let wasm_file = temp_dir.path().join("greet_skill.wasm");
        let prev_time = fs::metadata(&wasm_file)?.modified()?;
        // Another skill so we can copy it over
        given_chat_skill();
        fs::copy("./skills/chat_skill.wasm", &wasm_file)?;
        let new_time = fs::metadata(&wasm_file)?.modified()?;
        assert_ne!(prev_time, new_time);

        skill_provider.validate_digest(greet_skill.clone()).await?;

        // Then greet skill is no longer listed in the cache, but of course still available in the
        // list of all skills
        assert_eq!(skill_provider.list_cached_skills().count(), 0);
        assert_eq!(
            skill_provider.skills().collect::<Vec<_>>(),
            vec![&greet_skill]
        );

        Ok(())
    }

    #[tokio::test]
    async fn doesnt_invalidate_unchanged_digests() -> anyhow::Result<()> {
        // Given one cached "greet_skill"
        given_greet_skill();
        let greet_skill = SkillPath::new("local", "greet_skill");
        let engine = Arc::new(Engine::new(false)?);
        let mut skill_provider = SkillStoreState::new(engine, local_registries());
        skill_provider.upsert_skill(&greet_skill, None);
        skill_provider.fetch(&greet_skill).await?;

        // When we check it with no changes
        skill_provider.validate_digest(greet_skill).await?;

        // Then nothing changes
        assert_eq!(
            skill_provider.list_cached_skills().collect::<Vec<_>>(),
            skill_provider.skills().collect::<Vec<_>>(),
        );

        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn should_invalidate_cached_skill_whose_digest_has_changed() -> anyhow::Result<()> {
        // Given one cached "greet_skill"
        let (registries, temp_dir) = tmp_registries_with_skill()?;
        let greet_skill = SkillPath::new("local", "greet_skill");
        let engine = Arc::new(Engine::new(false)?);
        let skill_provider = SkillStore::new(engine, registries, Duration::from_secs(1));
        let api = skill_provider.api();
        api.upsert(greet_skill.clone(), None).await;
        api.fetch(greet_skill.clone()).await?;
        assert_eq!(api.list_cached().await, vec![greet_skill.clone()]);

        // When we update "greet_skill"s contents
        let wasm_file = temp_dir.path().join("greet_skill.wasm");
        let prev_time = fs::metadata(&wasm_file)?.modified()?;
        // Another skill so we can copy it over
        given_chat_skill();
        fs::copy("./skills/chat_skill.wasm", &wasm_file)?;
        let new_time = fs::metadata(&wasm_file)?.modified()?;
        assert_ne!(prev_time, new_time);
        sleep(Duration::from_secs(2)).await;

        // Then greet skill is no longer listed in the cache, but of course still available in the
        // list of all skills
        assert!(api.list_cached().await.is_empty());
        assert_eq!(api.list().await, vec![greet_skill]);

        // Cleanup
        drop(api);
        skill_provider.wait_for_shutdown().await;

        Ok(())
    }

    /// Creates a file registry in a tempdir with one greet skill
    fn tmp_registries_with_skill() -> anyhow::Result<(
        HashMap<String, Box<dyn SkillRegistry + Send + Sync>>,
        TempDir,
    )> {
        given_greet_skill();
        let dir = tempfile::tempdir()?;
        fs::copy(
            "./skills/greet_skill.wasm",
            dir.path().join("greet_skill.wasm"),
        )?;

        let namespace_cfg = NamespaceConfig::InPlace {
            skills: vec![SkillDescription {
                name: "greet_skill".to_owned(),
                tag: None,
            }],
            registry: Registry::File {
                path: dir.path().to_string_lossy().into(),
            },
        };
        Ok((
            std::iter::once(("local".to_owned(), (&namespace_cfg).into())).collect(),
            dir,
        ))
    }
}
