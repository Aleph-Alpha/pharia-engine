//! MCP server management and tool discovery.
//!
//! This module provides the [`McpApi`] trait for managing MCP servers (upsert, remove, list)
//! and consumes a [`McpSubscriber`] to notify other components when the available tool set
//! changes. The actor periodically polls configured servers to discover new tools.

use std::{pin::Pin, sync::Arc, time::Duration};

use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use crate::{
    mcp::{
        ConfiguredMcpServer, McpServerUrl, McpSubscriber,
        client::{McpClient, McpClientImpl},
        mcp_tool::McpTool,
        store::McpServerStore,
    },
    namespace_watcher::Namespace,
    tool::Tool,
};
#[cfg(test)]
use double_trait::double;
use futures::{Future, StreamExt, stream::FuturesUnordered};

/// Interface offered by the MCP actor.
///
/// Allows the MCP actor to be notified about changes in the list of configured MCP servers.
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
    /// Construct directly from a subscriber, using the defaults sensible for production use.
    pub fn from_subscriber(subscriber: impl McpSubscriber + Send + 'static) -> Self {
        Self::new(McpClientImpl::new(), subscriber, Duration::from_secs(60))
    }

    /// More powerful constructor that comes in handy for testing.
    pub fn new(
        client: impl McpClient,
        subscriber: impl McpSubscriber + Send + 'static,
        check_interval: Duration,
    ) -> Self {
        let (send, receiver) = mpsc::channel::<McpMsg>(1);
        let mut actor = McpActor::new(receiver, client, subscriber, check_interval);
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

/// A request to fetch tools for a given MCP server.
type ToolServerRequest =
    Pin<Box<dyn Future<Output = Result<(McpServerUrl, Vec<String>), anyhow::Error>> + Send>>;

struct McpActor<C, S> {
    store: McpServerStore,
    receiver: mpsc::Receiver<McpMsg>,
    client: Arc<C>,
    subscriber: S,
    check_interval: Duration,
    running_requests: FuturesUnordered<ToolServerRequest>,
}

impl<C, S> McpActor<C, S>
where
    C: McpClient,
    S: McpSubscriber,
{
    fn new(
        receiver: mpsc::Receiver<McpMsg>,
        client: C,
        subscriber: S,
        check_interval: Duration,
    ) -> Self {
        Self {
            store: McpServerStore::new(),
            receiver,
            client: Arc::new(client),
            subscriber,
            check_interval,
            running_requests: FuturesUnordered::new(),
        }
    }

    async fn run(&mut self) {
        // This select statement ensure that we can always accept and handle incoming messages,
        // even if requests to some mcp servers are pending. Also, if a mcp server is slow to
        // respond, it does not block updates from other mcp servers to be propagated.
        loop {
            tokio::select! {
                // Listen for incoming messages and act on them directly.
                msg = self.receiver.recv() => {
                    match msg {
                        Some(msg) => self.act(msg).await,
                        None => break,
                    }
                }
                // Wait for the next mcp server to be up for refresh, then add it to the backlog.
                next_up_for_refresh = self.store.wait_for_next_refresh(self.check_interval) => {
                    if let Some(server) = next_up_for_refresh {
                        self.add_to_backlog(server);
                    }
                }
                // Continuously poll the backlog. Once a task in the backlog is completed, set the
                // tools in the store. If the list of tools has changed, report the updated tools
                // to the subscriber.
                result = self.running_requests.select_next_some(), if !self.running_requests.is_empty() => {
                    if let Ok((server, tools)) = result {
                        let updated = self.store.update_tools(server, tools);
                        if updated {
                            self.report_updated_tools().await;
                        }
                    }
                    // In the error case, we do not need to do anything. The tool list is already initialized
                    // with an empty list, so no update is required there.
                }

            }
        }
    }

    /// Create a new background task to fetch the tools for a given server.
    fn add_to_backlog(&mut self, server: McpServerUrl) {
        let client = self.client.clone();
        let request = Box::pin(async move {
            McpServerStore::fetch_tools_for(&server, client.as_ref())
                .await
                .map(|tools| (server, tools))
        });
        self.running_requests.push(request);
    }

    async fn act(&mut self, msg: McpMsg) {
        match msg {
            McpMsg::Upsert { server } => {
                let url = server.url.clone();
                self.store.upsert(server.namespace, server.url);
                self.add_to_backlog(url);
            }
            McpMsg::Remove { server } => {
                self.store.remove(server.namespace, server.url);
                self.report_updated_tools().await;
            }
            McpMsg::List { namespace, send } => {
                let result = self.store.list_in_namespace(&namespace).collect();
                drop(send.send(result));
            }
        }
    }

    async fn report_updated_tools(&mut self) {
        let tools = self
            .store
            .all_tools_by_name()
            .map(|(qtn, desc)| {
                let tool = McpTool::new(desc, self.client.clone());
                let tool: Arc<dyn Tool + Send + Sync> = Arc::new(tool);
                (qtn, tool)
            })
            .collect();
        self.subscriber.report_updated_tools(tools).await;
    }
}

#[cfg(test)]
pub mod tests {

    use futures::future::pending;
    use std::collections::{HashMap, HashSet};
    use std::sync::Mutex;
    use tokio::time::Instant as TokioInstant;

    use crate::{
        mcp::{
            McpClientDouble,
            subscribers::{McpSubscriberDouble, ToolMap},
        },
        tool::QualifiedToolName,
    };

    struct DummySubscriber;
    impl McpSubscriberDouble for DummySubscriber {
        async fn report_updated_tools(
            &mut self,
            _tools: HashMap<QualifiedToolName, Arc<dyn Tool + Send + Sync>>,
        ) {
            // Do nothing
        }
    }

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn tool_update_happens_in_background() {
        use tokio::sync::Mutex;

        // Given a client that optionally timeout
        #[derive(Clone)]
        struct SaboteurClient {
            timeout: Arc<Mutex<bool>>,
        }

        impl SaboteurClient {
            fn new() -> Self {
                Self {
                    timeout: Arc::new(Mutex::new(false)),
                }
            }

            pub async fn set_blocking(&self) {
                let mut timeout = self.timeout.lock().await;
                *timeout = true;
            }
        }

        impl McpClientDouble for SaboteurClient {
            async fn list_tools(&self, _url: &McpServerUrl) -> Result<Vec<String>, anyhow::Error> {
                let timeout = self.timeout.lock().await;
                if *timeout {
                    pending().await
                } else {
                    Ok(vec!["list_fish".to_owned(), "catch_fish".to_owned()])
                }
            }
        }

        // And given the mcp actor is configured with this mcp server
        let namespace = Namespace::new("pike").unwrap();
        let server = ConfiguredMcpServer::new("http://localhost:8000/mcp", namespace.clone());
        let client = SaboteurClient::new();
        let mcp = Mcp::new(client.clone(), DummySubscriber, Duration::from_millis(10)).api();
        mcp.upsert(server).await;

        // When the client blocks and the background update is running
        client.set_blocking().await;
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Then the list of tools can still be fetched as the update happens in the background
        tokio::time::timeout(Duration::from_secs(1), mcp.mcp_list(namespace))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn upserting_pending_tool_server_does_not_block() {
        // Given a client that hangs forever
        struct McpClientStub;
        impl McpClientDouble for McpClientStub {
            async fn list_tools(&self, _: &McpServerUrl) -> Result<Vec<String>, anyhow::Error> {
                pending().await
            }
        }

        // When upserting a tool server
        let mcp = Mcp::new(McpClientStub, DummySubscriber, Duration::from_secs(1000)).api();
        mcp.upsert(ConfiguredMcpServer::new(
            "http://localhost:8000/mcp",
            Namespace::new("test").unwrap(),
        ))
        .await;

        // Then the mcp actor still responds to other requests
        tokio::time::timeout(
            Duration::from_secs(1),
            mcp.mcp_list(Namespace::new("test").unwrap()),
        )
        .await
        .unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn tools_are_updated_regularly() {
        // Given a client which reports an additional tool after 30 seconds
        struct McpClientStub {
            start: TokioInstant,
        }
        impl McpClientDouble for McpClientStub {
            async fn list_tools(&self, _: &McpServerUrl) -> Result<Vec<String>, anyhow::Error> {
                let elapsed = self.start.elapsed();
                if elapsed >= Duration::from_secs(30) {
                    Ok(vec!["tool_one".to_string(), "tool_two".to_string()])
                } else {
                    Ok(vec!["tool_one".to_string()])
                }
            }
        }

        let subscriber = RecordingSubscriber::new();
        let mcp = Mcp::new(
            McpClientStub {
                start: TokioInstant::now(),
            },
            subscriber.clone(),
            Duration::from_secs(60),
        )
        .api();
        // and a known mcp server
        let namespace = Namespace::new("test").unwrap();
        let server = ConfiguredMcpServer::new("http://localhost:8000/mcp", namespace.clone());
        mcp.upsert(server).await;

        // When time advances for at least 60 seconds (the check interval)
        tokio::time::advance(Duration::from_secs(80)).await;

        // Then the subscriber has been called with the updated tools
        let last_list = subscriber.calls().last().cloned().unwrap();
        assert_eq!(last_list.len(), 2);
    }

    #[tokio::test]
    async fn list_mcp_servers_none_configured() {
        // Given a MCP API that knows about no mcp servers
        let mcp = Mcp::from_subscriber(DummySubscriber).api();

        // When listing mcp servers for a namespace
        let result = mcp.mcp_list(Namespace::new("test").unwrap()).await;

        // Then we get an empty list
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn upserted_server_is_listed() {
        // Given a MCP API that knows about no mcp servers
        let mcp = Mcp::from_subscriber(DummySubscriber).api();

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
            async fn report_updated_tools(
                &mut self,
                tools: HashMap<QualifiedToolName, Arc<dyn Tool + Send + Sync>>,
            ) {
                // Then the tool reported by the server is reported to the subscriber
                let expected_tool_name = QualifiedToolName {
                    namespace: Namespace::new("test-namespace").unwrap(),
                    name: "new-tool".to_owned(),
                };
                assert_eq!(tools.len(), 1);
                assert!(tools.contains_key(&expected_tool_name));
            }
        }
        struct StubClient;
        impl McpClientDouble for StubClient {
            async fn list_tools(&self, _: &McpServerUrl) -> Result<Vec<String>, anyhow::Error> {
                // Simulate a tool server that has one tool
                Ok(vec!["new-tool".into()])
            }
        }
        let mcp = Mcp::new(StubClient, SubscriberMock, Duration::from_secs(60)).api();

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
        let mcp = Mcp::from_subscriber(DummySubscriber).api();
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
        let mcp = Mcp::from_subscriber(DummySubscriber).api();
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
        let mcp = Mcp::from_subscriber(DummySubscriber).api();
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
        let mcp = Mcp::from_subscriber(DummySubscriber).api();
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
    async fn one_bad_mcp_server_still_allows_to_list_other_tools() {
        // Given an mcp server that returns an error when listing it's tools
        struct ClientOneGoodOtherBad;
        impl McpClientDouble for ClientOneGoodOtherBad {
            async fn list_tools(&self, url: &McpServerUrl) -> Result<Vec<String>, anyhow::Error> {
                if url.0 == "http://localhost:8000/mcp" {
                    Ok(vec!["one_tool".to_owned()])
                } else {
                    Err(anyhow::anyhow!("Request to mcp server timed out."))
                }
            }
        }
        let namespace = Namespace::new("test").unwrap();
        let servers = [
            ConfiguredMcpServer::new("http://localhost:8000/mcp", namespace.clone()),
            ConfiguredMcpServer::new("http://localhost:8001/mcp", namespace.clone()),
        ];
        let subscriber = RecordingSubscriber::new();
        let mcp = Mcp::new(
            ClientOneGoodOtherBad,
            subscriber.clone(),
            Duration::from_secs(60),
        )
        .api();

        // When upserting both servers
        mcp.upsert(servers[0].clone()).await;
        mcp.upsert(servers[1].clone()).await;

        // And waiting shortly for the actor to poll the task
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Then the search tool is listed
        let tool_list = subscriber.calls().last().unwrap().clone();
        assert!(tool_list.contains_key(&QualifiedToolName {
            namespace: namespace.clone(),
            name: "one_tool".to_owned(),
        }));
    }

    #[derive(Clone)]
    struct RecordingSubscriber {
        calls: Arc<Mutex<Vec<ToolMap>>>,
    }

    impl RecordingSubscriber {
        pub fn new() -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        pub fn calls(&self) -> Vec<ToolMap> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl McpSubscriberDouble for RecordingSubscriber {
        async fn report_updated_tools(
            &mut self,
            tools: HashMap<QualifiedToolName, Arc<dyn Tool + Send + Sync>>,
        ) {
            self.calls.lock().unwrap().push(tools);
        }
    }
}
