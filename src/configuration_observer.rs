use tokio::{select, task::JoinHandle, time::Duration};

use crate::skills::SkillExecutorApi;

/// Periodically observes changes in remote repositories containing
/// skill configurations and reports detected changes to the skill executor
pub struct ConfigurationObserver {
    shutdown: tokio::sync::watch::Sender<bool>,
    handle: JoinHandle<()>,
}

impl ConfigurationObserver {
    pub fn new(skill_executor_api: SkillExecutorApi) -> Self {
        let (sender, receiver) = tokio::sync::watch::channel(false);
        let handle = ConfigurationObserverActor::new(receiver, skill_executor_api).run();
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
}

impl ConfigurationObserverActor {
    fn new(
        shutdown: tokio::sync::watch::Receiver<bool>,
        skill_executor_api: SkillExecutorApi,
    ) -> Self {
        Self {
            shutdown,
            skill_executor_api,
        }
    }
    fn run(mut self) -> JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                select! {
                    _ = self.shutdown.changed() => break,
                    _ = tokio::time::sleep(Duration::from_secs(10)) => (),
                };
            }
        })
    }
}
