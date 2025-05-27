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

    fn list_tools(&self) -> impl Future<Output = Result<Vec<String>, anyhow::Error>> + Send;
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    // We plan on passing this error variant to the Skills and model.
    #[error("{0}")]
    ToolCallFailed(String),
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

    async fn list_tools(&self) -> Result<Vec<String>, anyhow::Error> {
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
        send: oneshot::Sender<Result<Vec<String>, anyhow::Error>>,
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
        // We always expect to have a calculator MCP server.
        let mcp_address = self.mcp_servers.get("calculator").unwrap();
        self.client
            .invoke_tool(request, mcp_address, tracing_context)
            .await
    }

    async fn list_tools(&self) -> Result<Vec<String>, anyhow::Error> {
        let mut tools = vec![];
        for address in self.mcp_servers.values() {
            tools.extend(self.client.list_tools(address).await?);
        }
        Ok(tools)
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

        async fn list_tools(&self) -> Result<Vec<String>, anyhow::Error> {
            unimplemented!()
        }
    }

    /// Only report tools for one particular server address
    struct ToolClientMock;

    impl ToolClient for ToolClientMock {
        async fn list_tools(&self, mcp_address: &str) -> Result<Vec<String>, anyhow::Error> {
            if mcp_address == "http://localhost:8000/mcp" {
                Ok(vec!["stub_tool".to_owned()])
            } else {
                Ok(vec![])
            }
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
    async fn tool_server_is_upserted() {
        // Given a tool client that only reports tools for a particular url
        let tool = Tool::with_client(ToolClientMock).api();

        // When a tool server is upserted with that particular url
        tool.upsert_tool_server(
            "calculator".to_owned(),
            "http://localhost:8000/mcp".to_owned(),
        )
        .await;

        // Then the tools of that tool server are available
        let tools = tool.list_tools().await.unwrap();
        assert_eq!(tools, vec!["stub_tool".to_owned()]);
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
        drop(tool.list_tools().await.unwrap());

        // Then both tool servers are queried
        let queried = queried.lock().await.clone();
        assert_eq!(queried.len(), 2);
        assert!(queried.contains(&"http://localhost:8000/mcp".to_owned()));
        assert!(queried.contains(&"http://localhost:8001/mcp".to_owned()));
    }
}
