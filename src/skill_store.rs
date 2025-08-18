// This is coming from the OpenAPI macro from utoipa.
#[allow(clippy::needless_for_each)]
mod routes;

pub use routes::{
    SkillStoreProvider, http_skill_store_v0, http_skill_store_v1, openapi_skill_store_v1,
};

use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc, time::Duration};

use crate::{
    hardcoded_skills::{SkillChat, hardcoded_skill},
    logging::TracingContext,
    namespace_watcher::{Namespace, SkillDescription},
    skill::{Skill, SkillPath},
    skill_cache::SkillCache,
    skill_loader::{
        ConfiguredSkill, LoadedSkill, ProgrammableSkill, SkillDescriptionFilterType,
        SkillFetchError, SkillLoaderApi,
    },
};
use anyhow::anyhow;
use bytesize::ByteSize;
use futures::{StreamExt, stream::FuturesUnordered};
use itertools::Itertools;
use tokio::{
    select,
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::sleep_until,
};
use tracing::{error, info};

#[cfg(test)]
use double_trait::double;

struct SkillStoreState<L> {
    known_skills: HashMap<SkillPath, ConfiguredSkill>,
    cached_skills: SkillCache,
    invalid_namespaces: HashMap<Namespace, anyhow::Error>,
    skill_loader: L,
}

