use futures::StreamExt;
use futures::future::join_all;
use futures::stream::FuturesUnordered;
use std::collections::HashMap;
use std::pin::Pin;
use tokio::select;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::logging::TracingContext;
use crate::namespace_watcher::Namespace;
use crate::tool::Argument;
use crate::tool::Modality;
use crate::tool::ToolError;
use crate::tool::ToolOutput;
use crate::tool::toolbox::McpServerUrl;
use crate::tool::toolbox::{ConfiguredNativeTool, Toolbox};

#[cfg(test)]
use double_trait::double;

use super::client::McpClient;

/// An MCP server that is configured with a namespace.
///
/// Per namespace configuration allows different Skills to have access to different tools.
#[derive(Clone, PartialEq, Eq, Debug, Hash)]
pub struct ConfiguredMcpServer {
    pub url: McpServerUrl,
    pub namespace: Namespace,
}

impl ConfiguredMcpServer {
    pub fn new(url: impl Into<McpServerUrl>, namespace: Namespace) -> Self {
        Self {
            url: url.into(),
            namespace,
        }
    }
}

/// Interact with tool server storage.
///
/// Whereas the [`ToolApi`] allows to interact with tools (and does not care that they are
/// implemented with different MCP Servers), the [`McpServerStore`] allows someone else (e.g.
/// the `NamespaceDescriptionLoaders`) to notify about new or removed tool servers.
#[cfg_attr(test, double(ToolStoreDouble))]
pub trait ToolStoreApi {
    fn mcp_upsert(&self, server: ConfiguredMcpServer) -> impl Future<Output = ()> + Send;
    fn mcp_remove(&self, server: ConfiguredMcpServer) -> impl Future<Output = ()> + Send;
    fn mcp_list(&self, namespace: Namespace) -> impl Future<Output = Vec<McpServerUrl>> + Send;

    // These could become a separate interface, if we decide to have a separate actor for managing
    // MCP servers.
    fn native_tool_upsert(&self, tool: ConfiguredNativeTool) -> impl Future<Output = ()> + Send;

    fn native_tool_remove(&self, tool: ConfiguredNativeTool) -> impl Future<Output = ()> + Send;
}

/// CSI facing interface, allows to invoke and list tools
#[cfg_attr(test, double(ToolDouble))]
pub trait ToolApi {
    fn invoke_tool(
        &self,
        request: InvokeRequest,
        namespace: Namespace,
        tracing_context: TracingContext,
    ) -> impl Future<Output = Result<ToolOutput, ToolError>> + Send;

    fn list_tools(&self, namespace: Namespace) -> impl Future<Output = Vec<String>> + Send;
}

pub struct ToolRuntime {
    handle: JoinHandle<()>,
    send: mpsc::Sender<ToolMsg>,
}

impl ToolRuntime {
    pub fn new() -> Self {
        let client = McpClient::new();
        Self::with_client(client)
    }

    pub fn with_client(client: impl ToolClient) -> Self {
        let (send, receiver) = tokio::sync::mpsc::channel::<ToolMsg>(1);
        let mut actor = ToolActor::new(receiver, client);
        let handle = tokio::spawn(async move { actor.run().await });
        Self { handle, send }
    }

    pub fn api(&self) -> ToolSender {
        ToolSender(self.send.clone())
    }

    pub async fn wait_for_shutdown(self) {
        drop(self.send);
        self.handle.await.unwrap();
    }
}

impl ToolApi for ToolSender {
    async fn invoke_tool(
        &self,
        request: InvokeRequest,
        namespace: Namespace,
        tracing_context: TracingContext,
    ) -> Result<ToolOutput, ToolError> {
        let (send, receive) = oneshot::channel();
        let msg = ToolMsg::InvokeTool {
            request,
            namespace,
            tracing_context,
            send,
        };

        // We know that the receiver is still alive as long as Tool is alive.
        self.0.send(msg).await.unwrap();
        receive.await.unwrap()
    }

    async fn list_tools(&self, namespace: Namespace) -> Vec<String> {
        let (send, receive) = oneshot::channel();
        let msg = ToolMsg::ListTools { send, namespace };
        self.0.send(msg).await.unwrap();
        receive.await.unwrap()
    }
}

/// Opaque wrapper around a sender to the tool actor, so we do not need to expose our message
/// type.
#[derive(Clone)]
pub struct ToolSender(mpsc::Sender<ToolMsg>);

