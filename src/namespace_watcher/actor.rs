use std::collections::{HashMap, HashSet};

use anyhow::Context;
use async_trait::async_trait;
use tokio::{select, task::JoinHandle, time::Duration};
use tracing::error;

use crate::{
    skill_loader::ConfiguredSkill,
    skill_store::SkillStoreApi,
    skills::SkillPath,
    tool::{McpServerUrl, ToolStoreApi},
};

use super::{
    Namespace, NamespaceConfigs, NamespaceDescriptionLoader,
    namespace_description::{NamespaceDescription, NamespaceDescriptionError, SkillDescription},
};

#[async_trait]
pub trait ObservableConfig {
    fn namespaces(&self) -> Vec<Namespace>;
    async fn description(
        &self,
        namespace: &Namespace,
    ) -> Result<NamespaceDescription, NamespaceDescriptionError>;
}

pub struct NamespaceDescriptionLoaders {
    namespaces: HashMap<Namespace, Box<dyn NamespaceDescriptionLoader + Send + Sync>>,
    beta: bool,
}

impl NamespaceDescriptionLoaders {
    pub fn new(deserialized: NamespaceConfigs, beta: bool) -> anyhow::Result<Self> {
        let namespaces = deserialized
            .into_iter()
            .map(|(namespace, config)| {
                config
                    .loader()
                    .with_context(|| {
                        format!("Unable to load configuration of namespace: '{namespace:?}'")
                    })
                    .map(|loader| (namespace, loader))
            })
            .collect::<anyhow::Result<HashMap<_, _>>>()?;
        Ok(Self { namespaces, beta })
    }
}

#[async_trait]
impl ObservableConfig for NamespaceDescriptionLoaders {
    fn namespaces(&self) -> Vec<Namespace> {
        self.namespaces.keys().cloned().collect()
    }

    async fn description(
        &self,
        namespace: &Namespace,
    ) -> Result<NamespaceDescription, NamespaceDescriptionError> {
        self.namespaces
            .get(namespace)
            .expect("namespace must exist.")
            .description(self.beta)
            .await
    }
}

/// Watches for changes in namespaces configuration. In particular which skills should currently be
/// served. These changes are communicated back to the skill provider, so it would serve the version
/// the skill operator intends to serve from then on. This enables Skill Operators to roll out
/// skills at runtime in self service.
pub struct NamespaceWatcher {
    ready: tokio::sync::watch::Receiver<bool>,
    shutdown: tokio::sync::watch::Sender<bool>,
    handle: JoinHandle<()>,
}

impl NamespaceWatcher {
    /// Completes after attempted to load all config once.
    /// This ensures that the requests are only accepted after initialization.
    pub async fn wait_for_ready(&mut self) {
        self.ready.wait_for(|ready| *ready).await.unwrap();
    }

    pub fn with_config(
        skill_store_api: impl SkillStoreApi + Send + Sync + 'static,
        tool_api: impl ToolStoreApi + Send + 'static,
        config: Box<dyn ObservableConfig + Send + Sync>,
        update_interval: Duration,
    ) -> Self {
        let (ready_sender, ready_receiver) = tokio::sync::watch::channel(false);
        let (shutdown_sender, shutdown_receiver) = tokio::sync::watch::channel(false);
        let handle = tokio::spawn(async move {
            NamespaceWatcherActor::new(
                ready_sender,
                shutdown_receiver,
                skill_store_api,
                tool_api,
                config,
                update_interval,
            )
            .run()
            .await;
        });
        Self {
            ready: ready_receiver,
            shutdown: shutdown_sender,
            handle,
        }
    }

    pub async fn wait_for_shutdown(self) {
        self.shutdown.send(true).unwrap();
        self.handle.await.unwrap();
    }
}

struct NamespaceWatcherActor<S, T> {
    ready: tokio::sync::watch::Sender<bool>,
    shutdown: tokio::sync::watch::Receiver<bool>,
    skill_store_api: S,
    tool_store_api: T,
    config: Box<dyn ObservableConfig + Send + Sync>,
    update_interval: Duration,
    descriptions: HashMap<Namespace, NamespaceDescription>,
    invalid_namespaces: HashSet<Namespace>,
}

