use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc, time::Duration};

use crate::skill_configuration::SkillConfigurationApi;
use crate::{
    registries::Digest,
    skill_loader::{ConfiguredSkill, SkillLoaderApi},
    skills::{Skill, SkillPath},
};
use anyhow::anyhow;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use tokio::{
    select,
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::{sleep_until, Instant},
};
use tracing::error;

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
    configuration: SkillConfigurationApi,
    cached_skills: HashMap<SkillPath, CachedSkill>,
    skill_loader: SkillLoaderApi,
}

impl SkillStoreState {
    pub fn new(skill_loader: SkillLoaderApi, configuration: SkillConfigurationApi) -> Self {
        SkillStoreState {
            configuration,
            cached_skills: HashMap::new(),
            skill_loader,
        }
    }

    /// All configured skills, sorted by namespace and name
    pub async fn skills(&self) -> Vec<SkillPath> {
        self.configuration.skills().await
    }

    pub fn list_cached_skills(&self) -> impl Iterator<Item = &SkillPath> + '_ {
        self.cached_skills.keys()
    }

    pub fn invalidate(&mut self, skill_path: &SkillPath) -> bool {
        self.cached_skills.remove(skill_path).is_some()
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
    async fn tag(&self, skill_path: &SkillPath) -> anyhow::Result<Option<String>> {
        self.configuration.tag(skill_path).await
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
            .configuration
            .tag(&skill_path)
            .await
            .expect("Bad namespace configuration for skill")
            .expect("Missing tag for skill");

        let skill = ConfiguredSkill::new(&skill_path.namespace, &skill_path.name, tag);
        match self.skill_loader.fetch_digest(skill).await {
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
    pub fn new(
        skill_loader: SkillLoaderApi,
        configuration: SkillConfigurationApi,
        digest_update_interval: Duration,
    ) -> Self {
        let (sender, recv) = mpsc::channel(1);
        let mut actor =
            SkillStoreActor::new(recv, skill_loader, configuration, digest_update_interval);
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
    InvalidateCache {
        skill_path: SkillPath,
        send: oneshot::Sender<bool>,
    },
}

struct SkillStoreActor {
    receiver: mpsc::Receiver<SkillStoreMessage>,
    provider: SkillStoreState,
    digest_update_interval: Duration,
    skill_requests: SkillRequests,
}

type SkillRequest =
    Pin<Box<dyn Future<Output = (SkillPath, Result<(Skill, Digest), anyhow::Error>)> + Send>>;

type Recipient = oneshot::Sender<Result<Option<Arc<Skill>>, anyhow::Error>>;
struct SkillRequests {
    requests: FuturesUnordered<SkillRequest>,
    recipients: HashMap<SkillPath, Vec<Recipient>>,
}

/// Keep track of running skill requests register recipients for these requests.
/// Make sure only one request for a skill is running at a time.
impl SkillRequests {
    pub fn new() -> Self {
        SkillRequests {
            requests: FuturesUnordered::new(),
            recipients: HashMap::new(),
        }
    }

    /// Register a new request for a skill.
    /// If there is already a request for the skill, no new request will be added,
    /// but the recipient will be added to the list of recipients.
    pub fn push(
        &mut self,
        skill_path: SkillPath,
        skill_request: SkillRequest,
        recipient: Recipient,
    ) {
        if let Some(recipients) = self.recipients.get_mut(&skill_path) {
            recipients.push(recipient);
        } else {
            self.requests.push(skill_request);
            self.recipients.insert(skill_path, vec![recipient]);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }

    /// Select the next finished skill request and send the result to the recipients.
    /// Return the skill path and result.
    pub async fn select_next_some(&mut self) -> anyhow::Result<(SkillPath, (Arc<Skill>, Digest))> {
        let (skill_path, result) = self.requests.select_next_some().await;
        let senders = self.recipients.remove(&skill_path).unwrap();
        match result {
            Ok((skill, digest)) => {
                let skill = Arc::new(skill);
                for sender in senders {
                    drop(sender.send(Ok(Some(skill.clone()))));
                }
                Ok((skill_path, (skill, digest)))
            }
            Err(e) => {
                for sender in senders {
                    drop(sender.send(Err(anyhow!(e.to_string()))));
                }
                Err(e)
            }
        }
    }
}

impl SkillStoreActor {
    pub fn new(
        receiver: mpsc::Receiver<SkillStoreMessage>,
        skill_loader: SkillLoaderApi,
        configuration: SkillConfigurationApi,
        digest_update_interval: Duration,
    ) -> Self {
        SkillStoreActor {
            receiver,
            provider: SkillStoreState::new(skill_loader, configuration),
            digest_update_interval,
            skill_requests: SkillRequests::new(),
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
                // The if expressions makes sure we only poll if there are cached skills so that `select`
                // will only wait on messages until we have a cache.
                result = self.provider.refresh_oldest_digest(self.digest_update_interval), if self.provider.list_cached_skills().next().is_some() => {
                    if let Err(e) = result {
                        error!("Error refreshing digest: {e}");
                    }
                },
                // FuturesUnordered will let them run in parallel. It will
                // yield once one of them is completed.
                result = self.skill_requests.select_next_some(), if !self.skill_requests.is_empty()  => {
                    if let Ok((skill_path, (skill, digest))) = result {
                        self.provider.insert(skill_path, skill, digest);
                    }
                }
            }
        }
    }

    pub async fn act(&mut self, msg: SkillStoreMessage) {
        match msg {
            SkillStoreMessage::Fetch { skill_path, send } => {
                if let Some(skill) = self.provider.cached_skill(&skill_path) {
                    drop(send.send(Ok(Some(skill))));
                    return;
                }
                match self.provider.tag(&skill_path).await {
                    Ok(Some(tag)) => {
                        let skill_loader = self.provider.skill_loader.clone();
                        let skill =
                            ConfiguredSkill::new(&skill_path.namespace, &skill_path.name, tag);
                        let cloned_skill_path = skill_path.clone();
                        let fut = async move {
                            let result = skill_loader.fetch(skill).await;
                            (cloned_skill_path, result)
                        };
                        self.skill_requests.push(skill_path, Box::pin(fut), send);
                    }
                    Ok(None) => {
                        drop(send.send(Ok(None)));
                    }
                    Err(e) => {
                        drop(send.send(Err(e)));
                    }
                }
            }
            SkillStoreMessage::List { send } => {
                drop(send.send(self.provider.skills().await));
            }
            SkillStoreMessage::Remove { skill_path } => {
                self.provider.invalidate(&skill_path);
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
    use test_skills::{given_chat_skill, given_greet_skill_v0_2};
    use tokio::time::{sleep, timeout};

    use crate::skill_configuration::tests::StubSkillConfiguration;
    use crate::skill_configuration::SkillConfiguration;
    use crate::skill_loader::{RegistryConfig, SkillLoader, SkillLoaderMsg};
    use crate::skills::{Engine, SkillPath};

    use super::*;

    pub use super::SkillStoreMessage;

    pub fn dummy_skill_store_api() -> SkillStoreApi {
        let (send, _recv) = mpsc::channel(1);
        SkillStoreApi::new(send)
    }

    impl SkillStoreState {
        async fn with_namespace_and_skill(
            engine: Arc<Engine>,
            configuration: SkillConfigurationApi,
            skill_path: &SkillPath,
        ) -> Self {
            let skill = ConfiguredSkill::from_path(skill_path);
            configuration.upsert(skill).await;
            let skill_loader =
                SkillLoader::with_file_registry(engine, skill_path.namespace.clone()).api();

            SkillStoreState::new(skill_loader, configuration)
        }
    }

    // Needed for unwrapping the error in the test
    impl std::fmt::Debug for Skill {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "Skill")
        }
    }

    #[tokio::test]
    async fn requests_for_same_skill_are_cached() {
        // Given a skill request cache
        let mut cache = SkillRequests::new();
        let skill_path = SkillPath::dummy();
        let skill_path_clone = skill_path.clone();

        // When pushing two requests for the same skill to the cache
        let fut = async move { (skill_path_clone, Err(anyhow!("First response"))) };
        let (first_send, first_recv) = oneshot::channel();
        cache.push(skill_path.clone(), Box::pin(fut), first_send);

        let skill_path_clone = skill_path.clone();
        let fut = async move { (skill_path_clone, Err(anyhow!("Second response"))) };
        let (second_send, second_recv) = oneshot::channel();
        cache.push(skill_path, Box::pin(fut), second_send);

        // And awaiting the next skill request
        cache.select_next_some().await.unwrap_err();

        // Then both have been answered by the first response
        assert!(first_recv
            .await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("First response"));
        assert!(second_recv
            .await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("First response"));

        // And the cache is empty
        assert!(cache.recipients.is_empty());
        assert!(cache.requests.is_empty());
    }

    #[tokio::test]
    async fn requests_for_different_skills_are_not_cached() {
        // Given a skill request cache
        let mut cache = SkillRequests::new();
        let first_skill = SkillPath::new("local", "first_skill");
        let second_skill = SkillPath::new("local", "second_skill");

        // When pushing two requests for different skills to the cache
        let first_skill_clone = first_skill.clone();
        let fut = async move { (first_skill_clone, Err(anyhow!("First response"))) };
        let (first_send, first_recv) = oneshot::channel();
        cache.push(first_skill, Box::pin(fut), first_send);

        let second_skill_clone = second_skill.clone();
        let fut = async move { (second_skill_clone, Err(anyhow!("Second response"))) };
        let (second_send, second_recv) = oneshot::channel();
        cache.push(second_skill, Box::pin(fut), second_send);

        // And awaiting the next skill request
        cache.select_next_some().await.unwrap_err();
        cache.select_next_some().await.unwrap_err();

        // Then the first request has been answered by the first response
        assert!(first_recv
            .await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("First response"));
        assert!(second_recv
            .await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("Second response"));

        // And the cache is empty
        assert!(cache.requests.is_empty());
        assert!(cache.recipients.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn skill_store_issues_only_one_request_for_a_skill() {
        // Given a skill store with a configured skill
        let configuration = StubSkillConfiguration::all_skills_allowed().api();
        let (send, mut recv) = mpsc::channel(2);
        let skill_loader = SkillLoaderApi::new(send);
        let skill_store =
            SkillStore::new(skill_loader, configuration, Duration::from_secs(10)).api();

        let skill_path = SkillPath::new("local", "greet_skill_v0_2");

        // And given two pending fetch requests
        let cloned_skill_path = skill_path.clone();
        let cloned_skill_store = skill_store.clone();
        let first_request =
            tokio::spawn(async move { cloned_skill_store.fetch(cloned_skill_path).await });

        let second_request = tokio::spawn(async move { skill_store.fetch(skill_path).await });

        // And waiting for 10 ms for both requests to be issued
        sleep(Duration::from_millis(10)).await;

        // When answering the first request with an error message
        match recv.recv().await.unwrap() {
            SkillLoaderMsg::Fetch { send, .. } => {
                drop(send.send(Err(anyhow!("First request response"))));
            }
            SkillLoaderMsg::FetchDigest { .. } => unreachable!(),
        }

        // Then both requests have answered with the error response
        assert!(timeout(Duration::from_millis(10), first_request)
            .await
            .is_ok());
        assert!(timeout(Duration::from_millis(10), second_request)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn skills_are_sorted() {
        // Given a provider with two skills
        let (sender, _recv) = mpsc::channel(1);
        let skill_loader = SkillLoaderApi::new(sender);
        let skills = vec![
            ConfiguredSkill::new("a", "a", "latest"),
            ConfiguredSkill::new("a", "b", "latest"),
            ConfiguredSkill::new("b", "a", "latest"),
            ConfiguredSkill::new("b", "b", "latest"),
        ];
        let configuration = SkillConfiguration::with_skills(skills).await.api();
        let skill_store =
            SkillStore::new(skill_loader, configuration, Duration::from_secs(10)).api();

        // When skills are listed
        let skills = skill_store.list().await;

        // Then the skills are sorted
        assert_eq!(
            skills,
            vec![
                SkillPath::new("a", "a"),
                SkillPath::new("a", "b"),
                SkillPath::new("b", "a"),
                SkillPath::new("b", "b"),
            ]
        );
    }

    #[tokio::test]
    async fn skill_configured_but_not_loadable_error() {
        // Given a skill store with a configured skill that is not loadable
        let skill_path = SkillPath::dummy();
        let engine = Arc::new(Engine::new(false).unwrap());
        let skill_loader =
            SkillLoader::with_file_registry(engine, skill_path.namespace.clone()).api();
        let configuration = StubSkillConfiguration::all_skills_allowed().api();
        let skill_store =
            SkillStore::new(skill_loader, configuration, Duration::from_secs(10)).api();

        // When a fetch request is issued for a skill that is configured but not loadable
        let result = skill_store.fetch(skill_path).await;

        // Then a good error message is returned
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("configured but not loadable"));
    }

    #[tokio::test]
    async fn skill_component_not_in_config() {
        let skill_path = SkillPath::dummy();
        let engine = Arc::new(Engine::new(false).unwrap());
        let configuration = SkillConfiguration::new().api();
        let provider =
            SkillStoreState::with_namespace_and_skill(engine, configuration, &skill_path).await;

        let result = provider
            .tag(&SkillPath::new(&skill_path.namespace, "non_existing_skill"))
            .await;
        assert!(matches!(result, Ok(None)));
    }

    #[tokio::test]
    async fn namespace_not_in_config() {
        let skill_path = SkillPath::dummy();
        let engine = Arc::new(Engine::new(false).unwrap());
        let configuration = SkillConfiguration::new().api();
        let provider =
            SkillStoreState::with_namespace_and_skill(engine, configuration, &skill_path).await;

        let result = provider
            .tag(&SkillPath::new("non_existing_namespace", &skill_path.name))
            .await;

        assert!(matches!(result, Ok(None)));
    }

    #[tokio::test]
    async fn cached_skill_removed() {
        // Given one cached skill
        given_greet_skill_v0_2();
        let skill_path = SkillPath::new("local", "greet_skill_v0_2");
        let configured_skill = ConfiguredSkill::from_path(&skill_path);
        let engine = Arc::new(Engine::new(false).unwrap());
        let configuration = SkillConfiguration::new().api();
        let mut provider =
            SkillStoreState::with_namespace_and_skill(engine, configuration, &skill_path).await;
        let (skill, digest) = provider.skill_loader.fetch(configured_skill).await.unwrap();
        provider.insert(skill_path.clone(), Arc::new(skill), digest);

        // When we remove the skill
        provider.invalidate(&skill_path);

        // Then the skill is no longer cached
        assert!(provider.list_cached_skills().next().is_none());
    }

    #[tokio::test]
    async fn should_error_if_fetching_skill_from_invalid_namespace() {
        // given a skill in an invalid namespace
        let skill_path = SkillPath::new("local", "greet_skill_v0_2");
        let engine = Arc::new(Engine::new(false).unwrap());
        let configuration = SkillConfiguration::new().api();
        configuration
            .set_namespace_error(skill_path.namespace.clone(), Some(anyhow!("")))
            .await;
        let provider =
            SkillStoreState::with_namespace_and_skill(engine, configuration, &skill_path).await;

        // when fetching the tag
        let result = provider.tag(&skill_path).await;

        // then it returns an error
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn should_only_cache_skills_that_have_been_fetched() {
        given_greet_skill_v0_2();
        // Given local is a configured namespace, backed by a file repository with "greet_skill"
        // and "greet-py"
        let engine = Arc::new(Engine::new(false).unwrap());
        let configured_skills = vec![
            ConfiguredSkill::new("local", "greet_skill_v0_2", "latest"),
            ConfiguredSkill::new("local", "greet-py-v0_2", "latest"),
        ];
        let configuration = SkillConfiguration::with_skills(configured_skills)
            .await
            .api();
        let skill_loader = SkillLoader::with_file_registry(engine, "local".to_owned()).api();
        let skill_store = SkillStore::new(skill_loader, configuration, Duration::from_secs(10));

        let greet_skill = SkillPath::new("local", "greet_skill_v0_2");

        // When fetching "greet_skill" but not "greet-py"
        skill_store.api().fetch(greet_skill.clone()).await.unwrap();
        // and listing all cached skills
        let cached_skills = skill_store.api().list_cached().await;

        // Then only "greet_skill" will appear in that list, but not "greet-py"
        assert_eq!(cached_skills, vec![greet_skill]);

        // Cleanup
        skill_store.wait_for_shutdown().await;
    }

    #[tokio::test]
    async fn should_list_skills_that_have_been_added() {
        // Given an empty provider
        let engine = Arc::new(Engine::new(false).unwrap());
        let configuration = SkillConfiguration::new();
        let skill_loader = SkillLoader::with_file_registry(engine, "local".to_owned()).api();
        let skill_store =
            SkillStore::new(skill_loader, configuration.api(), Duration::from_secs(10));
        let api = configuration.api();

        // When adding two skills
        let skill = ConfiguredSkill::from_path(&SkillPath::new("local", "one"));
        api.upsert(skill).await;
        let skill = ConfiguredSkill::from_path(&SkillPath::new("local", "two"));
        api.upsert(skill).await;
        let skills = skill_store.api().list().await;

        // Then the skills are listed by the skill executor api
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
        given_greet_skill_v0_2();
        let greet_skill = SkillPath::new("local", "greet_skill_v0_2");
        let configured_skill = ConfiguredSkill::from_path(&greet_skill);
        let engine = Arc::new(Engine::new(false).unwrap());
        let skill_loader = SkillLoader::with_file_registry(engine, "local".to_owned()).api();
        let configuration = SkillConfiguration::with_skill(configured_skill).await.api();
        let skill_store = SkillStore::new(skill_loader, configuration, Duration::from_secs(10));
        let api = skill_store.api();
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
        let configured_skills = ConfiguredSkill::new("local", "greet_skill", "latest");
        let configuration = SkillConfiguration::with_skill(configured_skills)
            .await
            .api();
        let skill_store = SkillStore::new(skill_loader, configuration, Duration::from_secs(10));
        let api = skill_store.api();

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
        let skill_loader = SkillLoader::from_config(engine, registry_config).api();
        let configuration = StubSkillConfiguration::all_skills_allowed().api();
        let mut skill_store_state = SkillStoreState::new(skill_loader, configuration);

        let greet_skill = SkillPath::new("local", "greet_skill_v0_2");
        let configured_skill = ConfiguredSkill::from_path(&greet_skill);
        let (skill, digest) = skill_store_state
            .skill_loader
            .fetch(configured_skill)
            .await?;
        skill_store_state.insert(greet_skill.clone(), Arc::new(skill), digest);
        assert_eq!(
            skill_store_state.list_cached_skills().collect::<Vec<_>>(),
            vec![&greet_skill]
        );

        // When we update "greet_skill"s contents and we clear out expired skills
        let wasm_file = temp_dir.path().join("greet_skill_v0_2.wasm");
        let prev_time = fs::metadata(&wasm_file)?.modified()?;
        // Another skill so we can copy it over
        given_chat_skill();
        fs::copy("./skills/chat_skill.wasm", &wasm_file)?;
        let new_time = fs::metadata(&wasm_file)?.modified()?;
        assert_ne!(prev_time, new_time);

        skill_store_state
            .validate_digest(greet_skill.clone())
            .await?;

        // Then greet skill is no longer listed in the cache
        assert_eq!(skill_store_state.list_cached_skills().count(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn doesnt_invalidate_unchanged_digests() -> anyhow::Result<()> {
        // Given one cached "greet_skill"
        given_greet_skill_v0_2();
        let greet_skill = SkillPath::new("local", "greet_skill_v0_2");
        let configured_skill = ConfiguredSkill::from_path(&greet_skill);
        let engine = Arc::new(Engine::new(false)?);
        let skill_loader = SkillLoader::with_file_registry(engine, "local".to_owned()).api();
        let configuration = SkillConfiguration::with_skill(configured_skill.clone())
            .await
            .api();
        let mut skill_store_state = SkillStoreState::new(skill_loader, configuration);

        let (skill, digest) = skill_store_state
            .skill_loader
            .fetch(configured_skill)
            .await?;
        skill_store_state.insert(greet_skill.clone(), Arc::new(skill), digest);

        // When we check it with no changes
        skill_store_state.validate_digest(greet_skill).await?;

        // Then nothing changes
        assert_eq!(skill_store_state.list_cached_skills().count(), 1);
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn should_invalidate_cached_skill_whose_digest_has_changed() -> anyhow::Result<()> {
        // Given one cached "greet_skill"
        let (registry_config, temp_dir) = tmp_registries_with_skill()?;
        let greet_skill = SkillPath::new("local", "greet_skill_v0_2");
        let configured_skill = ConfiguredSkill::from_path(&greet_skill);
        let engine = Arc::new(Engine::new(false)?);
        let skill_loader = SkillLoader::from_config(engine, registry_config).api();
        let configuration = SkillConfiguration::with_skill(configured_skill).await.api();
        let skill_store = SkillStore::new(skill_loader, configuration, Duration::from_secs(1));

        let api = skill_store.api();
        api.fetch(greet_skill.clone()).await?;
        assert_eq!(api.list_cached().await, vec![greet_skill.clone()]);

        // When we update "greet_skill"s contents
        let wasm_file = temp_dir.path().join("greet_skill_v0_2.wasm");
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
        given_greet_skill_v0_2();
        let dir = tempfile::tempdir()?;
        fs::copy(
            "./skills/greet_skill_v0_2.wasm",
            dir.path().join("greet_skill_v0_2.wasm"),
        )?;

        let path = dir.path().to_string_lossy().into();
        let registry_config = RegistryConfig::with_file_registry("local".to_owned(), path);
        Ok((registry_config, dir))
    }
}