impl<L> SkillStoreState<L>
where
    L: SkillLoaderApi,
{
    pub fn new(skill_loader: L, desired_memory_usage: ByteSize) -> Self {
        SkillStoreState {
            known_skills: HashMap::new(),
            cached_skills: SkillCache::new(desired_memory_usage),
            invalid_namespaces: HashMap::new(),
            skill_loader,
        }
    }

    pub fn upsert_skill(&mut self, skill: ConfiguredSkill) {
        info!(target: "pharia-kernel::skill-store", "New or changed skill: {skill}");
        let skill_path = skill.path();
        if self
            .known_skills
            .insert(skill_path.clone(), skill)
            .is_some()
        {
            self.invalidate(&skill_path);
        }
    }

    pub fn remove_skill(&mut self, skill: &SkillPath) {
        info!(target: "pharia-kernel::skill-store", "Removed skill: {skill}");
        self.known_skills.remove(skill);
        self.invalidate(skill);
    }

    /// All configured skills, sorted by namespace and name
    pub fn skills(
        &self,
        skill_type: Option<&SkillDescriptionFilterType>,
    ) -> impl Iterator<Item = &SkillPath> {
        self.known_skills
            .iter()
            .filter_map(|(path, skill)| {
                if let Some(skill_type) = skill_type {
                    match (skill_type, &skill.description) {
                        (SkillDescriptionFilterType::Chat, SkillDescription::Chat { .. }) => {
                            Some(path)
                        }
                        (
                            SkillDescriptionFilterType::Programmable,
                            SkillDescription::Programmable { .. },
                        ) => Some(path),
                        _ => None,
                    }
                } else {
                    Some(path)
                }
            })
            .sorted_by(|a, b| {
                a.namespace
                    .cmp(&b.namespace)
                    .then_with(|| a.name.cmp(&b.name))
            })
    }

    pub fn list_cached_skills(&self) -> impl Iterator<Item = SkillPath> + '_ {
        self.cached_skills.keys()
    }

    pub fn invalidate(&mut self, skill_path: &SkillPath) -> bool {
        self.cached_skills.remove(skill_path)
    }

    pub fn add_invalid_namespace(&mut self, namespace: Namespace, e: anyhow::Error) {
        self.invalid_namespaces.insert(namespace, e);
    }

    pub fn remove_invalid_namespace(&mut self, namespace: &Namespace) {
        self.invalid_namespaces.remove(namespace);
    }

    /// `Some` if the skill is present in the cache, `None` if not
    pub fn cached_skill(&self, skill_path: &SkillPath) -> Option<Arc<dyn Skill>> {
        self.cached_skills.get(skill_path)
    }

    /// Insert a skill into the cache.
    pub fn insert(&mut self, skill_path: SkillPath, compiled_skill: LoadedSkill) {
        self.cached_skills.insert(skill_path, compiled_skill);
    }

    /// Return the configuration for a given skill
    fn skill_description(
        &self,
        skill_path: &SkillPath,
    ) -> Result<Option<&SkillDescription>, SkillStoreError> {
        if let Some(error) = self.invalid_namespaces.get(&skill_path.namespace) {
            return Err(SkillStoreError::InvalidNamespaceError(
                skill_path.namespace.clone(),
                error.to_string(),
            ));
        }
        Ok(self.known_skills.get(skill_path).map(|s| &s.description))
    }

    /// Compares the digest in the cache with the digest behind the corresponding tag in the registry.
    /// If the digest behind the tag has changed, remove the cache entry. Otherwise, update the validation time.
    async fn validate_digest(&mut self, skill_path: SkillPath) -> anyhow::Result<()> {
        let SkillDescription::Programmable { tag, .. } = self
            .skill_description(&skill_path)
            .expect("Bad namespace configuration for skill")
            .expect("Cached skill must be known")
        else {
            return Err(anyhow!("Skill {skill_path} is not a programmable skill"));
        };

        let skill = ProgrammableSkill::new(
            skill_path.namespace.clone(),
            skill_path.name.clone(),
            tag.to_owned(),
        );
        let result = self
            .skill_loader
            .fetch_digest(skill)
            .await
            .map_err(Into::into)
            .and_then(|digest| {
                digest.ok_or_else(|| anyhow!("Missing digest for skill {skill_path}"))
            });
        match result {
            Ok(latest_digest) => self
                .cached_skills
                .compare_latest_digest(skill_path, &latest_digest),
            Err(e) => {
                // Always update the digest_validated time so we don't loop over errors constantly.
                self.cached_skills.update_digest_validated(skill_path);
                Err(e)
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
        let Some((skill_path, last_checked)) = self.cached_skills.oldest_digest() else {
            return Ok(());
        };
        // Wait until is it time to refresh
        sleep_until(last_checked + update_interval).await;
        self.validate_digest(skill_path.as_ref().clone()).await
    }
}

pub struct SkillStore {
    sender: mpsc::Sender<SkillStoreMsg>,
    handle: JoinHandle<()>,
}

impl SkillStore {
    pub fn new(
        skill_loader: impl SkillLoaderApi + Clone + Send + 'static,
        digest_update_interval: Duration,
        desired_memory_usage: ByteSize,
    ) -> Self {
        let (sender, recv) = mpsc::channel(1);
        let mut actor = SkillStoreActor::new(
            recv,
            skill_loader,
            digest_update_interval,
            desired_memory_usage,
        );
        let handle = tokio::spawn(async move {
            actor.run().await;
        });
        SkillStore { sender, handle }
    }

    pub fn api(&self) -> mpsc::Sender<SkillStoreMsg> {
        self.sender.clone()
    }

    pub async fn wait_for_shutdown(self) {
        // Drop sender so actor terminates, as soon as all api handles are freed.
        drop(self.sender);
        self.handle.await.unwrap();
    }
}

#[cfg_attr(test, double(SkillStoreApiDouble))]
pub trait SkillStoreApi {
    fn remove(&self, skill_path: SkillPath) -> impl Future<Output = ()> + Send;

    fn upsert(&self, skill: ConfiguredSkill) -> impl Future<Output = ()> + Send;

    /// Report a namespace as erroneous (e.g. in case its configuration is messed up). Set `None`
    /// to communicate that a namespace is no longer erroneous.
    fn set_namespace_error(
        &self,
        namespace: Namespace,
        error: Option<anyhow::Error>,
    ) -> impl Future<Output = ()> + Send;

    /// Fetch an executable skill
    fn fetch(
        &self,
        skill_path: SkillPath,
        tracing_context: &TracingContext,
    ) -> impl Future<Output = Result<Option<Arc<dyn Skill>>, SkillStoreError>> + Send;

    /// List all skills which are currently cached and can be executed without fetching the wasm
    /// component from an OCI
    fn list_cached(&self) -> impl Future<Output = Vec<SkillPath>> + Send;

    /// List all skills from all namespaces
    fn list(
        &self,
        skill_type: Option<SkillDescriptionFilterType>,
    ) -> impl Future<Output = Vec<SkillPath>> + Send;

    /// Drops a skill from the cache in case it has been cached before. `true` if the skill has been
    /// in the cache before, `false` otherwise .
    fn invalidate_cache(&self, skill_path: SkillPath) -> impl Future<Output = bool> + Send;
}

impl SkillStoreApi for mpsc::Sender<SkillStoreMsg> {
    async fn remove(&self, skill_path: SkillPath) {
        let msg = SkillStoreMsg::Remove { skill_path };
        self.send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
    }

    async fn upsert(&self, skill: ConfiguredSkill) {
        let msg = SkillStoreMsg::Upsert { skill };
        self.send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
    }

    /// Report a namespace as erroneous (e.g. in case its configuration is messed up). Set `None`
    /// to communicate that a namespace is no longer erroneous.
    async fn set_namespace_error(&self, namespace: Namespace, error: Option<anyhow::Error>) {
        let msg = SkillStoreMsg::SetNamespaceError { namespace, error };
        self.send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
    }

    /// Fetch an executable skill
    async fn fetch(
        &self,
        skill_path: SkillPath,
        tracing_context: &TracingContext,
    ) -> Result<Option<Arc<dyn Skill>>, SkillStoreError> {
        let (send, recv) = oneshot::channel();
        let msg = SkillStoreMsg::Fetch {
            skill_path,
            send,
            tracing_context: tracing_context.clone(),
        };
        self.send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }

    /// List all skills which are currently cached and can be executed without fetching the wasm
    /// component from an OCI
    async fn list_cached(&self) -> Vec<SkillPath> {
        let (send, recv) = oneshot::channel();
        let msg = SkillStoreMsg::ListCached { send };
        self.send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }

    /// List all skills from all namespaces
    async fn list(&self, skill_type: Option<SkillDescriptionFilterType>) -> Vec<SkillPath> {
        let (send, recv) = oneshot::channel();
        let msg = SkillStoreMsg::List { send, skill_type };
        self.send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }

    /// Drops a skill from the cache in case it has been cached before. `true` if the skill has been
    /// in the cache before, `false` otherwise .
    async fn invalidate_cache(&self, skill_path: SkillPath) -> bool {
        let (send, recv) = oneshot::channel();
        let msg = SkillStoreMsg::InvalidateCache { skill_path, send };
        self.send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }
}

pub enum SkillStoreMsg {
    Fetch {
        skill_path: SkillPath,
        send: oneshot::Sender<Result<Option<Arc<dyn Skill>>, SkillStoreError>>,
        tracing_context: TracingContext,
    },
    List {
        send: oneshot::Sender<Vec<SkillPath>>,
        skill_type: Option<SkillDescriptionFilterType>,
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

#[derive(Debug, thiserror::Error)]
pub enum SkillStoreError {
    #[error(transparent)]
    SkillLoaderError(#[from] SkillFetchError),
    #[error("Namespace {0} is invalid: {1}")]
    InvalidNamespaceError(Namespace, String),
}

struct SkillStoreActor<L> {
    receiver: mpsc::Receiver<SkillStoreMsg>,
    provider: SkillStoreState<L>,
    digest_update_interval: Duration,
    skill_requests: SkillRequests,
}

type SkillRequest =
    Pin<Box<dyn Future<Output = (SkillPath, Result<LoadedSkill, SkillFetchError>)> + Send>>;

type Recipient = oneshot::Sender<Result<Option<Arc<dyn Skill>>, SkillStoreError>>;
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
    pub async fn select_next_some(&mut self) -> Result<(SkillPath, LoadedSkill), SkillFetchError> {
        let (skill_path, result) = self.requests.select_next_some().await;
        let senders = self.recipients.remove(&skill_path).unwrap();
        match result {
            Ok(compiled_skill) => {
                for sender in senders {
                    drop(sender.send(Ok(Some(compiled_skill.skill.clone()))));
                }
                Ok((skill_path, compiled_skill))
            }
            Err(e) => {
                for sender in senders {
                    drop(sender.send(Err(e.clone().into())));
                }
                Err(e)
            }
        }
    }
}

impl<L> SkillStoreActor<L>
where
    L: SkillLoaderApi + Clone + Send + 'static,
{
    pub fn new(
        receiver: mpsc::Receiver<SkillStoreMsg>,
        skill_loader: L,
        digest_update_interval: Duration,
        desired_memory_usage: ByteSize,
    ) -> Self {
        SkillStoreActor {
            receiver,
            provider: SkillStoreState::new(skill_loader, desired_memory_usage),
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
                        error!(target: "pharia-kernel::skill-store", "Error refreshing digest: {e:#}");
                    }
                },
                // FuturesUnordered will let them run in parallel. It will
                // yield once one of them is completed.
                result = self.skill_requests.select_next_some(), if !self.skill_requests.is_empty()  => {
                    if let Ok((skill_path, compiled_skill)) = result {
                        self.provider.insert(skill_path, compiled_skill);
                    }
                }
            }
        }
    }

    pub fn act(&mut self, msg: SkillStoreMsg) {
        match msg {
            SkillStoreMsg::Fetch {
                skill_path,
                send,
                tracing_context,
            } => {
                if let Some(skill) = self.provider.cached_skill(&skill_path) {
                    drop(send.send(Ok(Some(skill))));
                    return;
                }
                if let Some(skill) = hardcoded_skill(&skill_path) {
                    drop(send.send(Ok(Some(skill))));
                    return;
                }
                match self.provider.skill_description(&skill_path) {
                    Ok(Some(description)) => match description {
                        SkillDescription::Chat {
                            model,
                            system_prompt,
                            ..
                        } => {
                            let skill = SkillChat::new(model, system_prompt);
                            drop(send.send(Ok(Some(Arc::new(skill)))));
                        }
                        SkillDescription::Programmable { tag, .. } => {
                            let skill_loader = self.provider.skill_loader.clone();
                            let skill = ProgrammableSkill::new(
                                skill_path.namespace.clone(),
                                skill_path.name.clone(),
                                tag.to_owned(),
                            );
                            let cloned_skill_path = skill_path.clone();
                            self.skill_requests.push(
                                skill_path,
                                Box::pin(async move {
                                    let result = skill_loader.fetch(skill, tracing_context).await;
                                    (cloned_skill_path, result)
                                }),
                                send,
                            );
                        }
                    },
                    Ok(None) => {
                        drop(send.send(Ok(None)));
                    }
                    Err(e) => {
                        drop(send.send(Err(e)));
                    }
                }
            }
            SkillStoreMsg::List { send, skill_type } => {
                drop(send.send(self.provider.skills(skill_type.as_ref()).cloned().collect()));
            }
            SkillStoreMsg::Remove { skill_path } => {
                self.provider.remove_skill(&skill_path);
            }
            SkillStoreMsg::Upsert { skill } => {
                self.provider.upsert_skill(skill);
            }
            SkillStoreMsg::SetNamespaceError { namespace, error } => {
                if let Some(error) = error {
                    self.provider.add_invalid_namespace(namespace, error);
                } else {
                    self.provider.remove_invalid_namespace(&namespace);
                }
            }
            SkillStoreMsg::ListCached { send } => {
                drop(send.send(self.provider.list_cached_skills().collect()));
            }
            SkillStoreMsg::InvalidateCache { skill_path, send } => {
                let _ = send.send(self.provider.invalidate(&skill_path));
            }
        }
    }
}

