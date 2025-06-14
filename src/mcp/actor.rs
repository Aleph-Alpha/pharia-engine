use std::collections::{HashMap, HashSet};

use itertools::Itertools;
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

#[cfg(test)]
use double_trait::double;

use crate::{
    mcp::{ConfiguredMcpServer, McpClient, McpServerStore, McpServerUrl},
    namespace_watcher::Namespace,
    tool::ToolClient,
};

/// CSI facing interface, allows to invoke and list tools
#[cfg_attr(test, double(McpDouble))]
#[allow(dead_code)]
pub trait McpApi {
    fn upsert(&self, server: ConfiguredMcpServer) -> impl Future<Output = ()> + Send;
    fn remove(&self, server: ConfiguredMcpServer) -> impl Future<Output = ()> + Send;
    fn mcp_list(&self, namespace: Namespace) -> impl Future<Output = Vec<McpServerUrl>> + Send;
    fn list_tools(&self, namespace: Namespace) -> impl Future<Output = Vec<String>> + Send;
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

    #[allow(dead_code)]
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
    async fn upsert(&self, server: ConfiguredMcpServer) {
        let msg = McpMsg::Upsert { server };
        self.0.send(msg).await.unwrap();
    }

    async fn remove(&self, server: ConfiguredMcpServer) {
        let msg = McpMsg::Remove { server };
        self.0.send(msg).await.unwrap();
    }

    async fn mcp_list(&self, namespace: Namespace) -> Vec<McpServerUrl> {
        let (send, receive) = oneshot::channel();
        let msg = McpMsg::List { namespace, send };
        self.0.send(msg).await.unwrap();
        receive.await.unwrap()
    }

    async fn list_tools(&self, namespace: Namespace) -> Vec<String> {
        let (send, receive) = oneshot::channel();
        let msg = McpMsg::ListTools { namespace, send };
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
    ListTools {
        namespace: Namespace,
        send: oneshot::Sender<Vec<String>>,
    },
}

struct McpActor {
    store: McpServerStore,
    tools: HashMap<McpServerUrl, HashSet<String>>,
    client: McpClient,
    receiver: mpsc::Receiver<McpMsg>,
}

impl McpActor {
    fn new(receiver: mpsc::Receiver<McpMsg>) -> Self {
        Self {
            store: McpServerStore::new(),
            tools: HashMap::new(),
            client: McpClient::new(),
            receiver,
        }
    }

    async fn run(&mut self) {
        loop {
            let msg = self.receiver.recv().await;
            match msg {
                Some(msg) => self.act(msg).await,
                None => break,
            }
        }
    }

    async fn act(&mut self, msg: McpMsg) {
        match msg {
            McpMsg::Upsert { server } => {
                self.store.upsert(server.namespace, server.url.clone());
                if let Ok(tools) = self.client.list_tools(&server.url).await {
                    self.tools.insert(server.url, tools.into_iter().collect());
                } else {
                    self.tools.remove(&server.url);
                }
            }
            McpMsg::Remove { server } => {
                self.store.remove(server.namespace, server.url);
            }
            McpMsg::List { namespace, send } => {
                let result = self.store.list_in_namespace(&namespace).collect();
                drop(send.send(result));
            }
            McpMsg::ListTools { namespace, send } => {
                let urls = self.store.list_in_namespace(&namespace).collect::<Vec<_>>();
                let tools = urls
                    .iter()
                    .flat_map(|url| self.tools.get(url).cloned().unwrap_or_default())
                    .sorted()
                    .collect();
                drop(send.send(tools));
            }
        }
    }
}

#[cfg(test)]
pub mod tests {

    use std::collections::HashSet;

    use test_skills::given_sse_mcp_server;

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

    #[tokio::test]
    async fn upserted_server_is_listed() {
        // Given a MCP API that knows about no mcp servers
        let mcp = Mcp::new().api();

        // When upserting mcp server for the namespace
        mcp.upsert(ConfiguredMcpServer::new(
            "http://localhost:8000/mcp",
            Namespace::new("test").unwrap(),
        ))
        .await;

        // Then we get an empty list for a namespace that does not have a mcp server
        let result = mcp.mcp_list(Namespace::new("not-existing").unwrap()).await;
        assert_eq!(result, vec![]);

        // Then we get the mcp server
        let result = mcp.mcp_list(Namespace::new("test").unwrap()).await;
        assert_eq!(result, vec![McpServerUrl::new("http://localhost:8000/mcp")]);
    }