/// Keep track of changes that need to be propagated to the skill provider.
#[derive(Debug)]
struct Diff {
    added_or_changed: Vec<SkillDescription>,
    removed: Vec<String>,
}

impl Diff {
    fn new(added: Vec<SkillDescription>, removed: Vec<SkillDescription>) -> Self {
        // Do not list skills as removed if only the tag changed.
        let removed = removed
            .into_iter()
            .filter_map(|r| match r {
                SkillDescription::Chat { name, .. }
                | SkillDescription::Programmable { name, .. } => {
                    if added.iter().all(|a| match a {
                        SkillDescription::Chat { name: a_name, .. }
                        | SkillDescription::Programmable { name: a_name, .. } => {
                            a_name != name.as_str()
                        }
                    }) {
                        Some(name)
                    } else {
                        None
                    }
                }
            })
            .collect();
        Self {
            removed,
            added_or_changed: added,
        }
    }
}

#[derive(Debug)]
struct McpServerDiff {
    added: Vec<McpServerUrl>,
    removed: Vec<McpServerUrl>,
}

impl McpServerDiff {
    fn new(added: Vec<McpServerUrl>, removed: Vec<McpServerUrl>) -> Self {
        Self { added, removed }
    }

    fn compute(existing: &[McpServerUrl], incoming: &[McpServerUrl]) -> Self {
        let existing = existing.iter().collect::<HashSet<_>>();
        let incoming = incoming.iter().collect::<HashSet<_>>();

        let added = incoming
            .difference(&existing)
            .map(|&url| url.clone())
            .collect();
        let removed = existing
            .difference(&incoming)
            .map(|&url| url.clone())
            .collect();
        Self::new(added, removed)
    }
}

impl<S, T> NamespaceWatcherActor<S, T>
where
    S: SkillStoreApi,
    T: ToolStoreApi,
{
    fn new(
        ready: tokio::sync::watch::Sender<bool>,
        shutdown: tokio::sync::watch::Receiver<bool>,
        skill_store_api: S,
        tool_store_api: T,
        config: Box<dyn ObservableConfig + Send + Sync>,
        update_interval: Duration,
    ) -> Self {
        Self {
            ready,
            shutdown,
            skill_store_api,
            tool_store_api,
            config,
            update_interval,
            descriptions: HashMap::new(),
            invalid_namespaces: HashSet::new(),
        }
    }

    fn compute_skill_diff(existing: &[SkillDescription], incoming: &[SkillDescription]) -> Diff {
        let existing = existing.iter().collect::<HashSet<_>>();
        let incoming = incoming.iter().collect::<HashSet<_>>();

        let added = incoming
            .difference(&existing)
            .map(|&skill| skill.clone())
            .collect();
        let removed = existing
            .difference(&incoming)
            .map(|&skill| skill.clone())
            .collect();

        Diff::new(added, removed)
    }

    async fn run(mut self) {
        let mut started = tokio::time::Instant::now();
        self.report_all_changes().await;
        let _ = self.ready.send(true);
        loop {
            select! {
                _ = self.shutdown.changed() => break,
                () = tokio::time::sleep_until(started + self.update_interval) => (),
            };
            started = tokio::time::Instant::now();
            self.report_all_changes().await;
        }
    }

    async fn report_all_changes(&mut self) {
        let futures = self.config.namespaces().into_iter().map(async |namespace| {
            let descriptions = self.config.description(&namespace).await;
            (namespace, descriptions)
        });
        // While it would be nice to use a stream and update the state after each future has finished,
        // this would only work if all the members except config go into a member object.
        // If they are top level, we can not obtain a exclusive reference, as we already have shared references to them in the futures.
        for (namespace, skills) in futures::future::join_all(futures).await {
            self.report_changes_in_namespace(&namespace, skills).await;
        }
    }

    async fn report_changes_in_namespace(
        &mut self,
        namespace: &Namespace,
        description: Result<NamespaceDescription, NamespaceDescriptionError>,
    ) {
        let incoming = match description {
            Ok(incoming) => {
                if self.invalid_namespaces.contains(namespace) {
                    self.skill_store_api
                        .set_namespace_error(namespace.clone(), None)
                        .await;
                    self.invalid_namespaces.remove(namespace);
                }
                incoming
            }
            Err(NamespaceDescriptionError::Recoverable(e)) => {
                error!(
                    "Failed to get the skills in namespace {namespace}, fallback to existing state, caused by: {e}"
                );
                return;
            }
            Err(NamespaceDescriptionError::Unrecoverable(e)) => {
                error!(
                    "Failed to get the skills in namespace {namespace}, mark it as invalid and unload all skills, caused by: {e}"
                );
                self.skill_store_api
                    .set_namespace_error(namespace.clone(), Some(e))
                    .await;
                self.invalid_namespaces.insert(namespace.to_owned());
                NamespaceDescription::empty()
            }
        };
        let existing = self
            .descriptions
            .insert(namespace.to_owned(), incoming)
            .unwrap_or_default();
        let incoming = self.descriptions.get(namespace).unwrap();

        // propagate skill changes
        let diff = Self::compute_skill_diff(&existing.skills, &incoming.skills);
        for skill in diff.added_or_changed {
            let skill = ConfiguredSkill::new(namespace.clone(), skill);
            self.skill_store_api.upsert(skill).await;
        }
        for skill_name in diff.removed {
            self.skill_store_api
                .remove(SkillPath::new(namespace.clone(), &skill_name))
                .await;
        }

        // propagate mcp server changes
        let mcp_server_diff = McpServerDiff::compute(&existing.mcp_servers, &incoming.mcp_servers);
        for mcp_server in mcp_server_diff.added {
            self.tool_store_api.upsert_tool_server(mcp_server).await;
        }
        for mcp_server in mcp_server_diff.removed {
            self.tool_store_api.remove_tool_server(mcp_server).await;
        }
    }
}

