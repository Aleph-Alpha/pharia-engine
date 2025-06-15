use std::sync::Arc;

use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

#[cfg(test)]
use double_trait::double;

use crate::{
    mcp::{
        ConfiguredMcpServer, McpClient, McpClientImpl, McpServerStore, McpServerUrl, McpSubscriber,
        McpTool,
    },
    namespace_watcher::Namespace,
    tool::Tool,
};

/// CSI facing interface, allows to invoke and list tools
#[cfg_attr(test, double(McpDouble))]
pub trait McpApi {
    fn upsert(&self, server: ConfiguredMcpServer) -> impl Future<Output = ()> + Send;
    fn remove(&self, server: ConfiguredMcpServer) -> impl Future<Output = ()> + Send;
    fn mcp_list(&self, namespace: Namespace) -> impl Future<Output = Vec<McpServerUrl>> + Send;
}

pub struct Mcp {
    handle: JoinHandle<()>,
    send: mpsc::Sender<McpMsg>,
}

impl Mcp {
    pub fn new(subscriber: impl McpSubscriber + Send + 'static) -> Self {
        Self::with_client(McpClientImpl::new(), subscriber)
    }

    pub fn with_client(
        client: impl McpClient,
        subscriber: impl McpSubscriber + Send + 'static,
    ) -> Self {
        let (send, receiver) = mpsc::channel::<McpMsg>(1);
        let mut actor = McpActor::new(receiver, client, subscriber);
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

struct McpActor<C, S> {
    store: McpServerStore,
    receiver: mpsc::Receiver<McpMsg>,
    client: Arc<C>,
    subscriber: S,
}

impl<C, S> McpActor<C, S>
where
    C: McpClient,
    S: McpSubscriber,
{
    fn new(receiver: mpsc::Receiver<McpMsg>, client: C, subscriber: S) -> Self {
        Self {
            store: McpServerStore::new(),
            receiver,
            client: Arc::new(client),
            subscriber,
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
                self.store
                    .upsert(server.namespace, server.url, self.client.as_ref())
                    .await;
                self.report_updated_tools();
            }
            McpMsg::Remove { server } => {
                self.store.remove(server.namespace, server.url);
                self.report_updated_tools();
            }
            McpMsg::List { namespace, send } => {
                let result = self.store.list_in_namespace(&namespace).collect();
                drop(send.send(result));
            }
        }
    }

    fn report_updated_tools(&self) {
        let tools = self
            .store
            .all_tools_by_name()
            .map(|(qtn, desc)| {
                let tool = McpTool::new(desc, self.client.clone());
                let tool: Arc<dyn Tool + Send + Sync> = Arc::new(tool);
                (qtn, tool)
            })
            .collect();
        self.subscriber.report_updated_tools(tools);
    }
}

#[cfg(test)]
pub mod tests {

    use std::collections::{HashMap, HashSet};

    use double_trait::Dummy;

    use crate::{mcp::{subscribers::McpSubscriberDouble, McpClientDouble}, tool::QualifiedToolName};

    use super::*;

    #[tokio::test]
    async fn list_mcp_servers_none_configured() {
        // Given a MCP API that knows about no mcp servers
        let mcp = Mcp::new(Dummy).api();

        // When listing mcp servers for a namespace
        let result = mcp.mcp_list(Namespace::new("test").unwrap()).await;

        // Then we get an empty list
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn upserted_server_is_listed() {
        // Given a MCP API that knows about no mcp servers
        let mcp = Mcp::new(Dummy).api();

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
    async fn report_tools_after_upserting_server() {
        // Given a MCP API that knows about no mcp servers
        struct SubscriberMock;
        impl McpSubscriberDouble for SubscriberMock {
            fn report_updated_tools(
                &self,
                tools: HashMap<QualifiedToolName, Arc<dyn Tool + Send + Sync>>,
            ) {
                // Then the tool reported by the server is reported to the subscriber
                let expected_tool_name = QualifiedToolName {
                    namespace: Namespace::new("test-namespace").unwrap(),
                    name: "new-tool".to_owned()
                };
                assert_eq!(tools.len(), 1);
                assert!(tools.contains_key(&expected_tool_name));
            }
        }
        struct StubClient;
        impl McpClientDouble for StubClient {
            async fn list_tools(&self,_: &McpServerUrl,) -> Result<Vec<String> ,anyhow::Error> {
                // Simulate a tool server that has one tool
                Ok(vec!["new-tool".into()])
            }
        }
        let mcp = Mcp::with_client(StubClient, SubscriberMock).api();

        // When upserting mcp server for the namespace
        mcp.upsert(ConfiguredMcpServer::new(
            "http://localhost:8000/mcp",
            Namespace::new("test-namespace").unwrap(),
        ))
        .await;
    }

    #[tokio::test]
    async fn removed_server_is_not_listed() {
        // Given a MCP API that knows about one mcp server
        let mcp = Mcp::new(Dummy).api();
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
        let mcp = Mcp::new(Dummy).api();
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
        let mcp = Mcp::new(Dummy).api();
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
        let mcp = Mcp::new(Dummy).api();
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
}
