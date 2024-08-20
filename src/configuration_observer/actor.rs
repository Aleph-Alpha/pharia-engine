use std::collections::HashMap;

use tokio::{select, task::JoinHandle, time::Duration};

use crate::skills::{SkillExecutorApi, SkillPath};

use super::{namespace_from_url, Namespace, OperatorConfig};

pub trait Config {
    fn namespaces(&self) -> Vec<&str>;
    fn skills(&self, namespace: &str) -> Vec<&str>;
}

struct ConfigImpl {
    namespaces: HashMap<String, Box<dyn Namespace + Send>>,
}

impl ConfigImpl {
    pub fn new(deserialized: OperatorConfig) -> anyhow::Result<Self> {
        let namespaces = deserialized
            .namespaces
            .into_iter()
            .map(|(namespace, config)| Ok((namespace, namespace_from_url(&config.config_url)?)))
            .collect::<anyhow::Result<HashMap<_, _>>>()?;
        Ok(Self { namespaces })
    }
}

impl Config for ConfigImpl {
    fn namespaces(&self) -> Vec<&str> {
        self.namespaces.keys().map(String::as_str).collect()
    }

    fn skills(&self, namespace: &str) -> Vec<&str> {
        let skills = self
            .namespaces
            .get(namespace)
            .expect("namespace must exist.")
            .skills();
        skills.into_iter().map(|s| s.name.as_str()).collect()
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
        let config_str = include_str!("../../config.toml");
        let config = OperatorConfig::from_str(config_str)?;
        let config = Box::new(ConfigImpl::new(config)?);
        Ok(Self::with_config(skill_executor_api, config))
    }

    pub fn with_config(
        skill_executor_api: SkillExecutorApi,
        config: Box<dyn Config + Send>,
    ) -> Self {
        let (sender, receiver) = tokio::sync::watch::channel(false);
        let handle = tokio::spawn(async move {
            ConfigurationObserverActor::new(receiver, skill_executor_api, config)
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
}

impl ConfigurationObserverActor {
    fn new(
        shutdown: tokio::sync::watch::Receiver<bool>,
        skill_executor_api: SkillExecutorApi,
        config: Box<dyn Config + Send>,
    ) -> Self {
        Self {
            shutdown,
            skill_executor_api,
            config,
        }
    }

    async fn run(mut self) {
        loop {
            for namespace in self.config.namespaces() {
                let skills = self.config.skills(namespace);
                for skill in skills {
                    self.skill_executor_api
                        .add_skill(SkillPath::new(namespace, skill))
                        .await;
                }
                // Later: compute difference and only send observed changes (new, drop)
            }
            select! {
                _ = self.shutdown.changed() => break,
                () = tokio::time::sleep(Duration::from_secs(10)) => (),
            };
        }
    }
}

#[cfg(test)]
mod tests {

    use std::collections::HashMap;

    use tokio::sync::mpsc;

    use crate::skills::tests::SkillExecutorMessage;
    use crate::skills::{SkillExecutorApi, SkillPath};

    use super::*;

    struct StubConfig {
        namespaces: HashMap<String, Vec<String>>,
    }

    impl StubConfig {
        fn new(namespaces: HashMap<String, Vec<String>>) -> Self {
            Self { namespaces }
        }
    }

    impl Config for StubConfig {
        fn namespaces(&self) -> Vec<&str> {
            self.namespaces.keys().map(String::as_str).collect()
        }

        fn skills(&self, namespace: &str) -> Vec<&str> {
            self.namespaces
                .get(namespace)
                .expect("namespace must exist.")
                .iter()
                .map(String::as_str)
                .collect()
        }
    }

    #[tokio::test]
    async fn on_start_reports_all_skills_to_executor_agent() {
        // Given some configured skills
        let dummy_namespace = "dummy_namespace";
        let dummy_skill = "dummy_skill";
        let namespaces =
            HashMap::from([(dummy_namespace.to_owned(), vec![dummy_skill.to_owned()])]);
        let stub_config = Box::new(StubConfig::new(namespaces));

        // When we boot up the configuration observer
        let (sender, mut receiver) = mpsc::channel::<SkillExecutorMessage>(1);
        let skill_executor_api = SkillExecutorApi::new(sender);
        let observer = ConfigurationObserver::with_config(skill_executor_api, stub_config);

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
}
