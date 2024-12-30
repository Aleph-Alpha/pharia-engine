use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc, time::Duration};

use crate::{
    namespace_watcher::Namespace,
    registries::Digest,
    skill_loader::{ConfiguredSkill, SkillLoaderApi, SkillLoaderError},
    skills::{Skill, SkillPath},
};
use anyhow::anyhow;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use itertools::Itertools;
use tokio::{
    select,
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::{sleep_until, Instant},
};
use tracing::{error, info};

/// A wrapper around the Skill to also keep track of which
/// digest was loaded last and when it was last checked.
struct CachedSkill {
    /// Compiled and pre-initialized skill
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
    known_skills: HashMap<SkillPath, String>,
    cached_skills: HashMap<SkillPath, CachedSkill>,
    invalid_namespaces: HashMap<Namespace, anyhow::Error>,
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

    pub fn upsert_skill(&mut self, skill: ConfiguredSkill) {
        info!("New or changed skill: {skill}");
        let skill_path = SkillPath::new(skill.namespace, &skill.name);
        if self
            .known_skills
            .insert(skill_path.clone(), skill.tag)
            .is_some()
        {
            self.invalidate(&skill_path);
        }
    }

    pub fn remove_skill(&mut self, skill: &SkillPath) {
        info!("Removed skill: {skill}");
        self.known_skills.remove(skill);
        self.invalidate(skill);
    }

    /// All configured skills, sorted by namespace and name
    pub fn skills(&self) -> impl Iterator<Item = &SkillPath> {
        self.known_skills.keys().sorted_by(|a, b| {
            a.namespace
                .cmp(&b.namespace)
                .then_with(|| a.name.cmp(&b.name))
        })
    }

