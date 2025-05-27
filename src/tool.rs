use anyhow::anyhow;
use futures::StreamExt;
use reqwest::Client;
use reqwest::Response;
use reqwest::header;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::logging::TracingContext;

pub trait ToolApi {
    fn invoke_tool(
        &self,
        request: InvokeRequest,
        tracing_context: TracingContext,
    ) -> impl Future<Output = Result<Vec<u8>, ToolError>> + Send;

    fn upsert_tool_server(&self, name: String, address: String) -> impl Future<Output = ()> + Send;
}

pub struct Tool {
    handle: JoinHandle<()>,
    send: mpsc::Sender<ToolMsg>,
}

impl Tool {
    pub fn new() -> Self {
        let client = McpClient;
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
}

pub struct Argument {
    pub name: String,
    pub value: Vec<u8>,
}

pub struct InvokeRequest {
    pub tool_name: String,
    pub arguments: Vec<Argument>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ToolCallResponseContent {
    Text { text: String },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolCallResponseResult {
    content: Vec<ToolCallResponseContent>,
    is_error: bool,
}

#[derive(Deserialize)]
struct ToolCallResponse {
    result: ToolCallResponseResult,
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    // We plan on passing this error variant to the Skills and model.
    #[error("{0}")]
    ToolCallFailed(String),
    #[error("The tool call could not be executed, original error: {0}")]
    Other(#[from] anyhow::Error),
}

/// Conditionally parse the response from either an SSE stream or a JSON object.
///
/// If the input contains any number of JSON-RPC requests, the server MUST either return
/// Content-Type: text/event-stream, to initiate an SSE stream, or
/// Content-Type: application/json, to return one JSON object.
/// The client MUST support both these cases.
/// See: <https://modelcontextprotocol.io/specification/2025-03-26/basic/transports#sending-messages-to-the-server>
async fn json_rpc_response_from_http<T>(response: Response) -> anyhow::Result<T>
where
    T: serde::de::DeserializeOwned,
{
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .ok_or_else(|| anyhow!("No content type in response header"))?
        .to_str()?;
    match content_type {
        "application/json" => {
            let data = response.json().await?;
            Ok(serde_json::from_value::<T>(data)?)
        }
        "text/event-stream" => {
            // We may get different type of results in the stream.
            // A client may send different notification types before sending the result we are interested in.
            // For now, we ignore these notification, which can include progress notifications, but also
            // log messages. See <https://modelcontextprotocol.io/specification/2025-03-26/basic#notifications>
            let mut stream = response.bytes_stream();
            while let Some(Ok(item)) = stream.next().await {
                let item = String::from_utf8(item.to_vec())?;
                let data = item
                    .split("data: ")
                    .nth(1)
                    .ok_or_else(|| anyhow!("No data in stream"))?;
                if let Ok(value) = serde_json::from_str::<T>(data) {
                    return Ok(value);
                }
            }
            Err(anyhow!("Expected JSON-RPC response not found in stream"))
        }
        _ => Err(anyhow!("unexpected content type"))?,
    }
}

pub trait ToolClient: Send + Sync + 'static {
    fn invoke_tool(
        &self,
        request: InvokeRequest,
        mcp_address: &str,
        tracing_context: TracingContext,
    ) -> impl Future<Output = Result<Vec<u8>, ToolError>> + Send;
}

pub struct McpClient;

impl ToolClient for McpClient {
    async fn invoke_tool(
        &self,
        request: InvokeRequest,
        mcp_address: &str,
        _tracing_context: TracingContext,
    ) -> Result<Vec<u8>, ToolError> {
        initialize(mcp_address).await?;

        let client = Client::new();
        let arguments = request
            .arguments
            .into_iter()
            .map(|argument| {
                serde_json::from_slice::<Value>(&argument.value).map(|value| (argument.name, value))
            })
            .collect::<Result<HashMap<_, _>, serde_json::Error>>()
            .map_err(anyhow::Error::from)?;

        let body = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": request.tool_name,
            "arguments": arguments
        }
        });
        let response = client
            .post(mcp_address)
            // MCP server want exactly these two headers, even a wildcard is not accepted
            .header("accept", "application/json,text/event-stream")
            .json(&body)
            .send()
            .await
            .map_err(anyhow::Error::from)?;

        let tool_response = json_rpc_response_from_http::<ToolCallResponse>(response).await?;
        match tool_response.result {
            ToolCallResponseResult {
                content,
                is_error: false,
            } => {
                let ToolCallResponseContent::Text { text } = content
                    .first()
                    .ok_or(anyhow!("No content in tool call response"))?;
                Ok(text.to_owned().into_bytes())
            }
            // We might want to represent a failed tool call in the wit world and pass it to the model.
            // this would mean not returning an `Err` case for this, but rather a variant of `Ok`.
            ToolCallResponseResult {
                content,
                is_error: true,
            } => {
                // Even for errors messages, we expect a text response for each tool call. So if there is no
                // text, the error is not a tool call failed, but rather a bad response by the MCP server.
                let ToolCallResponseContent::Text { text } = content
                    .first()
                    .ok_or(anyhow!("No content in tool call response"))?;
                Err(ToolError::ToolCallFailed(text.to_owned()))
            }
        }
    }
}