impl ToolStoreApi for ToolSender {
    async fn mcp_upsert(&self, server: ConfiguredMcpServer) {
        let msg = ToolMsg::UpsertToolServer { server };
        self.0.send(msg).await.unwrap();
    }

    async fn mcp_remove(&self, server: ConfiguredMcpServer) {
        let msg = ToolMsg::RemoveToolServer { server };
        self.0.send(msg).await.unwrap();
    }

    async fn mcp_list(&self, namespace: Namespace) -> Vec<McpServerUrl> {
        let (send, receive) = oneshot::channel();
        let msg = ToolMsg::ListToolServers { namespace, send };
        self.0.send(msg).await.unwrap();
        receive.await.unwrap()
    }

    async fn native_tool_upsert(&self, tool: ConfiguredNativeTool) {
        let msg = ToolMsg::UpsertNativeTool { tool };
        self.0.send(msg).await.unwrap();
    }

    async fn native_tool_remove(&self, tool: ConfiguredNativeTool) {
        let msg = ToolMsg::RemoveNativeTool { tool };
        self.0.send(msg).await.unwrap();
    }
}

enum ToolMsg {
    InvokeTool {
        request: InvokeRequest,
        namespace: Namespace,
        tracing_context: TracingContext,
        send: oneshot::Sender<Result<Vec<Modality>, ToolError>>,
    },
    ListTools {
        namespace: Namespace,
        send: oneshot::Sender<Vec<String>>,
    },
    UpsertToolServer {
        server: ConfiguredMcpServer,
    },
    RemoveToolServer {
        server: ConfiguredMcpServer,
    },
    ListToolServers {
        namespace: Namespace,
        send: oneshot::Sender<Vec<McpServerUrl>>,
    },
    UpsertNativeTool {
        tool: ConfiguredNativeTool,
    },
    RemoveNativeTool {
        tool: ConfiguredNativeTool,
    },
}

struct ToolActor<T: ToolClient> {
    toolbox: Toolbox<T>,
    receiver: mpsc::Receiver<ToolMsg>,
    running_requests: FuturesUnordered<Pin<Box<dyn Future<Output = ()> + Send>>>,
}

impl<T: ToolClient> ToolActor<T> {
    fn new(receiver: mpsc::Receiver<ToolMsg>, client: T) -> Self {
        Self {
            toolbox: Toolbox::new(client),
            receiver,
            running_requests: FuturesUnordered::new(),
        }
    }

    async fn run(&mut self) {
        // While there are messages and running requests, poll both.
        // If there is a message, add it to the queue.
        // If there are running requests, make progress on them.
        loop {
            select! {
                msg = self.receiver.recv() => match msg {
                    Some(msg) => self.act(msg),
                    None => break
                },
                () = self.running_requests.select_next_some(), if !self.running_requests.is_empty() => {}
            };
        }
    }

    fn act(&mut self, msg: ToolMsg) {
        match msg {
            ToolMsg::InvokeTool {
                request,
                namespace,
                tracing_context,
                send,
            } => {
                let tool = self.toolbox.fetch_tool(namespace, &request.name);
                self.running_requests.push(Box::pin(async move {
                    let result = tool
                        .expect("Can never returned None until the Toolbox is caching the tool")
                        .invoke(request.arguments, tracing_context)
                        .await;
                    drop(send.send(result));
                }));
            }
            ToolMsg::ListTools { namespace, send } => {
                let client = self.toolbox.client.clone();
                let servers = self
                    .toolbox
                    .list_mcp_servers_in_namespace(&namespace)
                    .collect();
                self.running_requests.push(Box::pin(async move {
                    let result = Self::tools(client.as_ref(), servers).await;
                    let result = result.into_values().flatten().collect();
                    drop(send.send(result));
                }));
            }
            ToolMsg::UpsertToolServer {
                server: ConfiguredMcpServer { url, namespace },
            } => {
                self.toolbox.upsert_mcp_server(namespace, url);
            }
            ToolMsg::RemoveToolServer {
                server: ConfiguredMcpServer { url, namespace },
            } => {
                self.toolbox.remove_mcp_server(namespace, url);
            }
            ToolMsg::ListToolServers { namespace, send } => {
                let result = self
                    .toolbox
                    .list_mcp_servers_in_namespace(&namespace)
                    .collect();
                drop(send.send(result));
            }
            ToolMsg::UpsertNativeTool { tool } => self.toolbox.upsert_native_tool(tool),
            ToolMsg::RemoveNativeTool { tool } => self.toolbox.remove_native_tool(tool),
        }
    }