    #[tokio::test]
    async fn removed_server_is_not_listed() {
        // Given a MCP API that knows about one mcp server
        let mcp = Mcp::new().api();
        mcp.upsert(ConfiguredMcpServer::new(
            "http://localhost:8000/mcp",
            Namespace::new("test").unwrap(),
        ))
        .await;

        // When removing the mcp server
        mcp.remove(ConfiguredMcpServer::new(
            "http://localhost:8000/mcp",
            Namespace::new("test").unwrap(),
        ))
        .await;

        // Then we get an empty list
        let result = mcp.mcp_list(Namespace::new("test").unwrap()).await;
        assert_eq!(result, vec![]);
    }

    #[tokio::test]
    async fn adding_existing_tool_server_does_not_add_it_again() {
        // Given a tool client that knows about two mcp servers for a namespace
        let mcp = Mcp::new().api();
        let namespace = Namespace::new("test").unwrap();
        let server = ConfiguredMcpServer::new("http://localhost:8000/mcp", namespace.clone());
        mcp.upsert(server.clone()).await;

        // When we add the same tool server again for that namespace
        mcp.upsert(server).await;

        // Then the list of tools is still the same
        let mcp_servers = mcp.mcp_list(namespace).await;
        assert_eq!(mcp_servers, vec!["http://localhost:8000/mcp".into()]);
    }

    #[tokio::test]
    async fn same_tool_server_can_be_configured_for_different_namespaces() {
        // Given a tool client
        let mcp = Mcp::new().api();
        let first = Namespace::new("first").unwrap();

        // When adding the same tool server for two namespaces
        let server_url = "http://localhost:8000/mcp";
        let server = ConfiguredMcpServer::new(server_url, first.clone());
        mcp.upsert(server.clone()).await;

        let second = Namespace::new("second").unwrap();
        let server = ConfiguredMcpServer::new(server_url, second.clone());
        mcp.upsert(server).await;

        // Then the server is configured for both namespaces
        let mcp_servers = mcp.mcp_list(first).await;
        assert_eq!(mcp_servers, vec![server_url.into()]);

        let mcp_servers = mcp.mcp_list(second).await;
        assert_eq!(mcp_servers, vec![server_url.into()]);
    }

    #[tokio::test]
    async fn list_tool_servers() {
        // Given a store with two mcp servers inserted for the namespace "test"
        let mcp = Mcp::new().api();
        let namespace = Namespace::new("test").unwrap();
        mcp.upsert(ConfiguredMcpServer::new(
            "http://localhost:8000/mcp",
            namespace.clone(),
        ))
        .await;
        mcp.upsert(ConfiguredMcpServer::new(
            "http://localhost:8001/mcp",
            namespace.clone(),
        ))
        .await;

        // When listing tool servers for that namespace
        let result: HashSet<McpServerUrl> = mcp.mcp_list(namespace).await.into_iter().collect();

        // Then we get the two mcp servers
        let expected = HashSet::from([
            "http://localhost:8000/mcp".into(),
            "http://localhost:8001/mcp".into(),
        ]);
        assert_eq!(result, expected);
    }

    #[tokio::test]
    async fn cache_tools_on_upsert() {
        // Given a MCP Server
        let mcp_server = given_sse_mcp_server().await;

        // Given a MCP actor
        let mcp = Mcp::new().api();

        // When adding the MCP server for a namespace
        let namespace = Namespace::new("test").unwrap();
        let server = ConfiguredMcpServer::new(mcp_server.address(), namespace.clone());
        mcp.upsert(server).await;

        // Then the tools from the MCP server are listed
        let tools = mcp.list_tools(namespace).await;
        assert_eq!(tools, vec!["add", "saboteur"]);
    }
}
