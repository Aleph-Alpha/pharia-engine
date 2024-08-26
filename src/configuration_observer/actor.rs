use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use tokio::{select, task::JoinHandle, time::Duration};
use tracing::error;

use crate::skills::{SkillExecutorApi, SkillPath};

use super::{
    namespace_description::Skill, namespace_from_url, NamespaceConfig, NamespaceDescriptionLoader,
    OperatorConfig,
};

#[async_trait]
pub trait ObservableConfig {
    fn namespaces(&self) -> Vec<String>;
    async fn skills(&mut self, namespace: &str) -> anyhow::Result<Vec<Skill>>;
}

pub struct NamespaceDescriptionLoaders {
    namespaces: HashMap<String, Box<dyn NamespaceDescriptionLoader + Send>>,
}

impl NamespaceDescriptionLoaders {
    pub fn new(deserialized: OperatorConfig) -> anyhow::Result<Self> {
        let namespaces = deserialized
            .namespaces
            .into_iter()
            .map(|(namespace, config)| match config {
                NamespaceConfig::File { config_url, .. }
                | NamespaceConfig::Oci { config_url, .. } => {
                    Ok((namespace, namespace_from_url(&config_url)?))
                }
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

    async fn skills(&mut self, namespace: &str) -> anyhow::Result<Vec<Skill>> {
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

/// Periodically observes changes in remote repositories containing
/// skill configurations and reports detected changes to the skill executor
pub struct ConfigurationObserver {
    ready: tokio::sync::watch::Receiver<bool>,
    shutdown: tokio::sync::watch::Sender<bool>,
    handle: JoinHandle<()>,
}

impl ConfigurationObserver {
    /// Completes after attempted to load all config once.
    /// This ensures that the requests are only accepted after initialization.
    pub async fn wait_for_ready(&mut self) {
        self.ready.wait_for(|ready| *ready).await.unwrap();
    }

    pub fn with_config(
        skill_executor_api: SkillExecutorApi,
        config: Box<dyn ObservableConfig + Send>,
        update_interval: Duration,
    ) -> Self {
        let (ready_sender, ready_receiver) = tokio::sync::watch::channel(false);
        let (shutdown_sender, shutdown_receiver) = tokio::sync::watch::channel(false);
        let handle = tokio::spawn(async move {
            ConfigurationObserverActor::new(
                ready_sender,
                shutdown_receiver,
                skill_executor_api,
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

struct ConfigurationObserverActor {
    ready: tokio::sync::watch::Sender<bool>,
    shutdown: tokio::sync::watch::Receiver<bool>,
    skill_executor_api: SkillExecutorApi,
    config: Box<dyn ObservableConfig + Send>,
    update_interval: Duration,
    skills: HashMap<String, Vec<Skill>>,
}

#[derive(Debug)]
struct Diff {
    added: Vec<Skill>,
    removed: Vec<Skill>,
}

impl ConfigurationObserverActor {
    fn new(
        ready: tokio::sync::watch::Sender<bool>,
        shutdown: tokio::sync::watch::Receiver<bool>,
        skill_executor_api: SkillExecutorApi,
        config: Box<dyn ObservableConfig + Send>,
        update_interval: Duration,
    ) -> Self {
        Self {
            ready,
            shutdown,
            skill_executor_api,
            config,
            update_interval,
            skills: HashMap::new(),
        }
    }

    fn compute_diff(existing: &[Skill], incoming: &[Skill]) -> Diff {
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

        Diff { added, removed }
    }

    async fn run(mut self) {
        let namespaces = self.config.namespaces();
        let mut started = tokio::time::Instant::now();
        self.load(&namespaces).await;
        let _ = self.ready.send(true);
        loop {
            select! {
                _ = self.shutdown.changed() => break,
                () = tokio::time::sleep_until(started + self.update_interval) => (),
            };
            started = tokio::time::Instant::now();
            self.load(&namespaces).await;
        }
    }

    async fn load(&mut self, namespaces: &Vec<String>) {
        for namespace in namespaces {
            let incoming = match self.config.skills(namespace).await {
                Ok(incoming) => incoming,
                Err(e) => {
                    error!(
                        "Failed to get the latest skills in namespace {namespace}, caused by: {e}"
                    );
                    continue;
                }
            };
            let existing = self
                .skills
                .insert(namespace.to_owned(), incoming)
                .unwrap_or_default();

            let incoming = self.skills.get(namespace).unwrap();
            let diff = Self::compute_diff(&existing, incoming);
            for skill in diff.added {
                self.skill_executor_api
                    .add_skill(SkillPath::new(namespace, &skill.name), skill.tag)
                    .await;
            }
            for skill in diff.removed {
                self.skill_executor_api
                    .remove_skill(SkillPath::new(namespace, &skill.name), skill.tag)
                    .await;
            }
        }
    }
}

#[cfg(test)]
pub mod tests {
    use std::collections::HashMap;
    use std::future::pending;

    use tokio::sync::mpsc;
    use tokio::time::timeout;

    use crate::skills::tests::SkillExecutorMessage;
    use crate::skills::{SkillExecutorApi, SkillPath};

    use super::*;

    pub struct StubConfig {
        namespaces: HashMap<String, Vec<Skill>>,
    }

    impl StubConfig {
        pub fn new(namespaces: HashMap<String, Vec<Skill>>) -> Self {
            Self { namespaces }
        }
    }

    #[async_trait]
    impl ObservableConfig for StubConfig {
        fn namespaces(&self) -> Vec<String> {
            self.namespaces.keys().cloned().collect()
        }

        async fn skills(&mut self, namespace: &str) -> anyhow::Result<Vec<Skill>> {
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

        async fn skills(&mut self, _namespace: &str) -> anyhow::Result<Vec<Skill>> {
            pending().await
        }
    }

    impl Skill {
        fn with_name(name: &str) -> Self {
            Self::new(name, None)
        }

        fn new(name: &str, tag: Option<&str>) -> Self {
            Self {
                name: name.to_owned(),
                tag: tag.map(|s| s.to_owned()),
            }
        }
    }

    #[test]
    fn diff_is_computed() {
        let incoming = vec![
            Skill::with_name("new_skill"),
            Skill::with_name("existing_skill"),
        ];
        let existing = vec![
            Skill::with_name("existing_skill"),
            Skill::with_name("old_skill"),
        ];

        let diff = ConfigurationObserverActor::compute_diff(&existing, &incoming);

        // when the observer checks for new skills
        assert_eq!(diff.added, vec![Skill::with_name("new_skill")]);
        assert_eq!(diff.removed, vec![Skill::with_name("old_skill")]);
    }

    #[test]
    fn diff_is_computed_over_versions() {
        // Given a skill has a new version
        let existing = Skill::new("existing_skill", Some("v1"));
        let incoming = Skill::new("existing_skill", Some("v2"));

        // When the observer checks for new skills
        let diff =
            ConfigurationObserverActor::compute_diff(&[existing.clone()], &[incoming.clone()]);

        // Then the new version is added and the old version is removed
        assert_eq!(diff.added, vec![incoming]);
        assert_eq!(diff.removed, vec![existing]);
    }

    #[tokio::test]
    async fn load_config_during_first_pass() {
        // Given a config that take forever to load
        let (sender, _) = mpsc::channel::<SkillExecutorMessage>(1);
        let skill_executor_api = SkillExecutorApi::new(sender);
        let config = Box::new(PendingConfig);
        let update_interval = Duration::from_millis(1);
        let mut observer =
            ConfigurationObserver::with_config(skill_executor_api, config, update_interval);

        // When waiting for the first pass
        let result = tokio::time::timeout(Duration::from_secs(1), observer.wait_for_ready()).await;

        // Then it will timeout
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn on_start_reports_all_skills_to_executor_agent() {
        // Given some configured skills
        let dummy_namespace = "dummy_namespace";
        let dummy_skill = "dummy_skill";
        let update_interval_ms = 1;
        let namespaces = HashMap::from([(
            dummy_namespace.to_owned(),
            vec![Skill::with_name(dummy_skill)],
        )]);
        let stub_config = Box::new(StubConfig::new(namespaces));

        // When we boot up the configuration observer
        let (sender, mut receiver) = mpsc::channel::<SkillExecutorMessage>(1);
        let skill_executor_api = SkillExecutorApi::new(sender);
        let update_interval = Duration::from_millis(update_interval_ms);
        let observer =
            ConfigurationObserver::with_config(skill_executor_api, stub_config, update_interval);

        // Then one new skill message is send for each skill configured
        let msg = receiver.recv().await;
        let msg = msg.unwrap();

        assert!(matches!(
            msg,
            SkillExecutorMessage::Add {
                skill: SkillPath {
                    namespace,
                    name,
                },
                tag: None
            }
            if namespace == dummy_namespace && name == dummy_skill
        ));

        observer.wait_for_shutdown().await;
    }

    #[tokio::test]
    async fn new_skill_only_reported_once() {
        // Given some configured skills
        let dummy_namespace = "dummy_namespace";
        let dummy_skill = "dummy_skill";
        let update_interval_ms = 1;
        let namespaces = HashMap::from([(
            dummy_namespace.to_owned(),
            vec![Skill::with_name(dummy_skill)],
        )]);
        let stub_config = Box::new(StubConfig::new(namespaces));

        // When we boot up the configuration observer
        let (sender, mut receiver) = mpsc::channel::<SkillExecutorMessage>(1);
        let skill_executor_api = SkillExecutorApi::new(sender);
        let update_interval = Duration::from_millis(update_interval_ms);
        let observer =
            ConfigurationObserver::with_config(skill_executor_api, stub_config, update_interval);

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
}