#[cfg(test)]
pub mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::future::pending;
    use std::sync::Arc;

    use futures::executor::block_on;
    use tempfile::tempdir;
    use tokio::sync::{Mutex, mpsc};
    use tokio::time::timeout;

    use crate::skill_store::tests::SkillStoreDummy;
    use crate::tool::McpServerUrl;
    use crate::tool::tests::ToolStoreDouble;
    use crate::{
        namespace_watcher::{config::Namespace, tests::NamespaceConfig},
        skill_store::tests::SkillStoreMsg,
        skills::SkillPath,
    };

    use super::*;
    use anyhow::anyhow;

    pub struct UpdatableConfig {
        config: Arc<Mutex<Box<dyn ObservableConfig + Send>>>,
    }

    impl UpdatableConfig {
        pub fn new(config: Arc<Mutex<Box<dyn ObservableConfig + Send>>>) -> Self {
            Self { config }
        }
    }

    #[async_trait]
    impl ObservableConfig for UpdatableConfig {
        fn namespaces(&self) -> Vec<Namespace> {
            block_on(self.config.lock()).namespaces()
        }

        async fn description(
            &self,
            namespace: &Namespace,
        ) -> Result<NamespaceDescription, NamespaceDescriptionError> {
            self.config.lock().await.description(namespace).await
        }
    }

    pub struct StubConfig {
        namespaces: HashMap<Namespace, Vec<SkillDescription>>,
    }

    impl StubConfig {
        pub fn new(namespaces: HashMap<Namespace, Vec<SkillDescription>>) -> Self {
            Self { namespaces }
        }
    }

    #[async_trait]
    impl ObservableConfig for StubConfig {
        fn namespaces(&self) -> Vec<Namespace> {
            self.namespaces.keys().cloned().collect()
        }

        async fn description(
            &self,
            namespace: &Namespace,
        ) -> Result<NamespaceDescription, NamespaceDescriptionError> {
            Ok(NamespaceDescription {
                skills: self
                    .namespaces
                    .get(namespace)
                    .expect("namespace must exist.")
                    .clone(),
                mcp_servers: vec![],
            })
        }
    }

    pub struct PendingConfig;

    #[async_trait]
    impl ObservableConfig for PendingConfig {
        fn namespaces(&self) -> Vec<Namespace> {
            vec![Namespace::new("dummy-namespace").unwrap()]
        }

        async fn description(
            &self,
            _namespace: &Namespace,
        ) -> Result<NamespaceDescription, NamespaceDescriptionError> {
            pending().await
        }
    }

    #[test]
    fn mcp_server_diff_is_computed() {
        let existing = vec![
            "http://localhost:8000/mcp".into(),
            "http://localhost:8001/mcp".into(),
        ];
        let incoming = vec![
            "http://localhost:8000/mcp".into(),
            "http://localhost:8002/mcp".into(),
        ];
        let diff = McpServerDiff::compute(&existing, &incoming);
        assert_eq!(diff.added, vec!["http://localhost:8002/mcp".into()]);
        assert_eq!(diff.removed, vec!["http://localhost:8001/mcp".into()]);
    }

    #[test]
    fn diff_is_computed() {
        let incoming = vec![
            SkillDescription::Programmable {
                name: "new_skill".to_owned(),
                tag: "latest".to_owned(),
            },
            SkillDescription::Programmable {
                name: "existing_skill".to_owned(),
                tag: "latest".to_owned(),
            },
        ];
        let existing = vec![
            SkillDescription::Programmable {
                name: "existing_skill".to_owned(),
                tag: "latest".to_owned(),
            },
            SkillDescription::Programmable {
                name: "old_skill".to_owned(),
                tag: "latest".to_owned(),
            },
        ];

        let diff = NamespaceWatcherActor::<SkillStoreDummy, ToolStoreDouble>::compute_skill_diff(
            &existing, &incoming,
        );

        // when the observer checks for new skills
        assert_eq!(
            diff.added_or_changed,
            vec![SkillDescription::Programmable {
                name: "new_skill".to_owned(),
                tag: "latest".to_owned()
            }]
        );
        assert_eq!(diff.removed, vec!["old_skill"]);
    }

    #[test]
    fn diff_is_computed_over_versions() {
        // Given a skill has a new version
        let existing = SkillDescription::Programmable {
            name: "existing_skill".to_owned(),
            tag: "v1".to_owned(),
        };
        let incoming = SkillDescription::Programmable {
            name: "existing_skill".to_owned(),
            tag: "v2".to_owned(),
        };

        // When the observer checks for new skills
        let diff = NamespaceWatcherActor::<SkillStoreDummy, ToolStoreDouble>::compute_skill_diff(
            &[existing.clone()],
            &[incoming.clone()],
        );

        // Then the new version is added and the old version is not removed as only the tag changed
        assert_eq!(diff.added_or_changed, vec![incoming]);
        assert!(diff.removed.is_empty());
    }

    #[tokio::test]
    async fn load_config_during_first_pass() {
        // Given a config that take forever to load
        let config = Box::new(PendingConfig);
        let update_interval = Duration::from_millis(1);
        let mut observer = NamespaceWatcher::with_config(
            SkillStoreDummy,
            ToolStoreDouble,
            config,
            update_interval,
        );

        // When waiting for the first pass
        let result = tokio::time::timeout(Duration::from_secs(1), observer.wait_for_ready()).await;

        // Then it will timeout
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn watch_skills_in_empty_directory() {
        let temp_dir = tempdir().unwrap();
        let namespaces = [(
            Namespace::new("local").unwrap(),
            NamespaceConfig::Watch {
                directory: temp_dir.path().to_owned(),
                mcp_servers: vec![],
            },
        )]
        .into_iter()
        .collect();
        let config = NamespaceConfigs::new(namespaces);

        let loaders = NamespaceDescriptionLoaders::new(config, false).unwrap();

        let namespaces = loaders.namespaces();
        assert_eq!(namespaces.len(), 1);
        assert!(
            loaders
                .description(&namespaces[0])
                .await
                .unwrap()
                .skills
                .is_empty()
        );
    }

    #[tokio::test]
    async fn watch_skills_in_directory() {
        let temp_dir = tempdir().unwrap();
        let directory = temp_dir.path();
        fs::File::create(directory.join("skill_1.wasm")).unwrap();
        fs::File::create(directory.join("skill_2.wasm")).unwrap();
        let namespaces = [(
            Namespace::new("local").unwrap(),
            NamespaceConfig::Watch {
                directory: directory.to_owned(),
                mcp_servers: vec![],
            },
        )]
        .into_iter()
        .collect();
        let config = NamespaceConfigs::new(namespaces);

        let loaders = NamespaceDescriptionLoaders::new(config, false).unwrap();

        let namespaces = loaders.namespaces();
        assert_eq!(namespaces.len(), 1);

        let skills = loaders.description(&namespaces[0]).await.unwrap().skills;
        assert_eq!(skills.len(), 2);
        assert!(matches!(&skills[0], SkillDescription::Programmable { .. }));
        assert!(matches!(&skills[1], SkillDescription::Programmable { .. }));
    }

    #[tokio::test]
    async fn on_start_reports_all_skills() {
        // Given some configured skills
        let dummy_namespace = Namespace::new("dummy-namespace").unwrap();
        let dummy_skill = "dummy_skill";
        let update_interval_ms = 1;
        let namespaces = HashMap::from([(
            dummy_namespace.clone(),
            vec![SkillDescription::Programmable {
                name: dummy_skill.to_owned(),
                tag: "latest".to_owned(),
            }],
        )]);
        let stub_config = Box::new(StubConfig::new(namespaces));

        // When we boot up the configuration observer
        let (sender, mut receiver) = mpsc::channel::<SkillStoreMsg>(1);
        let update_interval = Duration::from_millis(update_interval_ms);
        let mut observer =
            NamespaceWatcher::with_config(sender, ToolStoreDouble, stub_config, update_interval);
        observer.wait_for_ready().await;

        // Then one new skill message is send for each skill configured
        let msg = receiver.try_recv().unwrap();

        assert!(matches!(
            msg,
            SkillStoreMsg::Upsert {
                skill,
            }
            if skill == ConfiguredSkill::new(
                dummy_namespace,
                SkillDescription::Programmable { name: dummy_skill.to_owned(), tag: "latest".to_owned() }
            )
        ));

        observer.wait_for_shutdown().await;
    }

    impl<S> NamespaceWatcherActor<S, ToolStoreDouble>
    where
        S: SkillStoreApi + Send + Sync,
    {
        fn with_skill_store_api(
            descriptions: HashMap<Namespace, NamespaceDescription>,
            skill_store_api: S,
            config: Box<dyn ObservableConfig + Send + Sync>,
        ) -> Self {
            let (ready, _) = tokio::sync::watch::channel(false);
            let (_, shutdown) = tokio::sync::watch::channel(false);
            Self {
                ready,
                shutdown,
                skill_store_api,
                tool_store_api: ToolStoreDouble,
                config,
                update_interval: Duration::from_millis(1),
                descriptions,
                invalid_namespaces: HashSet::new(),
            }
        }
    }

    impl<T> NamespaceWatcherActor<SkillStoreDummy, T>
    where
        T: ToolStoreApi + Send + Sync,
    {
        fn with_tool_store_api(
            descriptions: HashMap<Namespace, NamespaceDescription>,
            tool_store_api: T,
            config: Box<dyn ObservableConfig + Send + Sync>,
        ) -> Self {
            let (ready, _) = tokio::sync::watch::channel(false);
            let (_, shutdown) = tokio::sync::watch::channel(false);
            Self {
                ready,
                shutdown,
                skill_store_api: SkillStoreDummy,
                tool_store_api,
                config,
                update_interval: Duration::from_millis(1),
                descriptions,
                invalid_namespaces: HashSet::new(),
            }
        }
    }

    struct SaboteurConfig {
        namespaces: Vec<Namespace>,
    }

    impl SaboteurConfig {
        fn new(namespaces: Vec<Namespace>) -> Self {
            Self { namespaces }
        }
    }

    #[async_trait]
    impl ObservableConfig for SaboteurConfig {
        fn namespaces(&self) -> Vec<Namespace> {
            self.namespaces.clone()
        }

        async fn description(
            &self,
            _namespace: &Namespace,
        ) -> Result<NamespaceDescription, NamespaceDescriptionError> {
            Err(NamespaceDescriptionError::Unrecoverable(anyhow!(
                "SaboteurConfig will always fail."
            )))
        }
    }

    impl ToolStoreApi for Arc<Mutex<Vec<McpServerUrl>>> {
        async fn upsert_tool_server(&self, url: McpServerUrl) {
            self.lock().await.push(url);
        }

        async fn remove_tool_server(&self, url: McpServerUrl) {
            self.lock().await.retain(|u| u != &url);
        }

        async fn list_tool_servers(&self) -> Vec<McpServerUrl> {
            self.lock().await.clone()
        }
    }

    #[tokio::test]
    async fn new_mcp_server_is_upserted() {
        // Given a namespace description watcher with empty descriptions
        let tool_store_api = Arc::new(Mutex::new(vec![]));
        let descriptions = HashMap::new();
        let config = Box::new(PendingConfig);
        let mut watcher = NamespaceWatcherActor::with_tool_store_api(
            descriptions,
            tool_store_api.clone(),
            config,
        );

        // When observing a namespace with an mcp server
        let namespace = Namespace::new("dummy-namespace").unwrap();
        let descriptions = NamespaceDescription {
            skills: vec![],
            mcp_servers: vec!["http://localhost:8000/mcp".into()],
        };
        watcher
            .report_changes_in_namespace(&namespace, Ok(descriptions))
            .await;

        // Then the new mcp servers is upserted
        let stored = tool_store_api.list_tool_servers().await;
        assert_eq!(stored, vec!["http://localhost:8000/mcp".into()]);
    }

    #[tokio::test]
    async fn dropped_mcp_server_is_removed() {
        // Given a namespace description watcher and a tool store that both know about an mcp server
        let mcp_servers = vec!["http://localhost:8000/mcp".into()];
        let tool_store = Arc::new(Mutex::new(mcp_servers.clone()));
        let namespace = Namespace::new("dummy-namespace").unwrap();
        let descriptions = HashMap::from([(
            namespace.clone(),
            NamespaceDescription {
                skills: vec![],
                mcp_servers,
            },
        )]);
        let config = Box::new(PendingConfig);
        let mut watcher =
            NamespaceWatcherActor::with_tool_store_api(descriptions, tool_store.clone(), config);

        // When the watcher observes a namespace with no mcp servers
        let namespace = Namespace::new("dummy-namespace").unwrap();
        let descriptions = NamespaceDescription {
            skills: vec![],
            mcp_servers: vec![],
        };
        watcher
            .report_changes_in_namespace(&namespace, Ok(descriptions))
            .await;

        // Then the tool store is notified that the mcp server is removed
        let stored = tool_store.list_tool_servers().await;
        assert_eq!(stored, vec![]);
    }

    #[tokio::test]
    async fn add_invalid_namespace_and_unload_skill_for_invalid_namespace_description() {
        // given an configuration observer actor
        let dummy_namespace = Namespace::new("dummy-namespace").unwrap();
        let dummy_skill = "dummy_skill";
        let namespaces = HashMap::from([(
            dummy_namespace.clone(),
            NamespaceDescription {
                skills: vec![SkillDescription::Programmable {
                    name: dummy_skill.to_owned(),
                    tag: "latest".to_owned(),
                }],
                mcp_servers: vec![],
            },
        )]);

        let (sender, mut receiver) = mpsc::channel::<SkillStoreMsg>(2);
        let config = Box::new(SaboteurConfig::new(vec![dummy_namespace.clone()]));

        let mut watcher = NamespaceWatcherActor::with_skill_store_api(namespaces, sender, config);

        // when we load an invalid namespace
        let description = watcher.config.description(&dummy_namespace).await;
        watcher
            .report_changes_in_namespace(&dummy_namespace, description)
            .await;

        // then mark the namespace as invalid and remove all skills of that namespace
        let msg = receiver.try_recv().unwrap();

        assert!(matches!(
            msg,
            SkillStoreMsg::SetNamespaceError {
                namespace, ..
            }
            if namespace == dummy_namespace
        ));

        let msg = receiver.try_recv().unwrap();

        assert!(matches!(
            msg,
            SkillStoreMsg::Remove {
                skill_path
            }
            if skill_path == SkillPath::new(dummy_namespace, dummy_skill)
        ));
    }

    #[tokio::test]
    async fn new_skill_only_reported_once() {
        // Given some configured skills
        let dummy_namespace = Namespace::new("dummy-namespace").unwrap();
        let dummy_skill = "dummy_skill";
        let update_interval_ms = 1;
        let namespaces = HashMap::from([(
            dummy_namespace,
            vec![SkillDescription::Programmable {
                name: dummy_skill.to_owned(),
                tag: "latest".to_owned(),
            }],
        )]);
        let stub_config = Box::new(StubConfig::new(namespaces));

        // When we boot up the configuration observer
        let (sender, mut receiver) = mpsc::channel::<SkillStoreMsg>(1);
        let update_interval = Duration::from_millis(update_interval_ms);
        let observer =
            NamespaceWatcher::with_config(sender, ToolStoreDouble, stub_config, update_interval);

        // Then only one new skill message is send for each skill configured
        receiver.recv().await.unwrap();

        let next_msg = timeout(
            Duration::from_millis(update_interval_ms + 10),
            receiver.recv(),
        )
        .await;

        assert!(next_msg.is_err());

        observer.wait_for_shutdown().await;
    }

    #[tokio::test]
    async fn remove_invalid_namespace_if_namespace_become_valid() {
        // given an invalid namespace
        let (sender, mut receiver) = mpsc::channel::<SkillStoreMsg>(2);
        let update_interval_ms = 1;
        let update_interval = Duration::from_millis(update_interval_ms);
        let dummy_namespace = Namespace::new("dummy-namespace").unwrap();
        let config_arc: Arc<Mutex<Box<dyn ObservableConfig + Send>>> =
            Arc::new(Mutex::new(Box::new(SaboteurConfig::new(vec![
                dummy_namespace.clone(),
            ]))));
        let config_arc_clone = Arc::clone(&config_arc);
        let config = Box::new(UpdatableConfig::new(config_arc));
        let mut observer =
            NamespaceWatcher::with_config(sender, ToolStoreDouble, config, update_interval);
        observer.wait_for_ready().await;
        receiver.recv().await.unwrap();

        // when the namespace become valid
        let dummy_skill = "dummy_skill";
        let namespaces = HashMap::from([(
            dummy_namespace.clone(),
            vec![SkillDescription::Programmable {
                name: dummy_skill.to_owned(),
                tag: "latest".to_owned(),
            }],
        )]);
        let stub_config = Box::new(StubConfig::new(namespaces));
        *config_arc_clone.lock().await = stub_config;

        // then we expect the namespace is no longer invalid and its skills are added
        let msg = timeout(
            Duration::from_millis(update_interval_ms + 100),
            receiver.recv(),
        )
        .await
        .unwrap()
        .unwrap();

        assert!(matches!(
             msg,
            SkillStoreMsg::SetNamespaceError {
                namespace, error: None
            }
            if namespace == dummy_namespace
        ));

        let msg = timeout(
            Duration::from_millis(update_interval_ms + 100),
            receiver.recv(),
        )
        .await
        .unwrap()
        .unwrap();

        assert!(matches!(
            msg,
            SkillStoreMsg::Upsert {
                skill,
            }
            if skill == ConfiguredSkill::new(
                dummy_namespace,
                SkillDescription::Programmable { name: dummy_skill.to_owned(), tag: "latest".to_owned() }
            )
        ));
    }
}
