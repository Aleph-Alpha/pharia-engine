use futures::{StreamExt, future::join_all, stream::FuturesUnordered};
use std::{collections::HashMap, pin::Pin};
use tokio::{
    select,
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use crate::{
    logging::TracingContext,
    mcp::{ConfiguredMcpServer, McpClient, McpClientImpl, McpServerUrl},
    namespace_watcher::Namespace,
    tool::{
        Argument, Modality, QualifiedToolName, ToolError, ToolOutput,
        toolbox::{ConfiguredNativeTool, Toolbox},
    },
};

#[cfg(test)]
use double_trait::double;

/// Interact with tool server storage.
///
/// Whereas the [`ToolApi`] allows to interact with tools (and does not care that they are
/// implemented with different MCP Servers), the [`McpServerStore`] allows someone else (e.g.
/// the `NamespaceDescriptionLoaders`) to notify about new or removed tool servers.
#[cfg_attr(test, double(ToolStoreDouble))]
pub trait ToolStoreApi {
    fn mcp_upsert(&self, server: ConfiguredMcpServer) -> impl Future<Output = ()> + Send;
    fn mcp_remove(&self, server: ConfiguredMcpServer) -> impl Future<Output = ()> + Send;

    // These could become a separate interface, if we decide to have a separate actor for managing
    // MCP servers.
    fn native_tool_upsert(&self, tool: ConfiguredNativeTool) -> impl Future<Output = ()> + Send;

    fn native_tool_remove(&self, tool: ConfiguredNativeTool) -> impl Future<Output = ()> + Send;
}

/// CSI facing interface, allows to invoke and list tools
#[cfg_attr(test, double(ToolRuntimeDouble))]
pub trait ToolRuntimeApi {
    fn invoke_tool(
        &self,
        tool: QualifiedToolName,
        arguments: Vec<Argument>,
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
        let client = McpClientImpl::new();
        Self::with_client(client)
    }

    pub fn with_client(client: impl McpClient) -> Self {
        let (send, receiver) = tokio::sync::mpsc::channel::<ToolMsg>(1);
        let mut actor = ToolActor::new(receiver, client);
        let handle = tokio::spawn(async move { actor.run().await });
        Self { handle, send }
    }

    pub fn api(&self) -> ToolRuntimeSender {
        ToolRuntimeSender(self.send.clone())
    }

    pub async fn wait_for_shutdown(self) {
        drop(self.send);
        self.handle.await.unwrap();
    }
}

impl ToolRuntimeApi for ToolRuntimeSender {
    async fn invoke_tool(
        &self,
        tool: QualifiedToolName,
        arguments: Vec<Argument>,
        tracing_context: TracingContext,
    ) -> Result<ToolOutput, ToolError> {
        let (send, receive) = oneshot::channel();
        let msg = ToolMsg::InvokeTool {
            name: tool,
            arguments,
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
pub struct ToolRuntimeSender(mpsc::Sender<ToolMsg>);

impl ToolStoreApi for ToolRuntimeSender {
    async fn mcp_upsert(&self, server: ConfiguredMcpServer) {
        let msg = ToolMsg::UpsertToolServer { server };
        self.0.send(msg).await.unwrap();
    }

    async fn mcp_remove(&self, server: ConfiguredMcpServer) {
        let msg = ToolMsg::RemoveToolServer { server };
        self.0.send(msg).await.unwrap();
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
        name: QualifiedToolName,
        arguments: Vec<Argument>,
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
    UpsertNativeTool {
        tool: ConfiguredNativeTool,
    },
    RemoveNativeTool {
        tool: ConfiguredNativeTool,
    },
}

struct ToolActor<T: McpClient> {
    toolbox: Toolbox<T>,
    receiver: mpsc::Receiver<ToolMsg>,
    running_requests: FuturesUnordered<Pin<Box<dyn Future<Output = ()> + Send>>>,
}

impl<T: McpClient> ToolActor<T> {
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
                    Some(msg) => self.act(msg).await,
                    None => break
                },
                () = self.running_requests.select_next_some(), if !self.running_requests.is_empty() => {}
            };
        }
    }

    async fn act(&mut self, msg: ToolMsg) {
        match msg {
            ToolMsg::InvokeTool {
                name: qualified_name,
                arguments,
                tracing_context,
                send,
            } => {
                let maybe_tool = self
                    .toolbox
                    .fetch_tool(&qualified_name)
                    .await;
                if let Some(tool) = maybe_tool {
                    self.running_requests.push(Box::pin(async move {
                        let result = tool.invoke(arguments, tracing_context).await;
                        drop(send.send(result));
                    }));
                } else {
                    drop(send.send(Err(ToolError::ToolNotFound(qualified_name.name))));
                }
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
                self.toolbox.upsert_mcp_server(namespace, url).await;
            }
            ToolMsg::RemoveToolServer {
                server: ConfiguredMcpServer { url, namespace },
            } => {
                self.toolbox.remove_mcp_server(namespace, url);
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
        client: &impl McpClient,
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

#[cfg(test)]
pub mod tests {
    use core::panic;
    use std::{collections::HashSet, sync::Arc, time::Duration};

    use futures::future::pending;
    use tokio::sync::Mutex;

    use crate::{
        logging::TracingContext,
        mcp::ToolClientDouble,
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
                panic!("This client only knows the localhost:8000/mcp server")
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
        let tool_name = QualifiedToolName {
            namespace: Namespace::dummy(),
            name: "unknown".to_owned(),
        };
        let arguments = Vec::new();
        let result = tool
            .invoke_tool(tool_name, arguments, TracingContext::dummy())
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
        let tool_name = QualifiedToolName {
            namespace: namespace.clone(),
            name: "add".to_owned(),
        };
        let arguments = vec![];
        let result = tool
            .invoke_tool(tool_name, arguments, TracingContext::dummy())
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
                "http://localhost:8000/mcp",
                foo,
            )])
            .await
            .api();

        // When we invoke a tool from a different namespace
        let tool_name = QualifiedToolName {
            namespace: Namespace::new("bar").unwrap(),
            name: "add".to_owned(),
        };
        let arguments = vec![];
        let result = tool
            .invoke_tool(tool_name, arguments, TracingContext::dummy())
            .await;

        // Then we get a tool not found error
        assert!(matches!(result, Err(ToolError::ToolNotFound(_))));
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
        let tool_name = QualifiedToolName {
            namespace: namespace.clone(),
            name: "search".to_owned(),
        };
        let result = tool
            .invoke_tool(tool_name, vec![], TracingContext::dummy())
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
        let tool_name = QualifiedToolName {
            namespace: namespace.clone(),
            name: "search".to_owned(),
        };
        let arguments = vec![];
        let result = tool
            .invoke_tool(tool_name, arguments, TracingContext::dummy())
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
        let tool_name = QualifiedToolName {
            namespace: namespace.clone(),
            name: "divide".to_owned(),
        };
        let cloned_api = api.clone();
        let handle = tokio::spawn(async move {
            drop(
                cloned_api
                    .invoke_tool(tool_name, vec![], TracingContext::dummy())
                    .await,
            );
        });

        // And waiting shortly for the message to arrive
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Then another request can still be answered
        let tool_name = QualifiedToolName {
            namespace: namespace.clone(),
            name: "add".to_owned(),
        };
        let result = api
            .invoke_tool(tool_name, vec![], TracingContext::dummy())
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0], Modality::Text { text } if text == "Success"));

        drop(handle);
    }
}
