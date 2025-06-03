use futures::StreamExt;
use futures::future::join_all;
use futures::stream::FuturesUnordered;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::collections::HashSet;
use std::pin::Pin;
use std::sync::Arc;
use tokio::select;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tracing::info;

use crate::logging::TracingContext;
use crate::namespace_watcher::Namespace;

use super::client::McpClient;

#[derive(Clone, Hash, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct McpServerUrl(pub String);

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

impl<T> From<T> for McpServerUrl
where
    T: Into<String>,
{
    fn from(value: T) -> Self {
        Self(value.into())
    }
}
/// Interact with tool server storage.
///
/// Whereas the [`ToolApi`] allows to interact with tools (and does not care that they are
/// implemented with different MCP Servers), the [`ToolStoreApi`] allows someone else (e.g.
/// the `NamespaceDescriptionLoaders`) to notify about new or removed tool servers.
pub trait ToolStoreApi {
    fn upsert_tool_server(&self, server: ConfiguredMcpServer) -> impl Future<Output = ()> + Send;
    fn remove_tool_server(&self, server: ConfiguredMcpServer) -> impl Future<Output = ()> + Send;

    // While this is not used yet (from e.g. the shell), it represents the public surface
    // to test the tool store.
    #[allow(dead_code)]
    fn list_tool_servers(
        &self,
        namespace: Namespace,
    ) -> impl Future<Output = Vec<McpServerUrl>> + Send;
}

pub trait ToolApi {
    fn invoke_tool(
        &self,
        request: InvokeRequest,
        tracing_context: TracingContext,
    ) -> impl Future<Output = Result<Value, ToolError>> + Send;

    #[allow(dead_code)]
    fn list_tools(&self) -> impl Future<Output = Vec<String>> + Send;
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    // We plan on passing this error variant to the Skills and model.
    #[error("{0}")]
    ToolCallFailed(String),
    // This variant should also be exposed to the Skill, as the model can recover from this.
    #[error("Tool {0} not found on any server.")]
    ToolNotFound(String),
    #[error("The tool call could not be executed, original error: {0}")]
    Other(#[from] anyhow::Error),
}

pub struct Tool {
    handle: JoinHandle<()>,
    send: mpsc::Sender<ToolMsg>,
}

impl Tool {
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

    pub fn api(&self) -> impl ToolApi + ToolStoreApi + Send + Sync + Clone + 'static {
        self.send.clone()
    }

    pub async fn wait_for_shutdown(self) {
        drop(self.send);
        self.handle.await.unwrap();
    }
}

impl ToolApi for mpsc::Sender<ToolMsg> {
    async fn invoke_tool(
        &self,
        request: InvokeRequest,
        tracing_context: TracingContext,
    ) -> Result<Value, ToolError> {
        let (send, receive) = oneshot::channel();
        let msg = ToolMsg::InvokeTool {
            request,
            tracing_context,
            send,
        };

        // We know that the receiver is still alive as long as Tool is alive.
        self.send(msg).await.unwrap();
        receive.await.unwrap()
    }

    async fn list_tools(&self) -> Vec<String> {
        let (send, receive) = oneshot::channel();
        let msg = ToolMsg::ListTools { send };
        self.send(msg).await.unwrap();
        receive.await.unwrap()
    }
}

impl ToolStoreApi for mpsc::Sender<ToolMsg> {
    async fn upsert_tool_server(&self, server: ConfiguredMcpServer) {
        let msg = ToolMsg::UpsertToolServer { server };
        self.send(msg).await.unwrap();
    }

    async fn remove_tool_server(&self, server: ConfiguredMcpServer) {
        let msg = ToolMsg::RemoveToolServer { server };
        self.send(msg).await.unwrap();
    }

    async fn list_tool_servers(&self, namespace: Namespace) -> Vec<McpServerUrl> {
        let (send, receive) = oneshot::channel();
        let msg = ToolMsg::ListToolServers { namespace, send };
        self.send(msg).await.unwrap();
        receive.await.unwrap()
    }
}