/// The initialization phase MUST be the first interaction between client and server.
/// During this phase, the client and server:
/// - Establish protocol version compatibility
/// - Exchange and negotiate capabilities
/// - Share implementation details
///
/// See: <https://modelcontextprotocol.io/specification/2025-03-26/basic/lifecycle#initialization>
pub async fn initialize(mcp_address: &str) -> Result<(), ToolError> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct InitializeResult {
        protocol_version: String,
    }

    #[derive(Deserialize)]
    struct InitializeResponse {
        result: InitializeResult,
    }

    // In the initialize request, the client MUST send a protocol version it supports.
    // This SHOULD be the latest version supported by the client.
    // See: <https://modelcontextprotocol.io/specification/2025-03-26/basic/lifecycle#version-negotiation>
    const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-03-26"];
    let body = json!({
        "jsonrpc": "2.0",
        "method": "initialize",
        "id": 1,
        "params": {
            "protocolVersion": SUPPORTED_PROTOCOL_VERSIONS[0],
            "capabilities": {},
            "clientInfo": {
                "name": "PhariaKernel",
                "version": env!("CARGO_PKG_VERSION")
            }
        }
    });

    let client = Client::new();
    let response = client
        .post(mcp_address)
        .header("accept", "application/json,text/event-stream")
        .json(&body)
        .send()
        .await
        .map_err(Into::<anyhow::Error>::into)?;

    let response = json_rpc_response_from_http::<InitializeResponse>(response).await?;

    // If the server supports the requested protocol version, it MUST respond with the same version.
    // Otherwise, the server MUST respond with another protocol version it supports.
    // This SHOULD be the latest version supported by the server.
    // If the client does not support the version in the server's response, it SHOULD disconnect.
    if !SUPPORTED_PROTOCOL_VERSIONS.contains(&response.result.protocol_version.as_str()) {
        return Err(anyhow!(
            "The proposed protocol version {} from the MCP server is not supported.",
            response.result.protocol_version,
        ))?;
    }

    // After successful initialization, the client MUST send an initialized notification to
    // indicate it is ready to begin normal operations:
    let body = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
    });

    let response = client
        .post(mcp_address)
        .header("accept", "application/json,text/event-stream")
        .json(&body)
        .send()
        .await
        .map_err(anyhow::Error::from)?;

    if !response.status().is_success() {
        return Err(ToolError::Other(anyhow!(
            "Failed to send initialized notification"
        )));
    }

    Ok(())
}

#[cfg(test)]
pub mod tests {
    use test_skills::{given_json_mcp_server, given_sse_mcp_server};

    use super::*;

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
    }

    #[tokio::test]
    async fn tool_server_is_upserted() {
        // Given a tool
        let tool = Tool::new().api();

        // When a tool server is upserted
        tool.upsert_tool_server(
            "calculator".to_owned(),
            "http://localhost:8000/mcp".to_owned(),
        )
        .await;

        // Then the tool is available, we can not test this nicely yet
    }

    #[tokio::test]
    async fn invoke_tool_given_mcp_server() {
        let mcp = given_sse_mcp_server().await;

        let request = InvokeRequest {
            tool_name: "add".to_owned(),
            arguments: vec![
                Argument {
                    name: "a".to_owned(),
                    value: json!(1).to_string().into_bytes(),
                },
                Argument {
                    name: "b".to_owned(),
                    value: json!(2).to_string().into_bytes(),
                },
            ],
        };
        let client = McpClient;
        let response = client
            .invoke_tool(request, mcp.address(), TracingContext::dummy())
            .await
            .unwrap();
        let response = String::from_utf8(response).unwrap();
        assert_eq!(response, "3");
    }

    #[tokio::test]
    async fn invoke_tool_against_json_mcp_server() {
        let mcp = given_json_mcp_server().await;

        let request = InvokeRequest {
            tool_name: "add".to_owned(),
            arguments: vec![
                Argument {
                    name: "a".to_owned(),
                    value: json!(1).to_string().into_bytes(),
                },
                Argument {
                    name: "b".to_owned(),
                    value: json!(2).to_string().into_bytes(),
                },
            ],
        };
        let client = McpClient;
        let response = client
            .invoke_tool(request, mcp.address(), TracingContext::dummy())
            .await
            .unwrap();
        let response = String::from_utf8(response).unwrap();
        assert_eq!(response, "3");
    }

    #[tokio::test]
    async fn invoke_unknown_tool_gives_error() {
        let mcp = given_sse_mcp_server().await;

        let request = InvokeRequest {
            tool_name: "unknown".to_owned(),
            arguments: vec![],
        };
        let client = McpClient;
        let response = client
            .invoke_tool(request, mcp.address(), TracingContext::dummy())
            .await
            .unwrap_err();
        assert!(matches!(response, ToolError::ToolCallFailed(_)));
    }

    #[tokio::test]
    async fn invoke_saboteur_tool_results_in_error() {
        let mcp = given_sse_mcp_server().await;

        let request = InvokeRequest {
            tool_name: "saboteur".to_owned(),
            arguments: vec![],
        };
        let client = McpClient;
        let response = client
            .invoke_tool(request, mcp.address(), TracingContext::dummy())
            .await
            .unwrap_err();
        assert_eq!(
            response.to_string(),
            "Error executing tool saboteur: Out of cheese."
        );
    }

    #[tokio::test]
    async fn initialize_request() {
        let mcp = given_sse_mcp_server().await;

        initialize(mcp.address()).await.unwrap();
    }
}
