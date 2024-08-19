use tokio::{select, task::JoinHandle, time::Duration};

use crate::{configuration_observer::Namespace, skills::SkillExecutorApi};

use super::Config;

/// Periodically observes changes in remote repositories containing
/// skill configurations and reports detected changes to the skill executor
pub struct ConfigurationObserver {
    shutdown: tokio::sync::watch::Sender<bool>,
    handle: JoinHandle<()>,
}

impl ConfigurationObserver {
    pub fn new(skill_executor_api: SkillExecutorApi) -> Self {
        let (sender, receiver) = tokio::sync::watch::channel(false);
        let config_str = include_str!("../../config.toml");
        let config = Config::from_str(config_str);
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
    config: Config,
}

impl ConfigurationObserverActor {
    fn new(
        shutdown: tokio::sync::watch::Receiver<bool>,
        skill_executor_api: SkillExecutorApi,
        config: Config,
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
            for (
                namespace,
                &Namespace {
                    ref repository,
                    ref registry,
                    ref config_url,
                },
            ) in &self.config.namespaces
            {
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

    #[test]
    fn got_namespace_and_skill() {}
}