#[cfg(test)]
pub mod tests {

    use double_trait::Dummy;
    use std::sync::Mutex;
    use tokio::time::{sleep, timeout};

    use crate::{
        namespace_watcher::Namespace,
        registries::{Digest, RegistryError},
        skill::SkillPath,
        skill_loader::{LoadedSkill, SkillLoader, SkillLoaderMsg},
    };

    use super::*;

    pub use super::SkillStoreMsg;

    impl SkillStore {
        /// Easier constuctor for tests.
        fn from_loader(skill_loader: impl SkillLoaderApi + Clone + Send + 'static) -> Self {
            Self::new(skill_loader, Duration::from_secs(10), ByteSize(u64::MAX))
        }
    }

    /// A testing double that can be loaded up with a particular skill that will always be loaded.
    #[derive(Clone)]
    pub struct SkillStoreStub {
        fetch_response: Option<Arc<dyn Skill>>,
    }

    impl SkillStoreStub {
        pub fn with_fetch_response(fetch_response: Option<Arc<dyn Skill>>) -> Self {
            SkillStoreStub { fetch_response }
        }
    }

    impl SkillStoreApiDouble for SkillStoreStub {
        async fn fetch(
            &self,
            _skill_path: SkillPath,
            _tracing_context: &TracingContext,
        ) -> Result<Option<Arc<dyn Skill>>, SkillStoreError> {
            Ok(self.fetch_response.clone())
        }
    }

