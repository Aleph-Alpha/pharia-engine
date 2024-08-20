use std::collections::HashMap;

use tokio::{select, task::JoinHandle, time::Duration};

use crate::skills::SkillExecutorApi;

use super::{NamespaceConfig, OperatorConfig};

pub trait Config {
    fn namespaces(&self) -> &[String];
}

struct ConfigImpl {
    namespaces: HashMap<String, Box<dyn NamespaceConfig + Send>>,
}

impl Config for ConfigImpl {
    fn namespaces(&self) -> &[String] {
        &[]
    }
}

impl ConfigImpl {
    pub fn new(deserialized: OperatorConfig) -> Self {
        Self {
            namespaces: HashMap::new(),
        }
    }
}

/// Periodically observes changes in remote repositories containing
/// skill configurations and reports detected changes to the skill executor
pub struct ConfigurationObserver {
    shutdown: tokio::sync::watch::Sender<bool>,
    handle: JoinHandle<()>,
}

impl ConfigurationObserver {
    pub fn new(skill_executor_api: SkillExecutorApi) -> Self {
        let config_str = include_str!("../../config.toml");
        let config = OperatorConfig::from_str(config_str);
        let config = Box::new(ConfigImpl::new(config));
        Self::with_config(skill_executor_api, config)
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
            select! {
                _ = self.shutdown.changed() => break,
                _ = tokio::time::sleep(Duration::from_secs(10)) => (),
            };
            for namespace in self.config.namespaces() {
                // TODO! next step
                // read the remote repository,
                // send all observed skills with namespace to the
                // executor API
                // Later: compute difference and only send observed changes (new, drop)
            }
        }
    }
}

#[cfg(test)]
mod tests {

    use super::Config;

    struct StubConfig {}

    impl StubConfig {
        fn new() -> StubConfig {
            StubConfig {}
        }
    }

    impl Config for StubConfig {
        fn namespaces(&self) -> &[String] {
            &[]
        }
    }

    #[test]
    fn on_start_reports_all_skills_to_executer_agent() {
        // Given some configured skills
        let stub_config = StubConfig::new();
        // When we boot up the configuration observer

        // Then one new skill message is send for each skill configured
    }
}
