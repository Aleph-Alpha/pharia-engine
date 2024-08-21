use std::collections::{HashMap, HashSet};

use tokio::{select, task::JoinHandle, time::Duration};

use crate::skills::{SkillExecutorApi, SkillPath};

use super::{namespace_from_url, Namespace, NamespaceConfig, OperatorConfig};

pub trait Config {
    fn namespaces(&self) -> Vec<&str>;
    fn skills(&self, namespace: &str) -> Vec<String>;
}

struct ConfigImpl {
    namespaces: HashMap<String, Box<dyn Namespace + Send>>,
}

impl ConfigImpl {
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

impl Config for ConfigImpl {
    fn namespaces(&self) -> Vec<&str> {
        self.namespaces.keys().map(String::as_str).collect()
    }

    fn skills(&self, namespace: &str) -> Vec<String> {
        let skills = self
            .namespaces
            .get(namespace)
            .expect("namespace must exist.")
            .skills();
        skills.iter().map(|s| s.name.clone()).collect()
    }
}

/// Periodically observes changes in remote repositories containing
/// skill configurations and reports detected changes to the skill executor
pub struct ConfigurationObserver {
    shutdown: tokio::sync::watch::Sender<bool>,
    handle: JoinHandle<()>,
}

impl ConfigurationObserver {
    pub fn new(skill_executor_api: SkillExecutorApi) -> anyhow::Result<Self> {
        let config = OperatorConfig::from_env_or_default()?;
        let config = Box::new(ConfigImpl::new(config)?);
        Ok(Self::with_config(
            skill_executor_api,
            config,
            Duration::from_secs(60),
        ))
    }

    pub fn with_config(
        skill_executor_api: SkillExecutorApi,
        config: Box<dyn Config + Send>,
        update_interval: Duration,
    ) -> Self {
        let (sender, receiver) = tokio::sync::watch::channel(false);
        let handle = tokio::spawn(async move {
            ConfigurationObserverActor::new(receiver, skill_executor_api, config, update_interval)
                .run()
                .await;
        });
        Self {
            shutdown: sender,
            handle,
        }
    }

    pub async fn wait_for_shutdown(self) {
        self.shutdown.send(true).unwrap();
        self.handle.await.unwrap();
    }
}

struct ConfigurationObserverActor {
    shutdown: tokio::sync::watch::Receiver<bool>,
    skill_executor_api: SkillExecutorApi,
    config: Box<dyn Config + Send>,
    update_interval: Duration,
    skills: HashMap<String, Vec<String>>,
}

struct Diff {
    added: Vec<String>,
    removed: Vec<String>,
}

impl ConfigurationObserverActor {
    fn new(
        shutdown: tokio::sync::watch::Receiver<bool>,
        skill_executor_api: SkillExecutorApi,
        config: Box<dyn Config + Send>,
        update_interval: Duration,
    ) -> Self {
        Self {
            shutdown,
            skill_executor_api,
            config,
            update_interval,
            skills: HashMap::new(),
        }
    }

    fn compute_diff(existing: &[String], incoming: &[String]) -> Diff {
        let existing = existing.iter().collect::<HashSet<_>>();
        let incoming = incoming.iter().collect::<HashSet<_>>();

        let added = incoming
            .difference(&existing)
            .map(ToString::to_string)
            .collect();
        let removed = existing
            .difference(&incoming)
            .map(ToString::to_string)
            .collect();

        Diff { added, removed }
    }

    async fn run(mut self) {
        loop {
            for namespace in self.config.namespaces() {
                let incoming = self.config.skills(namespace);
                let existing = self
                    .skills
                    .insert(namespace.to_owned(), incoming)
                    .unwrap_or_default();

                let incoming = self.skills.get(namespace).unwrap();
                let diff = Self::compute_diff(&existing, incoming);
                for skill in diff.added {
                    self.skill_executor_api
                        .add_skill(SkillPath::new(namespace, &skill))
                        .await;
                }
                for skill in diff.removed {
                    self.skill_executor_api
                        .remove_skill(SkillPath::new(namespace, &skill))
                        .await;
                }
            }
            select! {
                _ = self.shutdown.changed() => break,
                () = tokio::time::sleep(self.update_interval) => (),
            };
        }
    }
}

#[cfg(test)]
pub mod tests {

    use std::collections::HashMap;

    use tokio::sync::mpsc;
    use tokio::time::timeout;

    use crate::skills::tests::SkillExecutorMessage;
    use crate::skills::{SkillExecutorApi, SkillPath};

    use super::*;

    pub struct StubConfig {
        namespaces: HashMap<String, Vec<String>>,
    }

    impl StubConfig {
        pub fn new(namespaces: HashMap<String, Vec<String>>) -> Self {
            Self { namespaces }
        }
    }

    impl Config for StubConfig {
        fn namespaces(&self) -> Vec<&str> {
            self.namespaces.keys().map(String::as_str).collect()
        }

        fn skills(&self, namespace: &str) -> Vec<String> {
            self.namespaces
                .get(namespace)
                .expect("namespace must exist.")
                .clone()
        }
    }

    #[test]
    fn diff_is_computed() {
        let incoming = vec!["new_skill".to_owned(), "existing_skill".to_owned()];
        let existing = vec!["existing_skill".to_owned(), "old_skill".to_owned()];

        let diff = ConfigurationObserverActor::compute_diff(&existing, &incoming);

        // when the observer checks for new skills
        assert_eq!(diff.added, vec!["new_skill"]);
        assert_eq!(diff.removed, vec!["old_skill"]);
    }

    #[tokio::test]
    async fn on_start_reports_all_skills_to_executor_agent() {
        // Given some configured skills
        let dummy_namespace = "dummy_namespace";
        let dummy_skill = "dummy_skill";
        let update_interval_ms = 1;
        let namespaces =
            HashMap::from([(dummy_namespace.to_owned(), vec![dummy_skill.to_owned()])]);
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
                }
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
        let namespaces =
            HashMap::from([(dummy_namespace.to_owned(), vec![dummy_skill.to_owned()])]);
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
