use tokio::{sync::mpsc, task::JoinHandle,};

#[derive(Clone)]
pub struct TokenizersApi {
    sender: mpsc::Sender<TokenizersMsg>
}

impl TokenizersApi {
    pub fn new(sender: mpsc::Sender<TokenizersMsg>) -> Self {
        TokenizersApi { sender }
    }
}

/// Actor providing tokenizers. These tokenizers are currently used to power chunking logic for CSI
pub struct Tokenizers {
    sender: mpsc::Sender<TokenizersMsg>,
    handle: JoinHandle<()>
}

impl Tokenizers {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel(1);
        let handle = tokio::spawn(async move {
            let mut actor = TokenizersActor::new(receiver);
            actor.run().await
        });
        Tokenizers {
            sender,
            handle
        }
    }

    pub fn api(&self) -> TokenizersApi {
        TokenizersApi::new(self.sender.clone())
    }

    pub async fn wait_for_shutdown(self) {
        drop(self.sender);
        self.handle.await.unwrap();
    }
}

pub enum TokenizersMsg {}

struct TokenizersActor {
    receiver: mpsc::Receiver<TokenizersMsg>
}

impl TokenizersActor {
    pub fn new(receiver: mpsc::Receiver<TokenizersMsg>) -> Self {
        TokenizersActor { receiver }
    }

    pub async fn run(&mut self) {
        while let Some(msg) = self.receiver.recv().await {

        }
    }
}