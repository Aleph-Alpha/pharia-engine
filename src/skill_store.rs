use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::anyhow;
use tokio::{
    select,
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::{sleep_until, Instant},
};
use tracing::{error, info};

use crate::{
    registries::Digest,
    skill_loader::SkillLoaderApi,
    skills::{Skill, SkillPath},
};

/// A wrapper around the Skill to also keep track of which
/// digest was loaded last and when it was last checked.
struct CachedSkill {
    /// Compiled and pre initialized skill
    skill: Arc<Skill>,
    /// Digest of the skill when it was last loaded from the registry
    digest: Digest,
    /// When we last checked the digest
    digest_validated: Instant,
}

impl CachedSkill {
    fn new(skill: Arc<Skill>, digest: Digest) -> Self {
        Self {
            skill,
            digest,
            digest_validated: Instant::now(),
        }
    }
}

struct SkillStoreState {
    known_skills: HashMap<SkillPath, Option<String>>,
    cached_skills: HashMap<SkillPath, CachedSkill>,
    // key: Namespace, value: Error
    invalid_namespaces: HashMap<String, anyhow::Error>,
    skill_loader: SkillLoaderApi,
}

impl SkillStoreState {
    pub fn new(skill_loader: SkillLoaderApi) -> Self {
        SkillStoreState {
            known_skills: HashMap::new(),
            cached_skills: HashMap::new(),
            invalid_namespaces: HashMap::new(),
            skill_loader,
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
            let (skill, digest) = self
                .skill_loader
                .fetch(skill_path.clone(), tag.to_owned())
                .await?;
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

    /// Insert a skill into the cache.
    pub fn insert(&mut self, skill_path: SkillPath, skill: Arc<Skill>, digest: Digest) {
        self.cached_skills
            .insert(skill_path, CachedSkill::new(skill.clone(), digest));
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
    /// If the digest behind the tag has changed, remove the cache entry. Otherwise, update the validation time.
    async fn validate_digest(&mut self, skill_path: SkillPath) -> anyhow::Result<()> {
        let CachedSkill { digest, .. } = self
            .cached_skills
            .get(&skill_path)
            .ok_or_else(|| anyhow!("Missing cached skill for {skill_path}"))?;

        let tag = self
            .tag(&skill_path)
            .expect("Bad namespace configuration for skill")
            .expect("Missing tag for skill");

        match self
            .skill_loader
            .fetch_digest(skill_path.clone(), tag.to_owned())
            .await
        {
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
    sender: mpsc::Sender<SkillStoreMessage>,
    handle: JoinHandle<()>,
}

impl SkillStore {
    pub fn new(skill_loader: SkillLoaderApi, digest_update_interval: Duration) -> Self {
        let (sender, recv) = mpsc::channel(1);
        let mut actor = SkillStoreActor::new(recv, skill_loader, digest_update_interval);
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
    sender: mpsc::Sender<SkillStoreMessage>,
}

impl SkillStoreApi {
    pub fn new(sender: mpsc::Sender<SkillStoreMessage>) -> Self {
        SkillStoreApi { sender }
    }

    pub async fn remove(&self, skill_path: SkillPath) {
        let msg = SkillStoreMessage::Remove { skill_path };
        self.sender
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
    }

    pub async fn upsert(&self, skill_path: SkillPath, tag: Option<String>) {
        let msg = SkillStoreMessage::Upsert { skill_path, tag };
        self.sender
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
    }

    /// Report a namespace as erroneous (e.g. in case its configuration is messed up). Set `None`
    /// to communicate that a namespace is no longer erroneous.
    pub async fn set_namespace_error(&self, namespace: String, error: Option<anyhow::Error>) {
        let msg = SkillStoreMessage::SetNamespaceError { namespace, error };
        self.sender
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
    }

    /// Fetch an executable skill
    pub async fn fetch(&self, skill_path: SkillPath) -> Result<Option<Arc<Skill>>, anyhow::Error> {
        let (send, recv) = oneshot::channel();
        let msg = SkillStoreMessage::Fetch { skill_path, send };
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
        let msg = SkillStoreMessage::ListCached { send };
        self.sender
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }

    /// List all skills from all namespaces
    pub async fn list(&self) -> Vec<SkillPath> {
        let (send, recv) = oneshot::channel();
        let msg = SkillStoreMessage::List { send };
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
        let msg = SkillStoreMessage::InvalidateCache { skill_path, send };
        self.sender
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }
}

pub enum SkillStoreMessage {
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
    receiver: mpsc::Receiver<SkillStoreMessage>,
    provider: SkillStoreState,
    digest_update_interval: Duration,
}

impl SkillStoreActor {
    pub fn new(
        receiver: mpsc::Receiver<SkillStoreMessage>,
        skill_loader: SkillLoaderApi,
        digest_update_interval: Duration,
    ) -> Self {
        SkillStoreActor {
            receiver,
            provider: SkillStoreState::new(skill_loader),
            digest_update_interval,
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
                // While we are waiting on messages, refresh the digests behind the tags. If a new message
                // comes in while we are mid-request, it will get cancelled, but since we want to prioritize
                // processing messages, doing some duplicate requests is ok.
                //
                // The if experessions makes sure we only poll if there are cached skills so that `select`
                // will only wait on messages until we have a cache.
                result = self.provider.refresh_oldest_digest(self.digest_update_interval), if self.provider.list_cached_skills().next().is_some() => {
                    if let Err(e) = result {
                        error!("Error refreshing digest: {e}");
                    }
                },
            }
        }
    }

    pub async fn act(&mut self, msg: SkillStoreMessage) {
        match msg {
            SkillStoreMessage::Fetch { skill_path, send } => {
                drop(send.send(self.provider.fetch(&skill_path).await));
            }
            SkillStoreMessage::List { send } => {
                drop(send.send(self.provider.skills().cloned().collect()));
            }
            SkillStoreMessage::Remove { skill_path } => {
                self.provider.remove_skill(&skill_path);
            }
            SkillStoreMessage::Upsert { skill_path, tag } => {
                self.provider.upsert_skill(&skill_path, tag);
            }
            SkillStoreMessage::SetNamespaceError { namespace, error } => {
                if let Some(error) = error {
                    self.provider.add_invalid_namespace(namespace, error);
                } else {
                    self.provider.remove_invalid_namespace(&namespace);
                }
            }
            SkillStoreMessage::ListCached { send } => {
                drop(send.send(self.provider.list_cached_skills().cloned().collect()));
            }
            SkillStoreMessage::InvalidateCache { skill_path, send } => {
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

    use crate::skill_loader::{RegistryConfig, SkillLoader};
    use crate::skills::{Engine, SkillPath};

    use super::*;

    pub use super::SkillStoreMessage;

    pub fn dummy_skill_store_api() -> SkillStoreApi {
        let (send, _recv) = mpsc::channel(1);
        SkillStoreApi::new(send)
    }

    impl SkillStoreState {
        fn with_namespace_and_skill(engine: Arc<Engine>, skill_path: &SkillPath) -> Self {
            let skill_loader =
                SkillLoader::with_file_registry(engine, skill_path.namespace.clone()).api();

            let mut provider = SkillStoreState::new(skill_loader);
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
        let provider = SkillStoreState::with_namespace_and_skill(engine, &skill_path);

        let result = provider.tag(&SkillPath::new(&skill_path.namespace, "non_existing_skill"));
        assert!(matches!(result, Ok(None)));
    }

    #[tokio::test]
    async fn namespace_not_in_config() {
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

    #[tokio::test]
    async fn should_error_if_fetching_skill_from_invalid_namespace() {
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
        let skill_loader = SkillLoader::with_file_registry(engine, "local".to_owned()).api();
        let skill_store = SkillStore::new(skill_loader, Duration::from_secs(10));
        skill_store
            .api()
            .upsert(SkillPath::new("local", "greet_skill"), None)
            .await;
        skill_store
            .api()
            .upsert(SkillPath::new("local", "greet-py"), None)
            .await;

        // When fetching "greet_skill" but not "greet-py"
        skill_store
            .api()
            .fetch(SkillPath::new("local", "greet_skill"))
            .await
            .unwrap();
        // and listing all cached skills
        let cached_skills = skill_store.api().list_cached().await;

        // Then only "greet_skill" will appear in that list, but not "greet-py"
        assert_eq!(cached_skills, vec![SkillPath::new("local", "greet_skill")]);

        // Cleanup
        skill_store.wait_for_shutdown().await;
    }

    #[tokio::test]
    async fn should_list_skills_that_have_been_added() {
        // Given an empty provider
        let engine = Arc::new(Engine::new(false).unwrap());
        let skill_loader = SkillLoader::with_file_registry(engine, "local".to_owned()).api();
        let skill_store = SkillStore::new(skill_loader, Duration::from_secs(10));
        let api = skill_store.api();

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
        skill_store.wait_for_shutdown().await;
    }

    #[tokio::test]
    async fn should_remove_invalidated_skill_from_cache() {
        // Given one cached "greet_skill"
        given_greet_skill();
        let greet_skill = SkillPath::new("local", "greet_skill");
        let engine = Arc::new(Engine::new(false).unwrap());
        let skill_loader = SkillLoader::with_file_registry(engine, "local".to_owned()).api();
        let skill_store = SkillStore::new(skill_loader, Duration::from_secs(10));
        let api = skill_store.api();
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
        let skill_loader = SkillLoader::with_file_registry(engine, "local".to_owned()).api();
        let skill_store = SkillStore::new(skill_loader, Duration::from_secs(10));
        let api = skill_store.api();
        api.upsert(greet_skill.clone(), None).await;

        // When we invalidate "greet_skill"
        let skill_had_been_in_cache = api.invalidate_cache(greet_skill.clone()).await;

        // Then greet skill is of course still available in the list of all skills. The return value
        // indicates that greet skill never had been in the cache to begin with
        assert!(!skill_had_been_in_cache);
        assert_eq!(api.list().await, vec![greet_skill]);
        // Cleanup
        drop(api);
        skill_store.wait_for_shutdown().await;
    }

    #[tokio::test]
    async fn invalidate_cache_skills_after_digest_change() -> anyhow::Result<()> {
        // Given one cached "greet_skill"
        let (registry_config, temp_dir) = tmp_registries_with_skill()?;
        let engine = Arc::new(Engine::new(false)?);
        let skill_loader = SkillLoader::new(engine, registry_config).api();
        let mut skill_store_state = SkillStoreState::new(skill_loader);

        let greet_skill = SkillPath::new("local", "greet_skill");
        skill_store_state.upsert_skill(&greet_skill, None);
        skill_store_state.fetch(&greet_skill).await?;
        assert_eq!(
            skill_store_state.list_cached_skills().collect::<Vec<_>>(),
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

        skill_store_state
            .validate_digest(greet_skill.clone())
            .await?;

        // Then greet skill is no longer listed in the cache, but of course still available in the
        // list of all skills
        assert_eq!(skill_store_state.list_cached_skills().count(), 0);
        assert_eq!(
            skill_store_state.skills().collect::<Vec<_>>(),
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
        let skill_loader = SkillLoader::with_file_registry(engine, "local".to_owned()).api();
        let mut skill_store_state = SkillStoreState::new(skill_loader);

        skill_store_state.upsert_skill(&greet_skill, None);
        skill_store_state.fetch(&greet_skill).await?;

        // When we check it with no changes
        skill_store_state.validate_digest(greet_skill).await?;

        // Then nothing changes
        assert_eq!(
            skill_store_state.list_cached_skills().collect::<Vec<_>>(),
            skill_store_state.skills().collect::<Vec<_>>(),
        );

        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn should_invalidate_cached_skill_whose_digest_has_changed() -> anyhow::Result<()> {
        // Given one cached "greet_skill"
        let (registry_config, temp_dir) = tmp_registries_with_skill()?;
        let greet_skill = SkillPath::new("local", "greet_skill");
        let engine = Arc::new(Engine::new(false)?);
        let skill_loader = SkillLoader::new(engine, registry_config).api();
        let skill_store = SkillStore::new(skill_loader, Duration::from_secs(1));
        let api = skill_store.api();
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
        skill_store.wait_for_shutdown().await;

        Ok(())
    }

    /// Creates a file registry in a tempdir with one greet skill
    fn tmp_registries_with_skill() -> anyhow::Result<(RegistryConfig, TempDir)> {
        given_greet_skill();
        let dir = tempfile::tempdir()?;
        fs::copy(
            "./skills/greet_skill.wasm",
            dir.path().join("greet_skill.wasm"),
        )?;

        let path = dir.path().to_string_lossy().into();
        let registry_config = RegistryConfig::with_file_registry("local".to_owned(), path);
        Ok((registry_config, dir))
    }
}