    pub fn list_cached_skills(&self) -> impl Iterator<Item = &SkillPath> + '_ {
        self.cached_skills.keys()
    }

    pub fn invalidate(&mut self, skill_path: &SkillPath) -> bool {
        self.cached_skills.remove(skill_path).is_some()
    }

    pub fn add_invalid_namespace(&mut self, namespace: Namespace, e: anyhow::Error) {
        self.invalid_namespaces.insert(namespace, e);
    }

    pub fn remove_invalid_namespace(&mut self, namespace: &Namespace) {
        self.invalid_namespaces.remove(namespace);
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
    fn tag(&self, skill_path: &SkillPath) -> Result<Option<&str>, SkillLoaderError> {
        if let Some(error) = self.invalid_namespaces.get(&skill_path.namespace) {
            return Err(SkillLoaderError::InvalidNamespace(error.to_string()));
        }
        Ok(self.known_skills.get(skill_path).map(String::as_str))
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

        let skill = ConfiguredSkill::new(skill_path.namespace.clone(), &skill_path.name, tag);
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

    pub async fn upsert(&self, skill: ConfiguredSkill) {
        let msg = SkillStoreMessage::Upsert { skill };
        self.sender
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
    }

    /// Report a namespace as erroneous (e.g. in case its configuration is messed up). Set `None`
    /// to communicate that a namespace is no longer erroneous.
    pub async fn set_namespace_error(&self, namespace: Namespace, error: Option<anyhow::Error>) {
        let msg = SkillStoreMessage::SetNamespaceError { namespace, error };
        self.sender
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
    }

    /// Fetch an executable skill
    pub async fn fetch(
        &self,
        skill_path: SkillPath,
    ) -> Result<Option<Arc<Skill>>, SkillLoaderError> {
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
        send: oneshot::Sender<Result<Option<Arc<Skill>>, SkillLoaderError>>,
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
        skill: ConfiguredSkill,
    },
    SetNamespaceError {
        namespace: Namespace,
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
    skill_requests: SkillRequests,
}

type SkillRequest =
    Pin<Box<dyn Future<Output = (SkillPath, Result<(Skill, Digest), SkillLoaderError>)> + Send>>;

type Recipient = oneshot::Sender<Result<Option<Arc<Skill>>, SkillLoaderError>>;
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
    pub async fn select_next_some(
        &mut self,
    ) -> Result<(SkillPath, (Arc<Skill>, Digest)), SkillLoaderError> {
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
                    // We can not copy the error, but also don't want to make any formatting decisions
                    // at this level. Therefore, we try to clone the original error chain
                    match &e {
                        SkillLoaderError::Other(e) => {
                            let root = anyhow!(e.root_cause().to_string());
                            let error = e
                                .chain()
                                .rev()
                                .skip(1)
                                .fold(root, |error, cause| error.context(cause.to_string()));
                            drop(sender.send(Err(SkillLoaderError::Other(error))));
                        }
                        SkillLoaderError::NotSupportedYet => {
                            drop(sender.send(Err(SkillLoaderError::NotSupportedYet)));
                        }
                        SkillLoaderError::NoLongerSupported => {
                            drop(sender.send(Err(SkillLoaderError::NoLongerSupported)));
                        }
                        SkillLoaderError::LinkerError(e) => {
                            drop(sender.send(Err(SkillLoaderError::LinkerError(e.clone()))));
                        }
                        SkillLoaderError::InvalidNamespace(e) => {
                            drop(sender.send(Err(SkillLoaderError::InvalidNamespace(e.clone()))));
                        }
                        SkillLoaderError::Unloadable => {
                            drop(sender.send(Err(SkillLoaderError::Unloadable)));
                        }
                        SkillLoaderError::RegistryError(e) => {
                            drop(sender.send(Err(SkillLoaderError::RegistryError(e.clone()))));
                        }
                    }
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
        digest_update_interval: Duration,
    ) -> Self {
        SkillStoreActor {
            receiver,
            provider: SkillStoreState::new(skill_loader),
            digest_update_interval,
            skill_requests: SkillRequests::new(),
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

    pub fn act(&mut self, msg: SkillStoreMessage) {
        match msg {
            SkillStoreMessage::Fetch { skill_path, send } => {
                if let Some(skill) = self.provider.cached_skill(&skill_path) {
                    drop(send.send(Ok(Some(skill))));
                    return;
                }
                match self.provider.tag(&skill_path) {
                    Ok(Some(tag)) => {
                        let skill_loader = self.provider.skill_loader.clone();
                        let skill = ConfiguredSkill::new(
                            skill_path.namespace.clone(),
                            &skill_path.name,
                            tag,
                        );
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
                drop(send.send(self.provider.skills().cloned().collect()));
            }
            SkillStoreMessage::Remove { skill_path } => {
                self.provider.remove_skill(&skill_path);
            }
            SkillStoreMessage::Upsert { skill } => {
                self.provider.upsert_skill(skill);
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
    use test_skills::{given_chat_skill, given_greet_skill_v0_2};
    use tokio::time::{sleep, timeout};

    use crate::namespace_watcher::Namespace;
    use crate::skill_loader::{RegistryConfig, SkillLoader, SkillLoaderError, SkillLoaderMsg};
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
            let skill = ConfiguredSkill::from_path(skill_path);
            provider.upsert_skill(skill);
            provider
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
        let fut = async move {
            (
                skill_path_clone,
                Err(SkillLoaderError::LinkerError("First error".to_string())),
            )
        };
        let (first_send, first_recv) = oneshot::channel();
        cache.push(skill_path.clone(), Box::pin(fut), first_send);

        let skill_path_clone = skill_path.clone();
        let fut = async move {
            (
                skill_path_clone,
                Err(SkillLoaderError::LinkerError("Second error".to_string())),
            )
        };
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
            .contains("First error"));
        assert!(second_recv
            .await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("First error"));

        // And the cache is empty
        assert!(cache.recipients.is_empty());
        assert!(cache.requests.is_empty());
    }

    #[tokio::test]
    async fn requests_for_different_skills_are_not_cached() {
        // Given a skill request cache
        let mut cache = SkillRequests::new();
        let first_skill = SkillPath::local("first_skill");
        let second_skill = SkillPath::local("second_skill");

        // When pushing two requests for different skills to the cache
        let first_skill_clone = first_skill.clone();
        let fut = async move {
            (
                first_skill_clone,
                Err(SkillLoaderError::LinkerError("First error".to_string())),
            )
        };
        let (first_send, first_recv) = oneshot::channel();
        cache.push(first_skill, Box::pin(fut), first_send);

        let second_skill_clone = second_skill.clone();
        let fut = async move {
            (
                second_skill_clone,
                Err(SkillLoaderError::LinkerError("Second error".to_string())),
            )
        };
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
            .contains("First error"));
        assert!(second_recv
            .await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("Second error"));

        // And the cache is empty
        assert!(cache.requests.is_empty());
        assert!(cache.recipients.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn skill_store_issues_only_one_request_for_a_skill() {
        // Given a skill store with a configured skill
        let (send, mut recv) = mpsc::channel(2);
        let skill_loader = SkillLoaderApi::new(send);
        let skill_store = SkillStore::new(skill_loader, Duration::from_secs(10)).api();

        let skill_path = SkillPath::local("greet_skill_v0_2");
        let skill = ConfiguredSkill::from_path(&skill_path);
        skill_store.upsert(skill).await;

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
                drop(send.send(Err(SkillLoaderError::LinkerError(
                    "First request response".to_string(),
                ))));
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
        let a = Namespace::new("a").unwrap();
        let b = Namespace::new("b").unwrap();

        let skill_store = SkillStore::new(skill_loader, Duration::from_secs(10)).api();
        skill_store
            .upsert(ConfiguredSkill::new(a.clone(), "a", "latest"))
            .await;
        skill_store
            .upsert(ConfiguredSkill::new(a, "b", "latest"))
            .await;
        skill_store
            .upsert(ConfiguredSkill::new(b.clone(), "a", "latest"))
            .await;
        skill_store
            .upsert(ConfiguredSkill::new(b, "b", "latest"))
            .await;

        // When skills are listed
        let skills: Vec<String> = skill_store
            .list()
            .await
            .iter()
            .map(ToString::to_string)
            .collect();

        // Then the skills are sorted
        assert_eq!(skills, ["a/a", "a/b", "b/a", "b/b"]);
    }

    #[tokio::test]
    async fn skill_configured_but_not_loadable_error() {
        // Given a skill store with a configured skill that is not loadable
        let skill_path = SkillPath::dummy();
        let skill = ConfiguredSkill::from_path(&skill_path);
        let engine = Arc::new(Engine::new(false).unwrap());
        let skill_loader =
            SkillLoader::with_file_registry(engine, skill_path.namespace.clone()).api();

        let skill_store = SkillStore::new(skill_loader, Duration::from_secs(10)).api();

        skill_store.upsert(skill).await;

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
        let provider = SkillStoreState::with_namespace_and_skill(engine, &skill_path);

        let result = provider.tag(&SkillPath::new(skill_path.namespace, "non_existing_skill"));
        assert!(matches!(result, Ok(None)));
    }

    #[tokio::test]
    async fn namespace_not_in_config() {
        // Given a provider with a skill in a particular namespace
        let namespace = Namespace::new("non-existing").unwrap();
        let existing = SkillPath::new(namespace.clone(), "existing_skill");
        let engine = Arc::new(Engine::new(false).unwrap());
        let provider = SkillStoreState::with_namespace_and_skill(engine, &existing);

        // When requesting a different, non-existing skill in the same namespace
        let non_existing = SkillPath::new(namespace, "non_existing_skill");
        let result = provider.tag(&non_existing);

        // Then the skill is not found
        assert!(matches!(result, Ok(None)));
    }

    #[tokio::test]
    async fn cached_skill_removed() {
        // Given one cached skill
        given_greet_skill_v0_2();
        let skill_path = SkillPath::local("greet_skill_v0_2");
        let configured_skill = ConfiguredSkill::from_path(&skill_path);
        let engine = Arc::new(Engine::new(false).unwrap());
        let mut provider = SkillStoreState::with_namespace_and_skill(engine, &skill_path);
        let (skill, digest) = provider.skill_loader.fetch(configured_skill).await.unwrap();
        provider.insert(skill_path.clone(), Arc::new(skill), digest);

        // When we remove the skill
        provider.remove_skill(&skill_path);

        // Then the skill is no longer cached
        assert!(provider.list_cached_skills().next().is_none());
    }

    #[tokio::test]
    async fn should_error_if_fetching_skill_from_invalid_namespace() {
        // given a skill in an invalid namespace
        let skill_path = SkillPath::local("greet_skill_v0_2");
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
        given_greet_skill_v0_2();
        // Given local is a configured namespace, backed by a file repository with "greet_skill"
        // and "greet-py"
        let engine = Arc::new(Engine::new(false).unwrap());
        let greet_skill = SkillPath::local("greet_skill_v0_2");
        let skill_loader =
            SkillLoader::with_file_registry(engine, greet_skill.namespace.clone()).api();
        let skill_store = SkillStore::new(skill_loader, Duration::from_secs(10));
        let skill = ConfiguredSkill::from_path(&greet_skill);
        skill_store.api().upsert(skill).await;
        let skill = ConfiguredSkill::from_path(&SkillPath::local("greet-py-v0_2"));
        skill_store.api().upsert(skill).await;

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
        let first = SkillPath::local("one");
        let second = SkillPath::local("two");

        let skill_loader = SkillLoader::with_file_registry(engine, first.namespace.clone()).api();
        let skill_store = SkillStore::new(skill_loader, Duration::from_secs(10));
        let api = skill_store.api();

        // When adding two skills
        let skill = ConfiguredSkill::from_path(&first);
        api.upsert(skill).await;

        let skill = ConfiguredSkill::from_path(&second);
        api.upsert(skill).await;
        let skills = api.list().await;

        // Then the skills are listed by the skill executor api
        assert_eq!(skills.len(), 2);
        assert!(skills.contains(&first));
        assert!(skills.contains(&second));

        // Cleanup
        drop(api);
        skill_store.wait_for_shutdown().await;
    }

    #[tokio::test]
    async fn should_remove_invalidated_skill_from_cache() {
        // Given one cached "greet_skill"
        given_greet_skill_v0_2();
        let greet_skill = SkillPath::local("greet_skill_v0_2");
        let engine = Arc::new(Engine::new(false).unwrap());
        let skill_loader =
            SkillLoader::with_file_registry(engine, greet_skill.namespace.clone()).api();
        let skill_store = SkillStore::new(skill_loader, Duration::from_secs(10));
        let api = skill_store.api();
        let skill = ConfiguredSkill::from_path(&greet_skill);
        api.upsert(skill).await;
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
        let greet_skill = SkillPath::local("greet_skill");
        let engine = Arc::new(Engine::new(false).unwrap());
        let skill_loader =
            SkillLoader::with_file_registry(engine, greet_skill.namespace.clone()).api();
        let skill_store = SkillStore::new(skill_loader, Duration::from_secs(10));
        let api = skill_store.api();

        let skill = ConfiguredSkill::from_path(&greet_skill);
        api.upsert(skill).await;

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
        let mut skill_store_state = SkillStoreState::new(skill_loader);

        let greet_skill = SkillPath::local("greet_skill_v0_2");
        let configured_skill = ConfiguredSkill::from_path(&greet_skill);
        skill_store_state.upsert_skill(configured_skill.clone());
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
    async fn does_not_invalidate_unchanged_digests() -> anyhow::Result<()> {
        // Given one cached "greet_skill"
        given_greet_skill_v0_2();
        let greet_skill = SkillPath::local("greet_skill_v0_2");
        let engine = Arc::new(Engine::new(false)?);
        let skill_loader =
            SkillLoader::with_file_registry(engine, greet_skill.namespace.clone()).api();
        let mut skill_store_state = SkillStoreState::new(skill_loader);

        let configured_skill = ConfiguredSkill::from_path(&greet_skill);
        skill_store_state.upsert_skill(configured_skill.clone());
        let (skill, digest) = skill_store_state
            .skill_loader
            .fetch(configured_skill)
            .await?;
        skill_store_state.insert(greet_skill.clone(), Arc::new(skill), digest);

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
        let greet_skill = SkillPath::local("greet_skill_v0_2");
        let configured_skill = ConfiguredSkill::from_path(&greet_skill);
        let engine = Arc::new(Engine::new(false)?);
        let skill_loader = SkillLoader::from_config(engine, registry_config).api();
        let skill_store = SkillStore::new(skill_loader, Duration::from_secs(1));
        let api = skill_store.api();
        api.upsert(configured_skill).await;
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
        let namespace = Namespace::new("local").unwrap();
        let registry_config = RegistryConfig::with_file_registry(namespace, path);
        Ok((registry_config, dir))
    }
}