    impl SkillStoreState<mpsc::Sender<SkillLoaderMsg>> {
        fn with_namespace_and_skill(skill_path: &SkillPath) -> Self {
            let skill_loader = SkillLoader::with_file_registry(skill_path.namespace.clone()).api();

            let mut provider = SkillStoreState::new(skill_loader, ByteSize(u64::MAX));
            let skill = ConfiguredSkill::from_path(skill_path);
            provider.upsert_skill(skill);
            provider
        }
    }

    #[tokio::test]
    async fn requests_for_same_skill_are_cached() {
        // Given a skill request cache
        let mut requests = SkillRequests::new();
        let first_skill = SkillPath::local("first_skill");
        let second_skill = SkillPath::local("second_skill");
        let first_error =
            SkillFetchError::SkillNotFound(ProgrammableSkill::from_path(&first_skill));
        let second_error =
            SkillFetchError::SkillNotFound(ProgrammableSkill::from_path(&second_skill));

        // When pushing two requests for the same skill to the cache (but their futures return different errors)
        let first_skill_clone = first_skill.clone();
        let first_skill_clone_2 = first_skill.clone();
        let (first_send, first_recv) = oneshot::channel();
        requests.push(
            first_skill.clone(),
            Box::pin(async move { (first_skill_clone, Err(first_error)) }),
            first_send,
        );

        let (second_send, second_recv) = oneshot::channel();
        requests.push(
            first_skill,
            Box::pin(async move { (first_skill_clone_2, Err(second_error)) }),
            second_send,
        );

        // And awaiting the next skill request
        let _error = requests.select_next_some().await;

        // Then both have been answered by the first response
        assert!(
            first_recv
                .await
                .unwrap()
                .map(|_| ())
                .unwrap_err()
                .to_string()
                .contains("first_skill")
        );
        assert!(
            second_recv
                .await
                .unwrap()
                .map(|_| ())
                .unwrap_err()
                .to_string()
                .contains("first_skill")
        );

        // And the cache is empty
        assert!(requests.recipients.is_empty());
        assert!(requests.requests.is_empty());
    }