    /// Returns all tools found on any of the configured tool servers.
    ///
    /// We do not return a result here, but ignore MCP servers that are giving errors.
    /// A skill only is interested in a subset of tools. At some layer, we need to trade in
    /// tool names for the json schema. There would be the appropriate place to decide if the
    /// available information is enough.
    async fn tools(
        client: &impl ToolClient,
        servers: Vec<McpServerUrl>,
    ) -> HashMap<McpServerUrl, Vec<String>> {
        let results = join_all(servers.into_iter().map(|address| async move {
            let tools = client.list_tools(&address).await;
            (address, tools)
        }))
        .await;
        results
            .into_iter()
            .filter_map(|(address, result)| result.ok().map(|tools| (address, tools)))
            .collect()
    }
}

pub struct InvokeRequest {
    pub name: String,
    pub arguments: Vec<Argument>,
}

#[cfg_attr(test, double(ToolClientDouble))]
pub trait ToolClient: Send + Sync + 'static {
    fn invoke_tool(
        &self,
        name: &str,
        arguments: Vec<Argument>,
        url: &McpServerUrl,
        tracing_context: TracingContext,
    ) -> impl Future<Output = Result<Vec<Modality>, ToolError>> + Send;

    fn list_tools(
        &self,
        url: &McpServerUrl,
    ) -> impl Future<Output = Result<Vec<String>, anyhow::Error>> + Send + Sync;
}

#[cfg(test)]
pub mod tests {
    use core::panic;
    use std::{collections::HashSet, sync::Arc, time::Duration};

    use futures::future::pending;
    use tokio::sync::Mutex;

    use crate::{
        logging::TracingContext,
        tool::{ToolRuntime, actor::ToolStoreApi},
    };

    use super::*;

    /// Only report tools for one particular server address
    struct ToolClientMock;

    impl ToolClientDouble for ToolClientMock {
        async fn list_tools(&self, url: &McpServerUrl) -> Result<Vec<String>, anyhow::Error> {
            if url.0 == "http://localhost:8000/mcp" {
                Ok(vec!["add".to_owned()])
            } else {
                panic!("This client only knows the localhost:8000 mcp server")
            }
        }

        async fn invoke_tool(
            &self,
            _name: &str,
            _args: Vec<Argument>,
            url: &McpServerUrl,
            _tracing_context: TracingContext,
        ) -> Result<Vec<Modality>, ToolError> {
            if url.0 == "http://localhost:8000/mcp" {
                Ok(vec![Modality::Text {
                    text: "Success".to_owned(),
                }])
            } else {
                panic!("This client only knows the localhost:8000 mcp server")
            }
        }
    }

    #[tokio::test]
    async fn invoking_unknown_tool_without_servers_gives_tool_not_found() {
        // Given a tool client
        let tool = ToolRuntime::with_client(ToolClientMock).api();

        // When we invoke an unknown tool
        let request = InvokeRequest {
            name: "unknown".to_owned(),
            arguments: vec![],
        };
        let result = tool
            .invoke_tool(request, Namespace::dummy(), TracingContext::dummy())
            .await;

        // Then we get a tool not found error
        assert!(matches!(result, Err(ToolError::ToolNotFound(_))));
    }

