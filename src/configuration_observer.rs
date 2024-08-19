use tokio::time::Duration;
use tokio::{select, sync::mpsc, task::JoinHandle};

/// Periodically observes changes in remote repositories containing
/// skill configurations and reports detected changes to the skill executor
pub struct ConfigurationObserver {
    shutdown: tokio::sync::watch::Sender<bool>,
    handle: JoinHandle<()>,
}

impl ConfigurationObserver {
    pub fn new() -> Self {
        let (sender, mut receiver) = tokio::sync::watch::channel(false);
        let handle = tokio::spawn(async move {
            loop {
                select! {
                    _ = receiver.changed() => break,
                    _ = tokio::time::sleep(Duration::from_secs(10)) => (),
                };
            }
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
