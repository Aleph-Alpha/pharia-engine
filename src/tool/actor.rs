use std::collections::HashMap;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::logging::TracingContext;

use super::client::McpClient;

pub trait ToolApi {
    fn invoke_tool(
        &self,
        request: InvokeRequest,
        tracing_context: TracingContext,
    ) -> impl Future<Output = Result<Vec<u8>, ToolError>> + Send;

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
    ) -> Result<Vec<u8>, ToolError> {
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
        send: oneshot::Sender<Result<Vec<u8>, ToolError>>,
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
    receiver: mpsc::Receiver<ToolMsg>,
    client: T,
}

impl<T: ToolClient> ToolActor<T> {
    fn new(receiver: mpsc::Receiver<ToolMsg>, client: T) -> Self {
        Self {
            mcp_servers: HashMap::new(),
            receiver,
            client,
        }
    }

    async fn run(&mut self) {
        while let Some(msg) = self.receiver.recv().await {
            self.act(msg).await;
        }
    }

    async fn act(&mut self, msg: ToolMsg) {
        match msg {
            ToolMsg::InvokeTool {
                request,
                tracing_context,
                send: response,
            } => {
                let result = self.invoke_tool(request, tracing_context).await;
                drop(response.send(result));
            }
            ToolMsg::UpsertToolServer { name, address } => {
                self.mcp_servers.insert(name, address);
            }
            ToolMsg::ListTools { send } => {
                let result = self.list_tools().await;
                drop(send.send(result));
            }
        }
    }

    async fn invoke_tool(
        &self,
        request: InvokeRequest,
        tracing_context: TracingContext,
    ) -> Result<Vec<u8>, ToolError> {
        let mcp_address = self
            .server_for_tool(&request.tool_name)
            .await
            .ok_or(ToolError::ToolNotFound(request.tool_name.clone()))?;
        self.client
            .invoke_tool(request, mcp_address, tracing_context)
            .await
    }

    /// Returns all tools found on any of the configured tool servers.
    ///
    /// We do not return a result here, but ignore MCP servers that are giving errors.
    /// A skill only is interested in a subset of tools. At some layer, we need to trade in
    /// tool names for the json schema. There would be the appropriate place to decide if the
    /// available information is enough.
    async fn list_tools(&self) -> Vec<String> {
        let mut all_tools = vec![];
        for address in self.mcp_servers.values() {
            if let Ok(tools) = self.client.list_tools(address).await {
                all_tools.extend(tools);
            }
        }
        all_tools
    }

    /// Which server hosts this tool?
    ///
    /// We do not return a result here, but ignore MCP servers that are giving errors.
    /// We do not want to allow one bad server to prevent a skill which does not rely on that
    /// particular server from being invoked successfully.
    async fn server_for_tool(&self, tool: &str) -> Option<&str> {
        for address in self.mcp_servers.values() {
            if let Ok(tools) = self.client.list_tools(address).await {
                if tools.contains(&tool.to_owned()) {
                    return Some(address);
                }
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
    ) -> impl Future<Output = Result<Vec<u8>, ToolError>> + Send;

    fn list_tools(
        &self,
        mcp_address: &str,
    ) -> impl Future<Output = Result<Vec<String>, anyhow::Error>> + Send;
}

#[cfg(test)]
pub mod tests {
    use std::sync::Arc;

    use serde_json::json;
    use tokio::sync::Mutex;

    use crate::{logging::TracingContext, tool::Tool};

    use super::{InvokeRequest, ToolApi, ToolClient, ToolError};

    pub struct ToolDouble;

    impl ToolApi for ToolDouble {
        async fn invoke_tool(
            &self,
            _request: InvokeRequest,
            _tracing_context: TracingContext,
        ) -> Result<Vec<u8>, ToolError> {
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
        ) -> Result<Vec<u8>, ToolError> {
            if mcp_address == "http://localhost:8000/mcp" {
                Ok(json!("Success").to_string().into_bytes())
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
        assert_eq!(result, json!("Success").to_string().into_bytes());
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
        ) -> Result<Vec<u8>, ToolError> {
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
        ) -> Result<Vec<u8>, ToolError> {
            match mcp_address {
                "http://localhost:8000/mcp" => {
                    assert_eq!(request.tool_name, "search");
                    Ok(json!("search result").to_string().as_bytes().to_vec())
                }
                "http://localhost:8001/mcp" => {
                    assert_eq!(request.tool_name, "calculator");
                    Ok(json!("calculator result").to_string().as_bytes().to_vec())
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
        assert_eq!(
            result.unwrap(),
            json!("search result").to_string().as_bytes().to_vec()
        );
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
        ) -> Result<Vec<u8>, ToolError> {
            Ok(json!("Success").to_string().into_bytes())
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
        assert_eq!(result, json!("Success").to_string().into_bytes());
    }
}
