use std::collections::{HashMap, HashSet};

use anyhow::Context;
use async_trait::async_trait;
use tokio::{select, task::JoinHandle, time::Duration};
use tracing::error;

use crate::{
    skill_configuration::SkillConfigurationApi, skill_loader::ConfiguredSkill,
    skill_store::SkillStoreApi, skills::SkillPath,
};

use super::{
    namespace_description::{NamespaceDescriptionError, SkillDescription},
    NamespaceDescriptionLoader, OperatorConfig,
};

#[async_trait]
pub trait ObservableConfig {
    fn namespaces(&self) -> Vec<String>;
    async fn skills(
        &mut self,
        namespace: &str,
    ) -> Result<Vec<SkillDescription>, NamespaceDescriptionError>;
}

pub struct NamespaceDescriptionLoaders {
    namespaces: HashMap<String, Box<dyn NamespaceDescriptionLoader + Send>>,
}

impl NamespaceDescriptionLoaders {
    pub fn new(deserialized: OperatorConfig) -> anyhow::Result<Self> {
        let namespaces = deserialized
            .namespaces
            .into_iter()
            .map(|(namespace, config)| {
                config
                    .loader()
                    .with_context(|| {
                        format!("Unable to load configuration of namespace: '{namespace}'")
                    })
                    .map(|loader| (namespace, loader))
            })
            .collect::<anyhow::Result<HashMap<_, _>>>()?;
        Ok(Self { namespaces })
    }
}

#[async_trait]
impl ObservableConfig for NamespaceDescriptionLoaders {
    fn namespaces(&self) -> Vec<String> {
        self.namespaces.keys().cloned().collect()
    }

