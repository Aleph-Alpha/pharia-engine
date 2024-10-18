#![expect(dead_code)]
use tokio::{sync::mpsc, task::JoinHandle};

pub struct Authorization {
    send: mpsc::Sender<AuthorizationMsg>,
    handle: JoinHandle<()>,
}

impl Authorization {
    pub fn new() -> Authorization {
        let (send, _recv) = mpsc::channel(3);
        let handle = tokio::spawn(async move {});
        Authorization { send, handle }
    }

    pub fn api(&self) -> impl AuthorizationApi + use<> {
        self.send.clone()
    }

    /// Authorization is going to shutdown, as soon as the last instance of [`AuthorizationApi`] is
    /// dropped.
    pub async fn wait_for_shutdown(self) {
        drop(self.send);
        self.handle.await.unwrap();
    }
}

pub trait AuthorizationApi {}

impl AuthorizationApi for mpsc::Sender<AuthorizationMsg> {}

enum AuthorizationMsg {}