    #[tokio::test]
    async fn tool_server_is_available_for_configured_namespace() {
        // Given a tool client that only reports tools for a particular url
        let namespace = Namespace::new("test").unwrap();
        let tool = ToolRuntime::with_client(ToolClientMock)
            .with_servers(vec![ConfiguredMcpServer::new(
                "http://localhost:8000/mcp",
                namespace.clone(),
            )])
            .await
            .api();

        // When we invoke a tool that the mock client supports for the configured namespace
        let request = InvokeRequest {
            name: "add".to_owned(),
            arguments: vec![],
        };
        let result = tool
            .invoke_tool(request, namespace, TracingContext::dummy())
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0], Modality::Text { text } if text == "Success"));
    }

    #[tokio::test]
    async fn tool_server_from_different_namespace_is_not_available() {
        // Given an mcp server available for the foo namespace
        let foo = Namespace::new("foo").unwrap();
        let tool = ToolRuntime::with_client(ToolClientMock)
            .with_servers(vec![ConfiguredMcpServer::new(
                "http://localhost:800/mcp",
                foo,
            )])
            .await
            .api();

        // When we invoke a tool from a different namespace
        let request = InvokeRequest {
            name: "add".to_owned(),
            arguments: vec![],
        };
        let result = tool
            .invoke_tool(
                request,
                Namespace::new("bar").unwrap(),
                TracingContext::dummy(),
            )
            .await;

        // Then we get a tool not found error
        assert!(matches!(result, Err(ToolError::ToolNotFound(_))));
    }

    #[tokio::test]
    async fn tool_server_is_removed() {
        // Given a tool configured with two mcp servers for a namespace
        let namespace = Namespace::new("test").unwrap();
        let servers = vec![
            ConfiguredMcpServer::new("http://localhost:8000/mcp", namespace.clone()),
            ConfiguredMcpServer::new("http://localhost:8001/mcp", namespace.clone()),
        ];
        let tool = ToolRuntime::with_client(ToolClientMock)
            .with_servers(servers.clone())
            .await
            .api();

        // When the first mcp server is removed
        tool.mcp_remove(servers[0].clone()).await;

        // Then the list of tools is only the second mcp server
        let mcp_servers = tool.mcp_list(namespace).await;
        assert_eq!(mcp_servers, vec!["http://localhost:8001/mcp".into()]);
    }

    #[tokio::test]
    async fn adding_existing_tool_server_does_not_add_it_again() {
        // Given a tool client that knows about two mcp servers for a namespace
        let tool = ToolRuntime::with_client(ToolClientMock).api();
        let namespace = Namespace::new("test").unwrap();
        let server = ConfiguredMcpServer::new("http://localhost:8000/mcp", namespace.clone());
        tool.mcp_upsert(server.clone()).await;

        // When we add the same tool server again for that namespace
        tool.mcp_upsert(server).await;

        // Then the list of tools is still the same
        let mcp_servers = tool.mcp_list(namespace).await;
        assert_eq!(mcp_servers, vec!["http://localhost:8000/mcp".into()]);
    }

    #[tokio::test]
    async fn same_tool_server_can_be_configured_for_different_namespaces() {
        // Given a tool client
        let tool = ToolRuntime::with_client(ToolClientMock).api();
        let first = Namespace::new("first").unwrap();

        // When adding the same tool server for two namespaces
        let server_url = "http://localhost:8000/mcp";
        let server = ConfiguredMcpServer::new(server_url, first.clone());
        tool.mcp_upsert(server.clone()).await;

        let second = Namespace::new("second").unwrap();
        let server = ConfiguredMcpServer::new(server_url, second.clone());
        tool.mcp_upsert(server).await;

        // Then the server is configured for both namespaces
        let mcp_servers = tool.mcp_list(first).await;
        assert_eq!(mcp_servers, vec![server_url.into()]);

        let mcp_servers = tool.mcp_list(second).await;
        assert_eq!(mcp_servers, vec![server_url.into()]);
    }

    struct ToolClientSpy {
        queried: Arc<Mutex<HashSet<McpServerUrl>>>,
    }

    impl ToolClientSpy {
        fn new(queried: Arc<Mutex<HashSet<McpServerUrl>>>) -> Self {
            Self { queried }
        }
    }

    impl ToolClientDouble for ToolClientSpy {
        async fn list_tools(&self, url: &McpServerUrl) -> Result<Vec<String>, anyhow::Error> {
            let mut queried = self.queried.lock().await;
            queried.insert(url.to_owned());
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn tools_from_multiple_tool_servers_are_available() {
        // Given a tool with two configured tool servers for a namespace
        let queried = Arc::new(Mutex::new(HashSet::new()));
        let namespace = Namespace::new("test").unwrap();
        let servers = vec![
            ConfiguredMcpServer::new("http://localhost:8000/mcp", namespace.clone()),
            ConfiguredMcpServer::new("http://localhost:8001/mcp", namespace.clone()),
        ];
        let tool = ToolRuntime::with_client(ToolClientSpy::new(queried.clone()))
            .with_servers(servers.clone())
            .await
            .api();

        // When we ask for the list of tools for that namespace
        drop(tool.list_tools(namespace).await);

        // Then both tool servers are queried
        let queried = queried.lock().await.clone();
        assert_eq!(queried.len(), 2);
        assert!(
            queried
                .iter()
                .any(|url| url.0 == "http://localhost:8000/mcp")
        );
        assert!(
            queried
                .iter()
                .any(|url| url.0 == "http://localhost:8001/mcp")
        );
    }

    // Given a tool client that knows about two mcp servers
    struct TwoServerClient;

    impl ToolClientDouble for TwoServerClient {
        async fn list_tools(&self, url: &McpServerUrl) -> Result<Vec<String>, anyhow::Error> {
            match url.0.as_ref() {
                "http://localhost:8000/mcp" => Ok(vec!["search".to_owned()]),
                "http://localhost:8001/mcp" => Ok(vec!["calculator".to_owned()]),
                _ => {
                    panic!("Unknown mcp server asked for tools.")
                }
            }
        }

        async fn invoke_tool(
            &self,
            name: &str,
            _args: Vec<Argument>,
            url: &McpServerUrl,
            _tracing_context: TracingContext,
        ) -> Result<Vec<Modality>, ToolError> {
            match url.0.as_ref() {
                "http://localhost:8000/mcp" => {
                    assert_eq!(name, "search");
                    Ok(vec![Modality::Text {
                        text: "search result".to_owned(),
                    }])
                }
                "http://localhost:8001/mcp" => {
                    assert_eq!(name, "calculator");
                    Ok(vec![Modality::Text {
                        text: "calculator result".to_owned(),
                    }])
                }
                _ => Err(ToolError::Other(anyhow::anyhow!(
                    "Requested rooted to the wrong server."
                ))),
            }
        }
    }

    #[tokio::test]
    async fn correct_mcp_server_is_called_for_tool_invocation() {
        // Given a tool client that knows about two mcp servers
        let namespace = Namespace::new("test").unwrap();
        let servers = vec![
            ConfiguredMcpServer::new("http://localhost:8000/mcp", namespace.clone()),
            ConfiguredMcpServer::new("http://localhost:8001/mcp", namespace.clone()),
        ];
        let tool = ToolRuntime::with_client(TwoServerClient)
            .with_servers(servers)
            .await
            .api();

        // When we invoke the search tool
        let result = tool
            .invoke_tool(
                InvokeRequest {
                    name: "search".to_owned(),
                    arguments: vec![],
                },
                namespace,
                TracingContext::dummy(),
            )
            .await
            .unwrap();

        // Then we get the result from the brave_search server
        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0], Modality::Text { text } if text == "search result"));
    }

    struct ClientOneGoodOtherBad;

    impl ToolClientDouble for ClientOneGoodOtherBad {
        async fn list_tools(&self, url: &McpServerUrl) -> Result<Vec<String>, anyhow::Error> {
            if url.0 == "http://localhost:8000/mcp" {
                Ok(vec!["search".to_owned()])
            } else {
                Err(anyhow::anyhow!("Request to mcp server timed out."))
            }
        }

        async fn invoke_tool(
            &self,
            _name: &str,
            _args: Vec<Argument>,
            _url: &McpServerUrl,
            _tracing_context: TracingContext,
        ) -> Result<Vec<Modality>, ToolError> {
            Ok(vec![Modality::Text {
                text: "Success".to_owned(),
            }])
        }
    }

    impl ToolRuntime {
        async fn with_servers(self, servers: Vec<ConfiguredMcpServer>) -> Self {
            let api = self.api();
            for server in servers {
                api.mcp_upsert(server).await;
            }
            self
        }
    }

    #[tokio::test]
    async fn list_tool_servers_none_configured() {
        // Given a tool client that knows about no mcp servers
        let tool = ToolRuntime::with_client(ToolClientMock).api();

        // When listing tool servers for a namespace
        let result = tool.mcp_list(Namespace::new("test").unwrap()).await;

        // Then we get an empty list
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn list_tool_servers() {
        // Given a tool client that knows about two mcp servers for a namespace
        let namespace = Namespace::new("test").unwrap();
        let tool = ToolRuntime::with_client(ToolClientMock)
            .with_servers(vec![
                ConfiguredMcpServer::new("http://localhost:8000/mcp", namespace.clone()),
                ConfiguredMcpServer::new("http://localhost:8001/mcp", namespace.clone()),
            ])
            .await
            .api();

        // When listing tool servers for that namespace
        let result: HashSet<McpServerUrl> = tool.mcp_list(namespace).await.into_iter().collect();

        // Then we get the two mcp servers
        let expected = HashSet::from([
            "http://localhost:8000/mcp".into(),
            "http://localhost:8001/mcp".into(),
        ]);
        assert_eq!(result, expected);
    }

    // Client that, independent of the mcp server, always reports the add tool to be available
    struct AddTool;

    impl ToolClientDouble for AddTool {
        async fn list_tools(&self, _url: &McpServerUrl) -> Result<Vec<String>, anyhow::Error> {
            Ok(vec!["add".to_owned()])
        }
    }

    #[tokio::test]
    async fn tools_from_server_are_listed() {
        // Given a tool configured with one namespace and a client that always reports tools for
        // a particular mcp server
        let namespace = Namespace::new("test").unwrap();
        let tool = ToolRuntime::with_client(AddTool)
            .with_servers(vec![ConfiguredMcpServer::new(
                "http://localhost:8000/mcp",
                namespace.clone(),
            )])
            .await
            .api();

        // When listing tools for that namespace
        let result = tool.list_tools(namespace).await;

        // Then we get the tools from the mcp server
        assert_eq!(result, vec!["add".to_owned()]);
    }

    #[tokio::test]
    async fn tools_from_other_namespace_are_not_listed() {
        // Given a tool configured with one namespace and a client that always reports tools for
        // a particular mcp server
        let namespace = Namespace::new("test").unwrap();
        let tool = ToolRuntime::with_client(AddTool)
            .with_servers(vec![ConfiguredMcpServer::new(
                "http://localhost:8000/mcp",
                namespace.clone(),
            )])
            .await
            .api();

        // When listing tools for a different namespace
        let result = tool.list_tools(Namespace::new("other").unwrap()).await;

        // Then we get an empty list
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn one_bad_mcp_server_still_allows_to_list_tools() {
        // Given an mcp server that returns an error when listing it's tools
        let namespace = Namespace::new("test").unwrap();
        let servers = vec![
            ConfiguredMcpServer::new("http://localhost:8000/mcp", namespace.clone()),
            ConfiguredMcpServer::new("http://localhost:8001/mcp", namespace.clone()),
        ];
        let tool = ToolRuntime::with_client(ClientOneGoodOtherBad)
            .with_servers(servers)
            .await
            .api();

        // When listing tools
        let result = tool.list_tools(namespace).await;

        // Then the search tool is available
        assert_eq!(result, vec!["search".to_owned()]);
    }

    #[tokio::test]
    async fn one_bad_mcp_server_still_allows_to_invoke_tool() {
        // Given an mcp server that returns an error when listing it's tools
        let namespace = Namespace::new("test").unwrap();
        let servers = vec![
            ConfiguredMcpServer::new("http://localhost:8000/mcp", namespace.clone()),
            ConfiguredMcpServer::new("http://localhost:8001/mcp", namespace.clone()),
        ];
        let tool = ToolRuntime::with_client(ClientOneGoodOtherBad)
            .with_servers(servers)
            .await
            .api();

        // When invoking a tool
        let request = InvokeRequest {
            name: "search".to_owned(),
            arguments: vec![],
        };
        let result = tool
            .invoke_tool(request, namespace, TracingContext::dummy())
            .await
            .unwrap();

        // Then the search tool is available
        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0], Modality::Text { text } if text == "Success"));
    }

    struct SaboteurClient;

    impl ToolClientDouble for SaboteurClient {
        async fn invoke_tool(
            &self,
            name: &str,
            _args: Vec<Argument>,
            _url: &McpServerUrl,
            _tracing_context: TracingContext,
        ) -> Result<Vec<Modality>, ToolError> {
            match name {
                "add" => Ok(vec![Modality::Text {
                    text: "Success".to_owned(),
                }]),
                "divide" => pending().await,
                _ => panic!("unknown function called"),
            }
        }

        async fn list_tools(&self, _url: &McpServerUrl) -> Result<Vec<String>, anyhow::Error> {
            Ok(vec!["add".to_owned(), "divide".to_owned()])
        }
    }

    #[tokio::test]
    async fn tool_calls_are_processed_in_parallel() {
        // Given a tool that hangs forever for some tool invocation requestsa
        let namespace = Namespace::new("test").unwrap();
        let tool = ToolRuntime::with_client(SaboteurClient)
            .with_servers(vec![ConfiguredMcpServer::new(
                "http://localhost:8000/mcp",
                namespace.clone(),
            )])
            .await;
        let api = tool.api();

        // When one hanging request is in progress
        let request = InvokeRequest {
            name: "divide".to_owned(),
            arguments: vec![],
        };
        let cloned_namespace = namespace.clone();
        let handle = tokio::spawn(async move {
            drop(
                api.invoke_tool(request, cloned_namespace, TracingContext::dummy())
                    .await,
            );
        });

        // And waiting shortly for the message to arrive
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Then another request can still be answered
        let request = InvokeRequest {
            name: "add".to_owned(),
            arguments: vec![],
        };
        let result = tool
            .api()
            .invoke_tool(request, namespace, TracingContext::dummy())
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0], Modality::Text { text } if text == "Success"));

        drop(handle);
    }
}
