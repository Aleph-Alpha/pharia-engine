use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

#[cfg(test)]
use double_trait::double;

use crate::{
    mcp::{ConfiguredMcpServer, McpServerUrl},
    namespace_watcher::Namespace,
};

/// CSI facing interface, allows to invoke and list tools
#[cfg_attr(test, double(McpDouble))]
pub trait McpApi {
    fn mcp_upsert(&self, server: ConfiguredMcpServer) -> impl Future<Output = ()> + Send;
    fn mcp_remove(&self, server: ConfiguredMcpServer) -> impl Future<Output = ()> + Send;
    fn mcp_list(&self, namespace: Namespace) -> impl Future<Output = Vec<McpServerUrl>> + Send;
}

pub struct Mcp {
    handle: JoinHandle<()>,
    send: mpsc::Sender<McpMsg>,
}

impl Mcp {
    pub fn new() -> Self {
        let (send, receiver) = tokio::sync::mpsc::channel::<McpMsg>(1);
        let mut actor = McpActor::new(receiver);
        let handle = tokio::spawn(async move { actor.run().await });
        Self { handle, send }
    }

    pub fn api(&self) -> McpSender {
        McpSender(self.send.clone())
    }

    pub async fn wait_for_shutdown(self) {
        drop(self.send);
        self.handle.await.unwrap();
    }
}

/// Opaque wrapper around a sender to the MCP actor, so we do not need to expose our message
/// type.
#[derive(Clone)]
pub struct McpSender(mpsc::Sender<McpMsg>);

impl McpApi for McpSender {
    async fn mcp_upsert(&self, server: ConfiguredMcpServer) {
        let msg = McpMsg::Upsert { server };
        self.0.send(msg).await.unwrap();
    }

    async fn mcp_remove(&self, server: ConfiguredMcpServer) {
        let msg = McpMsg::Remove { server };
        self.0.send(msg).await.unwrap();
    }

    async fn mcp_list(&self, namespace: Namespace) -> Vec<McpServerUrl> {
        let (send, receive) = oneshot::channel();
        let msg = McpMsg::List { namespace, send };
        self.0.send(msg).await.unwrap();
        receive.await.unwrap()
    }
}

enum McpMsg {
    Upsert {
        server: ConfiguredMcpServer,
    },
    Remove {
        server: ConfiguredMcpServer,
    },
    List {
        namespace: Namespace,
        send: oneshot::Sender<Vec<McpServerUrl>>,
    },
}

struct McpActor {
    receiver: mpsc::Receiver<McpMsg>,
}

impl McpActor {
    fn new(receiver: mpsc::Receiver<McpMsg>) -> Self {
        Self { receiver }
    }

    async fn run(&mut self) {
        loop {
            let msg = self.receiver.recv().await;
            match msg {
                Some(msg) => self.act(msg),
                None => break,
            }
        }
    }

    fn act(&mut self, msg: McpMsg) {
        match msg {
            McpMsg::Upsert { server } => todo!(),
            McpMsg::Remove { server } => todo!(),
            McpMsg::List { namespace, send } => {
                send.send(vec![]).unwrap();
            }
        }
    }
}

#[cfg(test)]
pub mod tests {

    use super::*;

    #[tokio::test]
    async fn list_mcp_servers_none_configured() {
        // Given a MCP API that knows about no mcp servers
        let mcp = Mcp::new().api();

        // When listing mcp servers for a namespace
        let result = mcp.mcp_list(Namespace::new("test").unwrap()).await;

        // Then we get an empty list
        assert!(result.is_empty());
    }
}
