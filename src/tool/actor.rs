use futures::StreamExt;
use futures::future::join_all;
use futures::stream::FuturesUnordered;
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

use super::client::McpClient;

pub trait ToolApi {
    fn invoke_tool(
        &self,
        request: InvokeRequest,
        tracing_context: TracingContext,
    ) -> impl Future<Output = Result<Value, ToolError>> + Send;

    fn upsert_tool_server(&self, name: String, address: String) -> impl Future<Output = ()> + Send;

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

    pub fn api(&self) -> impl ToolApi + Send + Sync + Clone + 'static {
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

    async fn upsert_tool_server(&self, name: String, address: String) {
        let msg = ToolMsg::UpsertToolServer { name, address };
        self.send(msg).await.unwrap();
    }

    async fn list_tools(&self) -> Vec<String> {
        let (send, receive) = oneshot::channel();
        let msg = ToolMsg::ListTools { send };
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
        name: String,
        address: String,
    },
    ListTools {
        send: oneshot::Sender<Vec<String>>,
    },
}

struct ToolActor<T: ToolClient> {
    mcp_servers: HashMap<String, String>,
    mcp_servers_set: HashSet<String>,
    receiver: mpsc::Receiver<ToolMsg>,
    running_requests: FuturesUnordered<Pin<Box<dyn Future<Output = ()> + Send>>>,
    client: Arc<T>,
}