    async fn skills(
        &mut self,
        namespace: &str,
    ) -> Result<Vec<SkillDescription>, NamespaceDescriptionError> {
        let skills = self
            .namespaces
            .get_mut(namespace)
            .expect("namespace must exist.")
            .description()
            .await?
            .skills;
        Ok(skills)
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
        skill_store_api: SkillStoreApi,
        skill_configuration_api: SkillConfigurationApi,
        config: Box<dyn ObservableConfig + Send>,
        update_interval: Duration,
    ) -> Self {
        let (ready_sender, ready_receiver) = tokio::sync::watch::channel(false);
        let (shutdown_sender, shutdown_receiver) = tokio::sync::watch::channel(false);
        let handle = tokio::spawn(async move {
            NamespaceWatcherActor::new(
                ready_sender,
                shutdown_receiver,
                skill_store_api,
                skill_configuration_api,
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

struct NamespaceWatcherActor {
    ready: tokio::sync::watch::Sender<bool>,
    shutdown: tokio::sync::watch::Receiver<bool>,
    skill_store_api: SkillStoreApi,
    skill_configuration_api: SkillConfigurationApi,
    config: Box<dyn ObservableConfig + Send>,
    update_interval: Duration,
    skills: HashMap<String, Vec<SkillDescription>>,
    invalid_namespaces: HashSet<String>,
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
            .filter_map(|r| {
                if added.iter().all(|a| a.name != r.name) {
                    Some(r.name)
                } else {
                    None
                }
            })
            .collect();
        Self {
            removed,
            added_or_changed: added,
        }
    }
}

impl NamespaceWatcherActor {
    fn new(
        ready: tokio::sync::watch::Sender<bool>,
        shutdown: tokio::sync::watch::Receiver<bool>,
        skill_store_api: SkillStoreApi,
        skill_configuration_api: SkillConfigurationApi,
        config: Box<dyn ObservableConfig + Send>,
        update_interval: Duration,
    ) -> Self {
        Self {
            ready,
            shutdown,
            skill_store_api,
            skill_configuration_api,
            config,
            update_interval,
            skills: HashMap::new(),
            invalid_namespaces: HashSet::new(),
        }
    }

    fn compute_diff(existing: &[SkillDescription], incoming: &[SkillDescription]) -> Diff {
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
        for namespace in &self.config.namespaces() {
            self.report_changes_in_namespace(namespace).await;
        }
    }

    async fn report_changes_in_namespace(&mut self, namespace: &str) {
        let incoming = match self.config.skills(namespace).await {
            Ok(incoming) => {
                if self.invalid_namespaces.contains(namespace) {
                    self.skill_configuration_api
                        .set_namespace_error(namespace.to_owned(), None)
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
                self.skill_configuration_api
                    .set_namespace_error(namespace.to_owned(), Some(e))
                    .await;
                self.invalid_namespaces.insert(namespace.to_owned());
                vec![]
            }
        };
        let existing = self
            .skills
            .insert(namespace.to_owned(), incoming)
            .unwrap_or_default();
        let incoming = self.skills.get(namespace).unwrap();
        let diff = Self::compute_diff(&existing, incoming);
        for skill in diff.added_or_changed {
            let tag = skill.tag.as_deref().unwrap_or("latest");
            let skill = ConfiguredSkill::new(namespace, skill.name, tag);
            self.skill_configuration_api.upsert(skill).await;
        }

        for skill_name in diff.removed {
            self.skill_store_api
                .remove(SkillPath::new(namespace, &skill_name))
                .await;
            self.skill_configuration_api
                .remove(SkillPath::new(namespace, &skill_name))
                .await;
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
    use tokio::sync::{mpsc, Mutex};
    use tokio::time::timeout;

    use crate::namespace_watcher::tests::NamespaceConfig;
    use crate::skill_configuration::SkillConfigurationMsg;
    use crate::skill_store::tests::SkillStoreMessage;
    use crate::skills::SkillPath;

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
        fn namespaces(&self) -> Vec<String> {
            block_on(self.config.lock()).namespaces()
        }

        async fn skills(
            &mut self,
            namespace: &str,
        ) -> Result<Vec<SkillDescription>, NamespaceDescriptionError> {
            self.config.lock().await.skills(namespace).await
        }
    }

    pub struct StubConfig {
        namespaces: HashMap<String, Vec<SkillDescription>>,
    }

    impl StubConfig {
        pub fn new(namespaces: HashMap<String, Vec<SkillDescription>>) -> Self {
            Self { namespaces }
        }
    }

    #[async_trait]
    impl ObservableConfig for StubConfig {
        fn namespaces(&self) -> Vec<String> {
            self.namespaces.keys().cloned().collect()
        }

        async fn skills(
            &mut self,
            namespace: &str,
        ) -> Result<Vec<SkillDescription>, NamespaceDescriptionError> {
            Ok(self
                .namespaces
                .get(namespace)
                .expect("namespace must exist.")
                .clone())
        }
    }

    pub struct PendingConfig;

    #[async_trait]
    impl ObservableConfig for PendingConfig {
        fn namespaces(&self) -> Vec<String> {
            vec!["dummy_namespace".to_owned()]
        }

        async fn skills(
            &mut self,
            _namespace: &str,
        ) -> Result<Vec<SkillDescription>, NamespaceDescriptionError> {
            pending().await
        }
    }

    impl SkillDescription {
        fn with_name(name: &str) -> Self {
            Self::new(name, None)
        }

        fn new(name: &str, tag: Option<&str>) -> Self {
            Self {
                name: name.to_owned(),
                tag: tag.map(ToOwned::to_owned),
            }
        }
    }

    #[test]
    fn diff_is_computed() {
        let incoming = vec![
            SkillDescription::with_name("new_skill"),
            SkillDescription::with_name("existing_skill"),
        ];
        let existing = vec![
            SkillDescription::with_name("existing_skill"),
            SkillDescription::with_name("old_skill"),
        ];

        let diff = NamespaceWatcherActor::compute_diff(&existing, &incoming);

        // when the observer checks for new skills
        assert_eq!(
            diff.added_or_changed,
            vec![SkillDescription::with_name("new_skill")]
        );
        assert_eq!(diff.removed, vec!["old_skill"]);
    }

    #[test]
    fn diff_is_computed_over_versions() {
        // Given a skill has a new version
        let existing = SkillDescription::new("existing_skill", Some("v1"));
        let incoming = SkillDescription::new("existing_skill", Some("v2"));

        // When the observer checks for new skills
        let diff = NamespaceWatcherActor::compute_diff(&[existing.clone()], &[incoming.clone()]);

        // Then the new version is added and the old version is not removed as only the tag changed
        assert_eq!(diff.added_or_changed, vec![incoming]);
        assert!(diff.removed.is_empty());
    }

    #[tokio::test]
    async fn load_config_during_first_pass() {
        // Given a config that take forever to load
        let (sender, _) = mpsc::channel::<SkillStoreMessage>(1);
        let skill_store_api = SkillStoreApi::new(sender);
        let (sender, _) = mpsc::channel::<SkillConfigurationMsg>(1);
        let skill_configuration_api = SkillConfigurationApi::new(sender);
        let config = Box::new(PendingConfig);
        let update_interval = Duration::from_millis(1);
        let mut observer = NamespaceWatcher::with_config(
            skill_store_api,
            skill_configuration_api,
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
            "local".to_owned(),
            NamespaceConfig::Watch {
                directory: temp_dir.path().to_owned(),
            },
        )]
        .into_iter()
        .collect();
        let config = OperatorConfig { namespaces };

        let mut loaders = NamespaceDescriptionLoaders::new(config).unwrap();

        let namespaces = loaders.namespaces();
        assert_eq!(namespaces.len(), 1);
        assert!(loaders.skills(&namespaces[0]).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn watch_skills_in_directory() {
        let temp_dir = tempdir().unwrap();
        let directory = temp_dir.path();
        fs::File::create(directory.join("skill_1.wasm")).unwrap();
        fs::File::create(directory.join("skill_2.wasm")).unwrap();
        let namespaces = [(
            "local".to_owned(),
            NamespaceConfig::Watch {
                directory: directory.to_owned(),
            },
        )]
        .into_iter()
        .collect();
        let config = OperatorConfig { namespaces };

        let mut loaders = NamespaceDescriptionLoaders::new(config).unwrap();

        let namespaces = loaders.namespaces();
        assert_eq!(namespaces.len(), 1);
        let skills = loaders.skills(&namespaces[0]).await.unwrap();
        assert_eq!(skills.len(), 2);
        assert!(skills
            .iter()
            .all(|s| s.name == "skill_1" || s.name == "skill_2"));
    }

    #[tokio::test]
    async fn on_start_reports_all_skills() {
        // Given some configured skills
        let dummy_namespace = "dummy_namespace";
        let dummy_skill = "dummy_skill";
        let update_interval_ms = 1;
        let namespaces = HashMap::from([(
            dummy_namespace.to_owned(),
            vec![SkillDescription::with_name(dummy_skill)],
        )]);
        let stub_config = Box::new(StubConfig::new(namespaces));

        // When we boot up the configuration observer
        let (sender, _) = mpsc::channel::<SkillStoreMessage>(1);
        let skill_store_api = SkillStoreApi::new(sender);
        let (sender, mut receiver) = mpsc::channel::<SkillConfigurationMsg>(1);
        let skill_configuration_api = SkillConfigurationApi::new(sender);
        let update_interval = Duration::from_millis(update_interval_ms);
        let mut observer = NamespaceWatcher::with_config(
            skill_store_api,
            skill_configuration_api,
            stub_config,
            update_interval,
        );
        observer.wait_for_ready().await;

        // Then one new skill message is send for each skill configured
        let msg = receiver.try_recv().unwrap();

        assert!(matches!(
            msg,
            SkillConfigurationMsg::Upsert {
                skill,
            }
            if skill == ConfiguredSkill::new(dummy_namespace, dummy_skill, "latest")
        ));

        observer.wait_for_shutdown().await;
    }

    impl NamespaceWatcherActor {
        fn with_skills(
            skills: HashMap<String, Vec<SkillDescription>>,
            skill_store_api: SkillStoreApi,
            skill_configuration_api: SkillConfigurationApi,
            config: Box<dyn ObservableConfig + Send>,
        ) -> Self {
            let (ready, _) = tokio::sync::watch::channel(false);
            let (_, shutdown) = tokio::sync::watch::channel(false);
            Self {
                ready,
                shutdown,
                skill_store_api,
                skill_configuration_api,
                config,
                update_interval: Duration::from_millis(1),
                skills,
                invalid_namespaces: HashSet::new(),
            }
        }
    }

    struct SaboteurConfig {
        namespaces: Vec<String>,
    }

    impl SaboteurConfig {
        fn new(namespaces: Vec<String>) -> Self {
            Self { namespaces }
        }
    }

    #[async_trait]
    impl ObservableConfig for SaboteurConfig {
        fn namespaces(&self) -> Vec<String> {
            self.namespaces.clone()
        }

        async fn skills(
            &mut self,
            _namespace: &str,
        ) -> Result<Vec<SkillDescription>, NamespaceDescriptionError> {
            Err(NamespaceDescriptionError::Unrecoverable(anyhow!(
                "SaboteurConfig will always fail."
            )))
        }
    }

    #[tokio::test]
    async fn removed_skill_gets_dropped_from_cache_and_configuration() {
        // Given an configuration observer actor with a configured skill
        let dummy_namespace = "dummy_namespace";
        let dummy_skill = "dummy_skill";
        let namespaces = HashMap::from([(
            dummy_namespace.to_owned(),
            vec![SkillDescription::with_name(dummy_skill)],
        )]);

        let (sender, mut store_receiver) = mpsc::channel::<SkillStoreMessage>(2);
        let skill_store_api = SkillStoreApi::new(sender);
        let (sender, mut receiver) = mpsc::channel::<SkillConfigurationMsg>(1);
        let skill_configuration_api = SkillConfigurationApi::new(sender);

        // When we report changes from an empty configuration
        let config = StubConfig::new(HashMap::from([(dummy_namespace.to_owned(), vec![])]));
        let mut watcher = NamespaceWatcherActor::with_skills(
            namespaces,
            skill_store_api,
            skill_configuration_api,
            Box::new(config),
        );
        watcher.report_changes_in_namespace(dummy_namespace).await;

        // Then the skill is removed from the store cache
        let msg = store_receiver.try_recv().unwrap();

        assert!(
            matches!(msg, SkillStoreMessage::Remove{ skill_path } if skill_path == SkillPath::new(dummy_namespace, dummy_skill))
        );

        // And the skill is removed from the configuration
        let msg = receiver.try_recv().unwrap();

        assert!(matches!(
            msg,
            SkillConfigurationMsg::Remove { skill_path } if skill_path == SkillPath::new(dummy_namespace, dummy_skill)
        ));
    }

    #[tokio::test]
    async fn add_invalid_namespace_and_unload_skill_for_invalid_namespace_description() {
        // given an configuration observer actor
        let dummy_namespace = "dummy_namespace";
        let dummy_skill = "dummy_skill";
        let namespaces = HashMap::from([(
            dummy_namespace.to_owned(),
            vec![SkillDescription::with_name(dummy_skill)],
        )]);

        let (sender, mut store_receiver) = mpsc::channel::<SkillStoreMessage>(1);
        let skill_store_api = SkillStoreApi::new(sender);
        let (sender, mut receiver) = mpsc::channel::<SkillConfigurationMsg>(2);
        let skill_configuration_api = SkillConfigurationApi::new(sender);
        let config = Box::new(SaboteurConfig::new(vec![dummy_namespace.to_owned()]));

        let mut watcher = NamespaceWatcherActor::with_skills(
            namespaces,
            skill_store_api,
            skill_configuration_api,
            config,
        );

        // when we load an invalid namespace
        watcher.report_changes_in_namespace(dummy_namespace).await;

        // then mark the namespace as invalid
        let msg = receiver.try_recv().unwrap();

        assert!(matches!(
            msg,
            SkillConfigurationMsg::SetNamespaceError {
                namespace, ..
            }
            if namespace == dummy_namespace
        ));

        // and drop all skills of that namespace from the store cache
        let msg = store_receiver.try_recv().unwrap();

        assert!(matches!(
            msg,
            SkillStoreMessage::Remove { skill_path } if skill_path == SkillPath::new(dummy_namespace, dummy_skill)
        ));

        // and remove all skills of that namespace from the configuration
        let msg = receiver.try_recv().unwrap();

        assert!(matches!(
            msg,
            SkillConfigurationMsg::Remove {
                skill_path
            }
            if skill_path == SkillPath::new(dummy_namespace, dummy_skill)
        ));
    }

    #[tokio::test]
    async fn new_skill_only_reported_once() {
        // Given some configured skills
        let dummy_namespace = "dummy_namespace";
        let dummy_skill = "dummy_skill";
        let update_interval_ms = 1;
        let namespaces = HashMap::from([(
            dummy_namespace.to_owned(),
            vec![SkillDescription::with_name(dummy_skill)],
        )]);
        let stub_config = Box::new(StubConfig::new(namespaces));

        // When we boot up the configuration observer
        let (sender, _) = mpsc::channel::<SkillStoreMessage>(1);
        let skill_store_api = SkillStoreApi::new(sender);
        let (sender, mut receiver) = mpsc::channel::<SkillConfigurationMsg>(1);
        let skill_configuration_api = SkillConfigurationApi::new(sender);
        let update_interval = Duration::from_millis(update_interval_ms);
        let observer = NamespaceWatcher::with_config(
            skill_store_api,
            skill_configuration_api,
            stub_config,
            update_interval,
        );

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
        let (sender, _) = mpsc::channel::<SkillStoreMessage>(2);
        let skill_store_api = SkillStoreApi::new(sender);
        let (sender, mut receiver) = mpsc::channel::<SkillConfigurationMsg>(1);
        let skill_configuration_api = SkillConfigurationApi::new(sender);
        let update_interval_ms = 1;
        let update_interval = Duration::from_millis(update_interval_ms);
        let dummy_namespace = "dummy_namespace";
        let config_arc: Arc<Mutex<Box<dyn ObservableConfig + Send>>> =
            Arc::new(Mutex::new(Box::new(SaboteurConfig::new(vec![
                dummy_namespace.to_owned(),
            ]))));
        let config_arc_clone = Arc::clone(&config_arc);
        let config = Box::new(UpdatableConfig::new(config_arc));
        let mut observer = NamespaceWatcher::with_config(
            skill_store_api,
            skill_configuration_api,
            config,
            update_interval,
        );
        observer.wait_for_ready().await;
        receiver.recv().await.unwrap();

        // when the namespace become valid
        let dummy_skill = "dummy_skill";
        let namespaces = HashMap::from([(
            dummy_namespace.to_owned(),
            vec![SkillDescription::with_name(dummy_skill)],
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
            SkillConfigurationMsg::SetNamespaceError {
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
            SkillConfigurationMsg::Upsert {
                skill,
            }
            if skill == ConfiguredSkill::new(dummy_namespace, dummy_skill, "latest")
        ));
    }
}