    #[tokio::test]
    async fn requests_for_different_skills_are_not_cached() {
        // Given a skill request cache
        let mut cache = SkillRequests::new();
        let first_skill = SkillPath::local("first_skill");
        let second_skill = SkillPath::local("second_skill");

        // When pushing two requests for different skills to the cache
        let first_skill_clone = first_skill.clone();
        let (first_send, first_recv) = oneshot::channel();
        cache.push(
            first_skill,
            Box::pin(async move {
                (
                    first_skill_clone.clone(),
                    Err(SkillFetchError::SkillNotFound(
                        ProgrammableSkill::from_path(&first_skill_clone),
                    )),
                )
            }),
            first_send,
        );

        let second_skill_clone = second_skill.clone();
        let (second_send, second_recv) = oneshot::channel();
        cache.push(
            second_skill,
            Box::pin(async move {
                (
                    second_skill_clone.clone(),
                    Err(SkillFetchError::SkillNotFound(
                        ProgrammableSkill::from_path(&second_skill_clone),
                    )),
                )
            }),
            second_send,
        );

        // And awaiting the next skill request
        let _error = cache.select_next_some().await;
        let _error = cache.select_next_some().await;

        // Then the first request has been answered by the first response
        assert!(
            first_recv
                .await
                .unwrap()
                .map(|_| ())
                .unwrap_err()
                .to_string()
                .contains("first_skill")
        );
        assert!(
            second_recv
                .await
                .unwrap()
                .map(|_| ())
                .unwrap_err()
                .to_string()
                .contains("second_skill")
        );

        // And the cache is empty
        assert!(cache.requests.is_empty());
        assert!(cache.recipients.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn skill_store_issues_only_one_request_for_a_skill() {
        // Given a skill store with a configured skill
        let (send, mut recv) = mpsc::channel(2);
        let skill_store = SkillStore::from_loader(send).api();

        let skill_path = SkillPath::local("greet_skill_v0_2");
        let skill = ConfiguredSkill::from_path(&skill_path);
        skill_store.upsert(skill.clone()).await;

        // And given two pending fetch requests
        let cloned_skill_store = skill_store.clone();
        let cloned_skill_path = skill_path.clone();
        let first_request = tokio::spawn(async move {
            cloned_skill_store
                .fetch(cloned_skill_path, &TracingContext::dummy())
                .await
        });

        let cloned_skill_path2 = skill_path.clone();
        let second_request = tokio::spawn(async move {
            skill_store
                .fetch(cloned_skill_path2, &TracingContext::dummy())
                .await
        });

        // And waiting for 10 ms for both requests to be issued
        sleep(Duration::from_millis(10)).await;

        // When answering the first request with an error message
        match recv.recv().await.unwrap() {
            SkillLoaderMsg::Fetch { send, .. } => {
                drop(send.send(Err(SkillFetchError::SkillNotFound(
                    ProgrammableSkill::from_path(&skill_path),
                ))));
            }
            SkillLoaderMsg::FetchDigest { .. } => unreachable!(),
        }

        // Then both requests have answered with the error response
        assert!(
            timeout(Duration::from_millis(10), first_request)
                .await
                .is_ok()
        );
        assert!(
            timeout(Duration::from_millis(10), second_request)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn skills_are_sorted() {
        // Given a provider with two skills
        let (sender, _recv) = mpsc::channel(1);
        let skill_loader = sender;
        let a = Namespace::new("a").unwrap();
        let b = Namespace::new("b").unwrap();

        let skill_store = SkillStore::from_loader(skill_loader).api();
        skill_store
            .upsert(ConfiguredSkill::new(
                a.clone(),
                SkillDescription::Programmable {
                    name: "a".to_owned(),
                    tag: "latest".to_owned(),
                },
            ))
            .await;
        skill_store
            .upsert(ConfiguredSkill::new(
                a,
                SkillDescription::Programmable {
                    name: "b".to_owned(),
                    tag: "latest".to_owned(),
                },
            ))
            .await;
        skill_store
            .upsert(ConfiguredSkill::new(
                b.clone(),
                SkillDescription::Programmable {
                    name: "a".to_owned(),
                    tag: "latest".to_owned(),
                },
            ))
            .await;
        skill_store
            .upsert(ConfiguredSkill::new(
                b,
                SkillDescription::Programmable {
                    name: "b".to_owned(),
                    tag: "latest".to_owned(),
                },
            ))
            .await;

        // When skills are listed
        let skills: Vec<String> = skill_store
            .list(None)
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
        let skill_loader = SkillLoaderStub::new();
        let skill_store = SkillStore::from_loader(skill_loader).api();
        skill_store.upsert(skill).await;

        // When a fetch request is issued for a skill that is configured but not loadable
        let result = skill_store
            .fetch(skill_path, &TracingContext::dummy())
            .await;

        // Then a good error message is returned
        assert!(
            result
                .map(|_| ())
                .unwrap_err()
                .to_string()
                .contains("not found")
        );
    }

    #[tokio::test]
    async fn skill_component_not_in_config() {
        let skill_path = SkillPath::dummy();
        let provider = SkillStoreState::with_namespace_and_skill(&skill_path);

        let result =
            provider.skill_description(&SkillPath::new(skill_path.namespace, "non_existing_skill"));
        assert!(matches!(result, Ok(None)));
    }

    #[tokio::test]
    async fn namespace_not_in_config() {
        // Given a provider with a skill in a particular namespace
        let namespace = Namespace::new("non-existing").unwrap();
        let existing = SkillPath::new(namespace.clone(), "existing_skill");
        let provider = SkillStoreState::with_namespace_and_skill(&existing);

        // When requesting a different, non-existing skill in the same namespace
        let non_existing = SkillPath::new(namespace, "non_existing_skill");
        let result = provider.skill_description(&non_existing);

        // Then the skill is not found
        assert!(matches!(result, Ok(None)));
    }

    #[tokio::test]
    async fn cached_skill_removed() {
        // Given one cached skill
        let skill_path = SkillPath::local("greet");
        let mut provider = SkillStoreState::with_namespace_and_skill(&skill_path);
        provider.insert(
            skill_path.clone(),
            LoadedSkill::new(Arc::new(Dummy), Digest::new("dummy"), ByteSize(1)),
        );

        // When we remove the skill
        provider.remove_skill(&skill_path);

        // Then the skill is no longer cached
        assert!(provider.list_cached_skills().next().is_none());
    }

    #[tokio::test]
    async fn should_error_if_fetching_skill_from_invalid_namespace() {
        // given a skill in an invalid namespace
        let skill_path = SkillPath::local("greet_skill_v0_2");
        let mut provider = SkillStoreState::with_namespace_and_skill(&skill_path);
        provider.add_invalid_namespace(skill_path.namespace.clone(), anyhow!(""));

        // when fetching the tag
        let result = provider.skill_description(&skill_path);

        // then it returns an error
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn should_only_cache_skills_that_have_been_fetched() {
        // Given local is a configured namespace, backed by a file repository with first_skill and
        // second_skill
        let first_skill_path = SkillPath::local("first_skill");
        let second_skill_path = SkillPath::local("second_skill");
        let skill_loader = SkillLoaderStub::new();
        skill_loader.add(&first_skill_path, || {
            LoadedSkill::new(Arc::new(Dummy), Digest::new("first"), ByteSize(1))
        });
        skill_loader.add(&second_skill_path, || {
            LoadedSkill::new(Arc::new(Dummy), Digest::new("second"), ByteSize(1))
        });

        // When
        let skill_store = SkillStore::from_loader(skill_loader);
        let skill = ConfiguredSkill::from_path(&first_skill_path);
        skill_store.api().upsert(skill).await;
        let skill = ConfiguredSkill::from_path(&SkillPath::local("second_skill"));
        skill_store.api().upsert(skill).await;
        // When fetching "greet_skill" but not "greet-py"
        skill_store
            .api()
            .fetch(first_skill_path.clone(), &TracingContext::dummy())
            .await
            .unwrap();
        // and listing all cached skills
        let cached_skills = skill_store.api().list_cached().await;

        // Then only "greet_skill" will appear in that list, but not "greet-py"
        assert_eq!(cached_skills, vec![first_skill_path]);

        // Cleanup
        skill_store.wait_for_shutdown().await;
    }

    #[tokio::test]
    async fn should_list_skills_that_have_been_added() {
        // Given an empty provider
        let first = SkillPath::local("one");
        let second = SkillPath::local("two");

        let skill_loader = SkillLoader::with_file_registry(first.namespace.clone()).api();
        let skill_store = SkillStore::from_loader(skill_loader);
        let api = skill_store.api();

        // When adding two skills
        let skill = ConfiguredSkill::from_path(&first);
        api.upsert(skill).await;

        let skill = ConfiguredSkill::from_path(&second);
        api.upsert(skill).await;
        let skills = api.list(None).await;

        // Then the skills are listed by the skill runtime api
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
        let skill_path = SkillPath::local("greet");
        let skill = ConfiguredSkill::from_path(&skill_path);
        let skill_loader = SkillLoaderStub::new();
        skill_loader.add(&skill_path, || {
            LoadedSkill::new(Arc::new(Dummy), Digest::new("original-digest"), ByteSize(1))
        });
        let skill_store = SkillStore::from_loader(skill_loader);
        let api = skill_store.api();
        api.upsert(skill).await;
        api.fetch(skill_path.clone(), &TracingContext::dummy())
            .await
            .unwrap();

        // When we invalidate "greet_skill"
        let skill_had_been_in_cache = api.invalidate_cache(skill_path.clone()).await;

        // Then greet skill is no longer listed in the cache, but of course still available in the
        // list of all skills
        assert!(skill_had_been_in_cache);
        assert!(api.list_cached().await.is_empty());
        assert_eq!(api.list(None).await, vec![skill_path]);
    }

    #[tokio::test]
    async fn invalidation_of_an_uncached_skill() {
        // Given one "greet_skill" which is not in cache
        let greet_skill = SkillPath::local("greet_skill");
        let skill_loader = SkillLoaderStub::new();
        skill_loader.add(&greet_skill, || {
            LoadedSkill::new(Arc::new(Dummy), Digest::new("dummy digest"), ByteSize(1))
        });
        let skill_store = SkillStore::from_loader(skill_loader);
        let api = skill_store.api();

        let skill = ConfiguredSkill::from_path(&greet_skill);
        api.upsert(skill).await;

        // When we invalidate "greet_skill"
        let skill_had_been_in_cache = api.invalidate_cache(greet_skill.clone()).await;

        // Then greet skill is of course still available in the list of all skills. The return value
        // indicates that greet skill never had been in the cache to begin with
        assert!(!skill_had_been_in_cache);
        assert_eq!(api.list(None).await, vec![greet_skill]);
        // Cleanup
        drop(api);
        skill_store.wait_for_shutdown().await;
    }

    #[tokio::test]
    async fn list_skills_with_skill_type() {
        // Given two skills
        let skill_loader = SkillLoaderStub::new();
        let mut skill_store_state = SkillStoreState::new(skill_loader.clone(), ByteSize(u64::MAX));
        let skill_path = SkillPath::local("foo");
        skill_store_state.upsert_skill(ConfiguredSkill::from_path(&skill_path));
        let skill_path = SkillPath::local("bar");
        let skill = ConfiguredSkill::new(
            skill_path.namespace.clone(),
            SkillDescription::Chat {
                name: skill_path.name.clone(),
                version: "1.0.0".to_owned(),
                model: "pharia-1-llm-7b-control".to_owned(),
                system_prompt: "You are a helpful assistant.".to_owned(),
            },
        );
        skill_store_state.upsert_skill(skill);

        // When listing the skills with Chat skill type
        let skills = skill_store_state
            .skills(Some(&SkillDescriptionFilterType::Chat))
            .collect::<Vec<_>>();

        // Then only the Chat skill is returned
        assert_eq!(skills.len(), 1);
        assert!(skills.contains(&&skill_path));
    }

    #[tokio::test]
    async fn invalidate_cache_skills_after_digest_change() -> anyhow::Result<()> {
        // Given one cached "greet_skill"
        let skill_loader = SkillLoaderStub::new();
        let skill_path = SkillPath::local("greet");
        let mut skill_store_state = SkillStoreState::new(skill_loader.clone(), ByteSize(u64::MAX));
        skill_loader.add(&skill_path, || {
            LoadedSkill::new(Arc::new(Dummy), Digest::new("original-digest"), ByteSize(1))
        });
        skill_store_state.upsert_skill(ConfiguredSkill::from_path(&skill_path));
        skill_store_state.insert(
            skill_path.clone(),
            LoadedSkill::new(Arc::new(Dummy), Digest::new("original-digest"), ByteSize(1)),
        );

        // When we update the digest of the "greet" skill and we clear out expired skills
        skill_loader.add(&skill_path, || {
            LoadedSkill::new(
                Arc::new(Dummy),
                Digest::new("different-digest"),
                ByteSize(1),
            )
        });
        skill_store_state
            .validate_digest(skill_path.clone())
            .await?;

        // Then greet skill is no longer listed in the cache, but of course still available in the
        // list of all skills
        assert_eq!(skill_store_state.list_cached_skills().count(), 0);
        assert_eq!(
            skill_store_state.skills(None).collect::<Vec<_>>(),
            vec![&skill_path]
        );

        Ok(())
    }

    #[tokio::test]
    async fn does_not_invalidate_unchanged_digests() -> anyhow::Result<()> {
        // Given one unchanging "greet" skill inserted into the skill store state
        let skill_path = SkillPath::local("greet");
        let skill_loader = SkillLoaderStub::new();
        skill_loader.add(&skill_path, || {
            // Skill store always returns the same digest
            LoadedSkill::new(
                Arc::new(Dummy),
                Digest::new("originals-digest"),
                ByteSize(1),
            )
        });
        let mut skill_store_state = SkillStoreState::new(skill_loader, ByteSize(u64::MAX));
        let configured_skill = ConfiguredSkill::from_path(&skill_path);
        skill_store_state.upsert_skill(configured_skill.clone());
        let programmable_skill = ProgrammableSkill::from_path(&skill_path);
        let compiled_skill = skill_store_state
            .skill_loader
            .fetch(programmable_skill, TracingContext::dummy())
            .await?;
        skill_store_state.insert(skill_path.clone(), compiled_skill);

        // When we check it with no changes
        skill_store_state.validate_digest(skill_path).await?;

        // Then nothing changes
        assert_eq!(
            skill_store_state.list_cached_skills().collect::<Vec<_>>(),
            skill_store_state.skills(None).cloned().collect::<Vec<_>>(),
        );

        Ok(())
    }

    type SkillFactory = Box<dyn FnMut() -> LoadedSkill + Send>;
    #[derive(Clone)]
    struct SkillLoaderStub {
        skills: Arc<Mutex<HashMap<ProgrammableSkill, SkillFactory>>>,
    }

    impl SkillLoaderStub {
        pub fn new() -> Self {
            SkillLoaderStub {
                skills: Arc::new(Mutex::new(HashMap::new())),
            }
        }

        pub fn add(
            &self,
            path: &SkillPath,
            skill_factory: impl FnMut() -> LoadedSkill + Send + 'static,
        ) {
            let skill = ProgrammableSkill::from_path(path);
            self.skills
                .lock()
                .unwrap()
                .insert(skill, Box::new(skill_factory));
        }
    }

    impl SkillLoaderApi for SkillLoaderStub {
        async fn fetch(
            &self,
            skill: ProgrammableSkill,
            _tracing_context: TracingContext,
        ) -> Result<LoadedSkill, SkillFetchError> {
            self.skills
                .lock()
                .unwrap()
                .get_mut(&skill)
                .map(|f| f())
                .ok_or(SkillFetchError::SkillNotFound(skill))
        }

        async fn fetch_digest(
            &self,
            skill: ProgrammableSkill,
        ) -> Result<Option<Digest>, RegistryError> {
            let maybe_digest = self
                .skills
                .lock()
                .unwrap()
                .get_mut(&skill)
                .map(|f| f().digest);
            Ok(maybe_digest)
        }
    }
}