impl<T: ToolClient> ToolActor<T> {
    fn new(receiver: mpsc::Receiver<ToolMsg>, client: T) -> Self {
        Self {
            mcp_servers: HashMap::new(),
            mcp_servers_set: HashSet::new(),
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

    fn act(&mut self, msg: ToolMsg) {
        match msg {
            ToolMsg::InvokeTool {
                request,
                tracing_context,
                send,
            } => {
                let client = self.client.clone();
                let servers = self.mcp_servers_set.clone().into_iter().collect();
                self.running_requests.push(Box::pin(async move {
                    let result =
                        Self::invoke_tool(client.as_ref(), servers, request, tracing_context).await;
                    drop(send.send(result));
                }));
            }
            ToolMsg::ListTools { send } => {
                let client = self.client.clone();
                let servers = self.mcp_servers_set.clone().into_iter().collect();
                self.running_requests.push(Box::pin(async move {
                    let result = Self::tools(client.as_ref(), servers).await;
                    let result = result.into_values().flatten().collect();
                    drop(send.send(result));
                }));
            }
            ToolMsg::UpsertToolServer { name: _, address } => {
                self.mcp_servers_set.insert(address);
            }
        }
    }

    /// Invoke a tool and return the result.
    ///
    /// First, we need to locate the server on which the tool is hosted.
    /// Then, we invoke the tool on that server.
    async fn invoke_tool(
        client: &impl ToolClient,
        servers: Vec<String>,
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
    async fn tools(client: &impl ToolClient, servers: Vec<String>) -> HashMap<String, Vec<String>> {
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
        servers: Vec<String>,
        tool: &str,
    ) -> Option<String> {
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
        mcp_address: &str,
        tracing_context: TracingContext,
    ) -> impl Future<Output = Result<Value, ToolError>> + Send;

    fn list_tools(
        &self,
        mcp_address: &str,
    ) -> impl Future<Output = Result<Vec<String>, anyhow::Error>> + Send;
}

#[cfg(test)]
pub mod tests {
    use core::panic;
    use std::{sync::Arc, time::Duration};

    use futures::future::pending;
    use serde_json::{Value, json};
    use tokio::sync::Mutex;

    use crate::{logging::TracingContext, tool::Tool};

    use super::{InvokeRequest, ToolApi, ToolClient, ToolError};

    pub struct ToolDouble;

    impl ToolApi for ToolDouble {
        async fn invoke_tool(
            &self,
            _request: InvokeRequest,
            _tracing_context: TracingContext,
        ) -> Result<Value, ToolError> {
            unimplemented!()
        }

        async fn upsert_tool_server(&self, _name: String, _address: String) {}

        async fn list_tools(&self) -> Vec<String> {
            unimplemented!()
        }
    }

    /// Only report tools for one particular server address
    struct ToolClientMock;

    impl ToolClient for ToolClientMock {
        async fn list_tools(&self, mcp_address: &str) -> Result<Vec<String>, anyhow::Error> {
            if mcp_address == "http://localhost:8000/mcp" {
                Ok(vec!["add".to_owned()])
            } else {
                panic!("This client only knows the localhost:8000 mcp server")
            }
        }

        async fn invoke_tool(
            &self,
            _request: InvokeRequest,
            mcp_address: &str,
            _tracing_context: TracingContext,
        ) -> Result<Value, ToolError> {
            if mcp_address == "http://localhost:8000/mcp" {
                Ok(json!("Success"))
            } else {
                panic!("This client only knows the localhost:8000 mcp server")
            }
        }
    }

    #[tokio::test]
    async fn invoking_unknown_without_servers_gives_tool_not_found() {
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
        tool.upsert_tool_server(
            "calculator".to_owned(),
            "http://localhost:8000/mcp".to_owned(),
        )
        .await;

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

    struct ToolClientSpy {
        queried: Arc<Mutex<Vec<String>>>,
    }

    impl ToolClientSpy {
        fn new(queried: Arc<Mutex<Vec<String>>>) -> Self {
            Self { queried }
        }
    }

    impl ToolClient for ToolClientSpy {
        async fn list_tools(&self, mcp_address: &str) -> Result<Vec<String>, anyhow::Error> {
            let mut queried = self.queried.lock().await;
            queried.push(mcp_address.to_owned());
            Ok(vec![])
        }

        async fn invoke_tool(
            &self,
            _request: InvokeRequest,
            _mcp_address: &str,
            _tracing_context: TracingContext,
        ) -> Result<Value, ToolError> {
            unimplemented!()
        }
    }

    #[tokio::test]
    async fn tools_from_multiple_tool_servers_are_available() {
        // Given a tool with two configured tool servers
        let queried = Arc::new(Mutex::new(vec![]));
        let tool = Tool::with_client(ToolClientSpy::new(queried.clone())).api();

        tool.upsert_tool_server(
            "calculator".to_owned(),
            "http://localhost:8000/mcp".to_owned(),
        )
        .await;

        tool.upsert_tool_server(
            "brave_search".to_owned(),
            "http://localhost:8001/mcp".to_owned(),
        )
        .await;

        // When we ask for the list of tools
        drop(tool.list_tools().await);

        // Then both tool servers are queried
        let queried = queried.lock().await.clone();
        assert_eq!(queried.len(), 2);
        assert!(queried.contains(&"http://localhost:8000/mcp".to_owned()));
        assert!(queried.contains(&"http://localhost:8001/mcp".to_owned()));
    }

    // Given a tool client that knows about two mcp servers
    struct TwoServerClient;

    impl ToolClient for TwoServerClient {
        async fn list_tools(&self, mcp_address: &str) -> Result<Vec<String>, anyhow::Error> {
            match mcp_address {
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
            mcp_address: &str,
            _tracing_context: TracingContext,
        ) -> Result<Value, ToolError> {
            match mcp_address {
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
        let tool = Tool::with_client(TwoServerClient).api();

        // And given that the server is configured with these two servers
        tool.upsert_tool_server(
            "brave_search".to_owned(),
            "http://localhost:8000/mcp".to_owned(),
        )
        .await;

        tool.upsert_tool_server(
            "calculator".to_owned(),
            "http://localhost:8001/mcp".to_owned(),
        )
        .await;

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
        async fn list_tools(&self, mcp_address: &str) -> Result<Vec<String>, anyhow::Error> {
            if mcp_address == "http://localhost:8000/mcp" {
                Ok(vec!["search".to_owned()])
            } else {
                Err(anyhow::anyhow!("Request to mcp server timed out."))
            }
        }

        async fn invoke_tool(
            &self,
            _request: InvokeRequest,
            _mcp_address: &str,
            _tracing_context: TracingContext,
        ) -> Result<Value, ToolError> {
            Ok(json!("Success"))
        }
    }

    impl Tool {
        async fn with_servers(self, servers: Vec<(&str, &str)>) -> Self {
            let api = self.api();
            for (name, address) in servers {
                api.upsert_tool_server(name.to_owned(), address.to_owned())
                    .await;
            }
            self
        }
    }

    #[tokio::test]
    async fn one_bad_mcp_server_still_allows_to_list_tools() {
        // Given an mcp server that returns an error when listing it's tools
        let tool = Tool::with_client(ClientOneGoodOtherBad)
            .with_servers(vec![
                ("search", "http://localhost:8000/mcp"),
                ("calculator", "http://localhost:8001/mcp"),
            ])
            .await;

        // When listing tools
        let result = tool.api().list_tools().await;

        // Then the search tool is available
        assert_eq!(result, vec!["search".to_owned()]);
    }

    #[tokio::test]
    async fn one_bad_mcp_server_still_allows_to_invoke_tool() {
        // Given an mcp server that returns an error when listing it's tools
        let tool = Tool::with_client(ClientOneGoodOtherBad)
            .with_servers(vec![
                ("search", "http://localhost:8000/mcp"),
                ("calculator", "http://localhost:8001/mcp"),
            ])
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
            _mcp_address: &str,
            _tracing_context: TracingContext,
        ) -> Result<Value, ToolError> {
            match request.tool_name.as_str() {
                "add" => Ok(json!("Success")),
                "divide" => pending().await,
                _ => panic!("unknown function called"),
            }
        }

        async fn list_tools(&self, _mcp_address: &str) -> Result<Vec<String>, anyhow::Error> {
            Ok(vec!["add".to_owned(), "divide".to_owned()])
        }
    }

    #[tokio::test]
    async fn tool_calls_are_processed_in_parallel() {
        // Given a tool that hangs forever for some tool invocation requestsa
        let tool = Tool::with_client(SaboteurClient)
            .with_servers(vec![("calculator", "http://localhost:8000/mcp")])
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