enum ToolMsg {
    InvokeTool {
        request: InvokeRequest,
        tracing_context: TracingContext,
        send: oneshot::Sender<Result<Value, ToolError>>,
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
    ListTools {
        send: oneshot::Sender<Vec<String>>,
    },
}

struct ToolActor<T: ToolClient> {
    // Map of which MCP servers are configured for which namespace.
    mcp_servers: HashMap<Namespace, HashSet<McpServerUrl>>,
    receiver: mpsc::Receiver<ToolMsg>,
    running_requests: FuturesUnordered<Pin<Box<dyn Future<Output = ()> + Send>>>,
    client: Arc<T>,
}

impl<T: ToolClient> ToolActor<T> {
    fn new(receiver: mpsc::Receiver<ToolMsg>, client: T) -> Self {
        Self {
            mcp_servers: HashMap::new(),
            receiver,
            client: Arc::new(client),
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

    /// All configured servers, independent of the namespace.
    ///
    /// While Skills should only have access to the MCP servers for their namespace, we do not
    /// have the namespace information available at the tool level (yet). This method allows
    /// us to move incrementally and introduce the namespace concept at the actor level already.
    fn all_servers(&self) -> HashSet<McpServerUrl> {
        self.mcp_servers.values().flatten().cloned().collect()
    }

    fn act(&mut self, msg: ToolMsg) {
        match msg {
            ToolMsg::InvokeTool {
                request,
                tracing_context,
                send,
            } => {
                let client = self.client.clone();
                let servers = self.all_servers();
                self.running_requests.push(Box::pin(async move {
                    let result =
                        Self::invoke_tool(client.as_ref(), servers, request, tracing_context).await;
                    drop(send.send(result));
                }));
            }
            ToolMsg::ListTools { send } => {
                let client = self.client.clone();
                let servers = self.all_servers();
                self.running_requests.push(Box::pin(async move {
                    let result = Self::tools(client.as_ref(), servers).await;
                    let result = result.into_values().flatten().collect();
                    drop(send.send(result));
                }));
            }
            ToolMsg::UpsertToolServer {
                server: ConfiguredMcpServer { url, namespace },
            } => {
                self.mcp_servers.entry(namespace).or_default().insert(url);
            }
            ToolMsg::RemoveToolServer {
                server: ConfiguredMcpServer { url, namespace },
            } => {
                if let Some(servers) = self.mcp_servers.get_mut(&namespace) {
                    servers.remove(&url);
                }
            }
            ToolMsg::ListToolServers { namespace, send } => {
                let result = self
                    .mcp_servers
                    .get(&namespace)
                    .map(|s| s.iter().cloned().collect())
                    .unwrap_or_default();
                drop(send.send(result));
            }
        }
    }

    /// Invoke a tool and return the result.
    ///
    /// First, we need to locate the server on which the tool is hosted.
    /// Then, we invoke the tool on that server.
    async fn invoke_tool(
        client: &impl ToolClient,
        servers: HashSet<McpServerUrl>,
        request: InvokeRequest,
        tracing_context: TracingContext,
    ) -> Result<Value, ToolError> {
        let mcp_address = Self::server_for_tool(client, servers, &request.tool_name)
            .await
            .ok_or(ToolError::ToolNotFound(request.tool_name.clone()))
            .inspect_err(|e| info!(parent: tracing_context.span(), "{}", e))?;
        client
            .invoke_tool(request, &mcp_address, tracing_context)
            .await
    }

    /// Returns all tools found on any of the configured tool servers.
    ///
    /// We do not return a result here, but ignore MCP servers that are giving errors.
    /// A skill only is interested in a subset of tools. At some layer, we need to trade in
    /// tool names for the json schema. There would be the appropriate place to decide if the
    /// available information is enough.
    async fn tools(
        client: &impl ToolClient,
        servers: HashSet<McpServerUrl>,
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

    /// Return the server that hosts a particular tool, or None if the tool can not be found.
    ///
    /// We do not return a result here, but ignore MCP servers that are giving errors.
    async fn server_for_tool(
        client: &impl ToolClient,
        servers: HashSet<McpServerUrl>,
        tool: &str,
    ) -> Option<McpServerUrl> {
        let all_tools = Self::tools(client, servers).await;
        for (address, tools) in &all_tools {
            if tools.contains(&tool.to_owned()) {
                return Some(address.clone());
            }
        }
        None
    }
}

pub struct Argument {
    pub name: String,
    pub value: Vec<u8>,
}

pub struct InvokeRequest {
    pub tool_name: String,
    pub arguments: Vec<Argument>,
}

pub trait ToolClient: Send + Sync + 'static {
    fn invoke_tool(
        &self,
        request: InvokeRequest,
        url: &McpServerUrl,
        tracing_context: TracingContext,
    ) -> impl Future<Output = Result<Value, ToolError>> + Send;

    fn list_tools(
        &self,
        url: &McpServerUrl,
    ) -> impl Future<Output = Result<Vec<String>, anyhow::Error>> + Send;
}

#[cfg(test)]
pub mod tests {
    use core::panic;
    use std::{sync::Arc, time::Duration};

    use futures::future::pending;
    use serde_json::{Value, json};
    use tokio::sync::Mutex;

    use crate::{
        logging::TracingContext,
        tool::{Tool, actor::ToolStoreApi},
    };

    use super::*;

    pub struct ToolStoreDouble;

    impl ToolStoreApi for ToolStoreDouble {
        async fn upsert_tool_server(&self, _server: ConfiguredMcpServer) {}

        async fn remove_tool_server(&self, _server: ConfiguredMcpServer) {}

        async fn list_tool_servers(&self, _namespace: Namespace) -> Vec<McpServerUrl> {
            vec![]
        }
    }

    pub struct ToolDouble;

    impl ToolApi for ToolDouble {
        async fn invoke_tool(
            &self,
            _request: InvokeRequest,
            _tracing_context: TracingContext,
        ) -> Result<Value, ToolError> {
            unimplemented!()
        }

        async fn list_tools(&self) -> Vec<String> {
            unimplemented!()
        }
    }

    /// Only report tools for one particular server address
    struct ToolClientMock;

    impl ToolClient for ToolClientMock {
        async fn list_tools(&self, url: &McpServerUrl) -> Result<Vec<String>, anyhow::Error> {
            if url.0 == "http://localhost:8000/mcp" {
                Ok(vec!["add".to_owned()])
            } else {
                panic!("This client only knows the localhost:8000 mcp server")
            }
        }

        async fn invoke_tool(
            &self,
            _request: InvokeRequest,
            url: &McpServerUrl,
            _tracing_context: TracingContext,
        ) -> Result<Value, ToolError> {
            if url.0 == "http://localhost:8000/mcp" {
                Ok(json!("Success"))
            } else {
                panic!("This client only knows the localhost:8000 mcp server")
            }
        }
    }

    #[tokio::test]
    async fn invoking_unknown_tool_without_servers_gives_tool_not_found() {
        // Given a tool client
        let tool = Tool::with_client(ToolClientMock).api();

        // When we invoke an unknown tool
        let request = InvokeRequest {
            tool_name: "unknown".to_owned(),
            arguments: vec![],
        };
        let result = tool.invoke_tool(request, TracingContext::dummy()).await;

        // Then we get a tool not found error
        assert!(matches!(result, Err(ToolError::ToolNotFound(_))));
    }

    #[tokio::test]
    async fn tool_server_is_available_after_being_upserted() {
        // Given a tool client that only reports tools for a particular url
        let tool = Tool::with_client(ToolClientMock).api();

        // When a tool server is upserted with that particular url
        let namespace = Namespace::new("test").unwrap();
        let server = ConfiguredMcpServer::new("http://localhost:8000/mcp", namespace);
        tool.upsert_tool_server(server).await;

        // Then the calculator tool can be invoked
        let request = InvokeRequest {
            tool_name: "add".to_owned(),
            arguments: vec![],
        };
        let result = tool
            .invoke_tool(request, TracingContext::dummy())
            .await
            .unwrap();
        assert_eq!(result, json!("Success"));
    }

    #[tokio::test]
    async fn tool_server_is_removed() {
        // Given a tool configured with two mcp servers for a namespace
        let namespace = Namespace::new("test").unwrap();
        let servers = vec![
            ConfiguredMcpServer::new("http://localhost:8000/mcp", namespace.clone()),
            ConfiguredMcpServer::new("http://localhost:8001/mcp", namespace.clone()),
        ];
        let tool = Tool::with_client(ToolClientMock)
            .with_servers(servers.clone())
            .await
            .api();

        // When the first mcp server is removed
        tool.remove_tool_server(servers[0].clone()).await;

        // Then the list of tools is only the second mcp server
        let mcp_servers = tool.list_tool_servers(namespace).await;
        assert_eq!(mcp_servers, vec!["http://localhost:8001/mcp".into()]);
    }

    #[tokio::test]
    async fn adding_existing_tool_server_does_not_add_it_again() {
        // Given a tool client that knows about two mcp servers for a namespace
        let tool = Tool::with_client(ToolClientMock).api();
        let namespace = Namespace::new("test").unwrap();
        let server = ConfiguredMcpServer::new("http://localhost:8000/mcp", namespace.clone());
        tool.upsert_tool_server(server.clone()).await;

        // When we add the same tool server again for that namespace
        tool.upsert_tool_server(server).await;

        // Then the list of tools is still the same
        let mcp_servers = tool.list_tool_servers(namespace).await;
        assert_eq!(mcp_servers, vec!["http://localhost:8000/mcp".into()]);
    }

    #[tokio::test]
    async fn same_tool_server_can_be_configured_for_different_namespaces() {
        // Given a tool client
        let tool = Tool::with_client(ToolClientMock).api();
        let first = Namespace::new("first").unwrap();

        // When adding the same tool server for two namespaces
        let server_url = "http://localhost:8000/mcp";
        let server = ConfiguredMcpServer::new(server_url, first.clone());
        tool.upsert_tool_server(server.clone()).await;

        let second = Namespace::new("second").unwrap();
        let server = ConfiguredMcpServer::new(server_url, second.clone());
        tool.upsert_tool_server(server).await;

        // Then the server is configured for both namespaces
        let mcp_servers = tool.list_tool_servers(first).await;
        assert_eq!(mcp_servers, vec![server_url.into()]);

        let mcp_servers = tool.list_tool_servers(second).await;
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

    impl ToolClient for ToolClientSpy {
        async fn list_tools(&self, url: &McpServerUrl) -> Result<Vec<String>, anyhow::Error> {
            let mut queried = self.queried.lock().await;
            queried.insert(url.to_owned());
            Ok(vec![])
        }

        async fn invoke_tool(
            &self,
            _request: InvokeRequest,
            _url: &McpServerUrl,
            _tracing_context: TracingContext,
        ) -> Result<Value, ToolError> {
            unimplemented!()
        }
    }

    #[tokio::test]
    async fn tools_from_multiple_tool_servers_are_available() {
        // Given a tool with two configured tool servers
        let queried = Arc::new(Mutex::new(HashSet::new()));
        let servers = vec![
            ConfiguredMcpServer::new("http://localhost:8000/mcp", Namespace::new("test").unwrap()),
            ConfiguredMcpServer::new("http://localhost:8001/mcp", Namespace::new("test").unwrap()),
        ];
        let tool = Tool::with_client(ToolClientSpy::new(queried.clone()))
            .with_servers(servers.clone())
            .await
            .api();

        // When we ask for the list of tools
        drop(tool.list_tools().await);

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

    impl ToolClient for TwoServerClient {
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
            request: InvokeRequest,
            url: &McpServerUrl,
            _tracing_context: TracingContext,
        ) -> Result<Value, ToolError> {
            match url.0.as_ref() {
                "http://localhost:8000/mcp" => {
                    assert_eq!(request.tool_name, "search");
                    Ok(json!("search result"))
                }
                "http://localhost:8001/mcp" => {
                    assert_eq!(request.tool_name, "calculator");
                    Ok(json!("calculator result"))
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
        let servers = vec![
            ConfiguredMcpServer::new("http://localhost:8000/mcp", Namespace::new("test").unwrap()),
            ConfiguredMcpServer::new("http://localhost:8001/mcp", Namespace::new("test").unwrap()),
        ];
        let tool = Tool::with_client(TwoServerClient)
            .with_servers(servers)
            .await
            .api();

        // When we invoke the search tool
        let result = tool
            .invoke_tool(
                InvokeRequest {
                    tool_name: "search".to_owned(),
                    arguments: vec![],
                },
                TracingContext::dummy(),
            )
            .await;

        // Then we get the result from the brave_search server
        assert_eq!(result.unwrap(), json!("search result"));
    }

    struct ClientOneGoodOtherBad;

    impl ToolClient for ClientOneGoodOtherBad {
        async fn list_tools(&self, url: &McpServerUrl) -> Result<Vec<String>, anyhow::Error> {
            if url.0 == "http://localhost:8000/mcp" {
                Ok(vec!["search".to_owned()])
            } else {
                Err(anyhow::anyhow!("Request to mcp server timed out."))
            }
        }

        async fn invoke_tool(
            &self,
            _request: InvokeRequest,
            _url: &McpServerUrl,
            _tracing_context: TracingContext,
        ) -> Result<Value, ToolError> {
            Ok(json!("Success"))
        }
    }

    impl Tool {
        async fn with_servers(self, servers: Vec<ConfiguredMcpServer>) -> Self {
            let api = self.api();
            for server in servers {
                api.upsert_tool_server(server).await;
            }
            self
        }
    }

    #[tokio::test]
    async fn list_tool_servers_none_configured() {
        // Given a tool client that knows about no mcp servers
        let tool = Tool::with_client(ToolClientMock).api();

        // When listing tool servers for a namespace
        let result = tool
            .list_tool_servers(Namespace::new("test").unwrap())
            .await;

        // Then we get an empty list
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn list_tool_servers() {
        // Given a tool client that knows about two mcp servers for a namespace
        let tool = Tool::with_client(ToolClientMock).api();
        let namespace = Namespace::new("test").unwrap();
        let first = ConfiguredMcpServer::new("http://localhost:8000/mcp", namespace.clone());
        tool.upsert_tool_server(first).await;

        let second = ConfiguredMcpServer::new("http://localhost:8001/mcp", namespace.clone());
        tool.upsert_tool_server(second).await;

        // When listing tool servers for that namespace
        let result: HashSet<McpServerUrl> = tool
            .list_tool_servers(namespace)
            .await
            .into_iter()
            .collect();

        // Then we get the two mcp servers
        let expected = HashSet::from([
            "http://localhost:8000/mcp".into(),
            "http://localhost:8001/mcp".into(),
        ]);
        assert_eq!(result, expected);
    }

    #[tokio::test]
    async fn one_bad_mcp_server_still_allows_to_list_tools() {
        // Given an mcp server that returns an error when listing it's tools
        let servers = vec![
            ConfiguredMcpServer::new("http://localhost:8000/mcp", Namespace::new("test").unwrap()),
            ConfiguredMcpServer::new("http://localhost:8001/mcp", Namespace::new("test").unwrap()),
        ];
        let tool = Tool::with_client(ClientOneGoodOtherBad)
            .with_servers(servers)
            .await;

        // When listing tools
        let result = tool.api().list_tools().await;

        // Then the search tool is available
        assert_eq!(result, vec!["search".to_owned()]);
    }

    #[tokio::test]
    async fn one_bad_mcp_server_still_allows_to_invoke_tool() {
        // Given an mcp server that returns an error when listing it's tools
        let servers = vec![
            ConfiguredMcpServer::new("http://localhost:8000/mcp", Namespace::new("test").unwrap()),
            ConfiguredMcpServer::new("http://localhost:8001/mcp", Namespace::new("test").unwrap()),
        ];
        let tool = Tool::with_client(ClientOneGoodOtherBad)
            .with_servers(servers)
            .await;

        // When invoking a tool
        let request = InvokeRequest {
            tool_name: "search".to_owned(),
            arguments: vec![],
        };
        let result = tool
            .api()
            .invoke_tool(request, TracingContext::dummy())
            .await
            .unwrap();

        // Then the search tool is available
        assert_eq!(result, json!("Success"));
    }

    struct SaboteurClient;

    impl ToolClient for SaboteurClient {
        async fn invoke_tool(
            &self,
            request: InvokeRequest,
            _url: &McpServerUrl,
            _tracing_context: TracingContext,
        ) -> Result<Value, ToolError> {
            match request.tool_name.as_str() {
                "add" => Ok(json!("Success")),
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
        let tool = Tool::with_client(SaboteurClient)
            .with_servers(vec![ConfiguredMcpServer::new(
                "http://localhost:8000/mcp",
                Namespace::new("test").unwrap(),
            )])
            .await;

        let api = tool.api();

        // When one hanging request is in progress
        let request = InvokeRequest {
            tool_name: "divide".to_owned(),
            arguments: vec![],
        };
        let handle = tokio::spawn(async move {
            drop(api.invoke_tool(request, TracingContext::dummy()).await);
        });

        // And waiting shortly for the message to arrive
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Then another request can still be answered
        let request = InvokeRequest {
            tool_name: "add".to_owned(),
            arguments: vec![],
        };
        let result = tool
            .api()
            .invoke_tool(request, TracingContext::dummy())
            .await
            .unwrap();
        assert_eq!(result, json!("Success"));

        drop(handle);
    }
}
