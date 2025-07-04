use anyhow::anyhow;
use futures::StreamExt;
use reqwest::{Client, Response, header};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use tracing::{error, info};

use crate::{
    context,
    logging::TracingContext,
    mcp::McpServerUrl,
    tool::{Argument, ToolDescription, ToolError, ToolOutput},
};

#[cfg(test)]
use double_trait::double;

pub struct McpClientImpl {
    client: Client,
}

impl McpClientImpl {
    pub fn new() -> Self {
        let client = Client::new();
        Self { client }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolCallResult {
    content: ToolOutput,
    is_error: bool,
}

impl McpClientImpl {
    fn parse_tool_call_result(result: ToolCallResult) -> Result<ToolOutput, ToolError> {
        if result.is_error {
            Err(ToolError::ToolExecution(result.content.text()))
        } else {
            Ok(result.content)
        }
    }
}

/// A client used by the MCP actor to interact with the MCP servers.
#[cfg_attr(test, double(McpClientDouble))]
pub trait McpClient: Send + Sync + 'static {
    fn invoke_tool(
        &self,
        name: &str,
        arguments: Vec<Argument>,
        url: &McpServerUrl,
        tracing_context: TracingContext,
    ) -> impl Future<Output = Result<ToolOutput, ToolError>> + Send;

    fn list_tools(
        &self,
        url: &McpServerUrl,
    ) -> impl Future<Output = Result<Vec<ToolDescription>, anyhow::Error>> + Send + Sync;
}

impl McpClient for McpClientImpl {
    async fn invoke_tool(
        &self,
        name: &str,
        arguments: Vec<Argument>,
        url: &McpServerUrl,
        tracing_context: TracingContext,
    ) -> Result<ToolOutput, ToolError> {
        let child_context = context!(tracing_context, "pharia-kernel::tool", "initialize");
        self.initialize(url)
            .await
            .inspect_err(|e| error!(parent: child_context.span(), "{}", e))?;
        drop(child_context);

        let arguments = arguments
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
            "name": name,
            "arguments": arguments
        }
        });
        let response = self
            .client
            .post(&url.0)
            // MCP server want exactly these two headers, even a wildcard is not accepted
            .header("accept", "application/json,text/event-stream")
            .json(&body)
            .send()
            .await
            .map_err(anyhow::Error::from)
            .inspect_err(|e| error!(parent: tracing_context.span(), "{}", e))?;

        let result = Self::json_rpc_result_from_http::<ToolCallResult>(response).await?;
        let result = Self::parse_tool_call_result(result);
        if result.is_ok() {
            info!(parent: tracing_context.span(), "Tool call succeeded");
        } else {
            info!(parent: tracing_context.span(), "Tool call failed");
        }
        result
    }

    async fn list_tools(&self, url: &McpServerUrl) -> Result<Vec<ToolDescription>, anyhow::Error> {
        // See <https://modelcontextprotocol.io/specification/2025-06-18/server/tools#tool>
        // for the defintion of the ToolDescription struct.
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ToolDescriptionDeserializer {
            name: String,
            description: String,
            input_schema: Value,
        }

        #[derive(Deserialize)]
        struct ListToolsResult {
            tools: Vec<ToolDescriptionDeserializer>,
        }

        self.initialize(url).await?;

        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
        });
        let response = self
            .client
            .post(&url.0)
            .header("accept", "application/json,text/event-stream")
            .json(&body)
            .send()
            .await
            .map_err(anyhow::Error::from)?;

        let result = Self::json_rpc_result_from_http::<ListToolsResult>(response).await?;
        Ok(result
            .tools
            .into_iter()
            .map(|tool| ToolDescription::new(tool.name, tool.description, tool.input_schema))
            .collect())
    }
}

impl McpClientImpl {
    /// The initialization phase MUST be the first interaction between client and server.
    /// During this phase, the client and server:
    /// - Establish protocol version compatibility
    /// - Exchange and negotiate capabilities
    /// - Share implementation details
    ///
    /// See: <https://modelcontextprotocol.io/specification/2025-03-26/basic/lifecycle#initialization>
    pub async fn initialize(&self, url: &McpServerUrl) -> Result<(), ToolError> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct InitializeResult {
            protocol_version: String,
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

        let response = self
            .client
            .post(&url.0)
            .header("accept", "application/json,text/event-stream")
            .json(&body)
            .send()
            .await
            .map_err(Into::<anyhow::Error>::into)?;

        let result = Self::json_rpc_result_from_http::<InitializeResult>(response).await?;

        // If the server supports the requested protocol version, it MUST respond with the same version.
        // Otherwise, the server MUST respond with another protocol version it supports.
        // This SHOULD be the latest version supported by the server.
        // If the client does not support the version in the server's response, it SHOULD disconnect.
        if !SUPPORTED_PROTOCOL_VERSIONS.contains(&result.protocol_version.as_str()) {
            return Err(anyhow!(
                "The proposed protocol version {} from the MCP server is not supported.",
                result.protocol_version,
            ))?;
        }

        // After successful initialization, the client MUST send an initialized notification to
        // indicate it is ready to begin normal operations:
        let body = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
        });

        let response = self
            .client
            .post(&url.0)
            .header("accept", "application/json,text/event-stream")
            .json(&body)
            .send()
            .await
            .map_err(anyhow::Error::from)?;

        if !response.status().is_success() {
            return Err(ToolError::RuntimeError(anyhow!(
                "Failed to send initialized notification"
            )));
        }

        Ok(())
    }

    /// Conditionally parse the response from either an SSE stream or a JSON object.
    ///
    /// If the input contains any number of JSON-RPC requests, the server MUST either return
    /// Content-Type: text/event-stream, to initiate an SSE stream, or
    /// Content-Type: application/json, to return one JSON object.
    /// The client MUST support both these cases.
    /// See: <https://modelcontextprotocol.io/specification/2025-03-26/basic/transports#sending-messages-to-the-server>
    async fn json_rpc_result_from_http<T>(response: Response) -> anyhow::Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        #[derive(Deserialize)]
        struct JsonRpcResponse<T> {
            result: T,
        }

        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .ok_or_else(|| anyhow!("No content type in response header"))?
            .to_str()?;
        match content_type {
            "application/json" => {
                let data = response.json().await?;
                Ok(serde_json::from_value::<JsonRpcResponse<T>>(data)?.result)
            }
            "text/event-stream" => {
                // Collect bytes from the SSE stream until we have a complete event containing
                // the JSON-RPC response we are interested in.
                let stream = response.bytes_stream();
                let value: JsonRpcResponse<T> =
                    Self::json_rpc_result_from_sse_stream(stream).await?;
                Ok(value.result)
            }
            _ => Err(anyhow!("unexpected content type"))?,
        }
    }

    /// Parse an SSE `bytes_stream` and find the first occurence of <T> contained in a `data:`
    /// field. SSE events can span multiple chunks, and multiple events can be in a single chunk.
    async fn json_rpc_result_from_sse_stream<T, S>(mut stream: S) -> anyhow::Result<T>
    where
        T: serde::de::DeserializeOwned,
        S: futures::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Unpin,
    {
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let item = String::from_utf8_lossy(&chunk?).replace("\r\n", "\n");
            buffer.push_str(&item);

            // There might be multiple events in a single chunk.
            while let Some(pos) = buffer.find("\n\n") {
                let event = &buffer[..pos];

                for line in event.lines() {
                    if let Some(data) = line.strip_prefix("data: ") {
                        if let Ok(value) = serde_json::from_str::<T>(data) {
                            return Ok(value);
                        }
                    }
                }

                // Remove the processed event from the buffer
                buffer = buffer[pos + 2..].to_string();
            }
        }

        Err(anyhow!("Expected JSON-RPC response not found in stream"))
    }
}

#[cfg(test)]
pub mod tests {
    use bytes::Bytes;
    use futures::stream;
    use test_skills::{given_json_mcp_server, given_sse_mcp_server};

    use crate::tool::Argument;

    use super::*;

    #[tokio::test]
    async fn tools_can_be_listed() {
        // Given a MCP server
        let mcp = given_sse_mcp_server().await;
        let client = McpClientImpl::new();

        // When listing tools
        let tools = client.list_tools(&mcp.address().into()).await.unwrap();
        let names = tools.iter().map(ToolDescription::name).collect::<Vec<_>>();

        // Then the add tool is listed
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"add"));
        assert!(names.contains(&"saboteur"));
    }

    #[tokio::test]
    async fn tool_description_is_returned() {
        // Given a MCP server
        let mcp = given_sse_mcp_server().await;
        let client = McpClientImpl::new();

        // When listing tools
        let tools = client.list_tools(&mcp.address().into()).await.unwrap();

        let add = tools.iter().find(|t| t.name() == "add").unwrap();
        assert_eq!(add.description(), "Add two numbers");
    }

    #[tokio::test]
    async fn tool_input_schema_is_returned() {
        // Given a MCP server
        let mcp = given_sse_mcp_server().await;
        let client = McpClientImpl::new();

        // When listing tools
        let tools = client.list_tools(&mcp.address().into()).await.unwrap();

        let add = tools.iter().find(|t| t.name() == "add").unwrap();
        let input_schema = add.input_schema();
        assert_eq!(
            input_schema,
            &json!({
                "properties":  {
                    "a":  {
                        "title": "A",
                        "type": "integer"
                    },
                    "b": {
                        "title": "B",
                        "type": "integer"
                    }
                },
                "required":  ["a", "b"],
                "title": "addArguments",
                "type": "object"
            })
        );
    }

    #[tokio::test]
    async fn invoke_tool_given_mcp_server() {
        let mcp = given_sse_mcp_server().await;

        let arguments = vec![
            Argument {
                name: "a".to_owned(),
                value: json!(1).to_string().into_bytes(),
            },
            Argument {
                name: "b".to_owned(),
                value: json!(2).to_string().into_bytes(),
            },
        ];
        let client = McpClientImpl::new();
        let response = client
            .invoke_tool(
                "add",
                arguments,
                &mcp.address().into(),
                TracingContext::dummy(),
            )
            .await
            .unwrap();

        assert_eq!(response.text(), "3");
    }

    #[tokio::test]
    async fn invoke_tool_against_json_mcp_server() {
        let mcp = given_json_mcp_server().await;

        let arguments = vec![
            Argument {
                name: "a".to_owned(),
                value: json!(1).to_string().into_bytes(),
            },
            Argument {
                name: "b".to_owned(),
                value: json!(2).to_string().into_bytes(),
            },
        ];
        let client = McpClientImpl::new();
        let response = client
            .invoke_tool(
                "add",
                arguments,
                &mcp.address().into(),
                TracingContext::dummy(),
            )
            .await
            .unwrap();

        assert_eq!(response.text(), "3");
    }

    #[tokio::test]
    async fn invoke_unknown_tool_gives_error() {
        let mcp = given_sse_mcp_server().await;

        let client = McpClientImpl::new();
        let response = client
            .invoke_tool(
                "unknown",
                vec![],
                &mcp.address().into(),
                TracingContext::dummy(),
            )
            .await
            .unwrap_err();
        assert!(matches!(response, ToolError::ToolExecution(_)));
    }

    #[tokio::test]
    async fn invoke_saboteur_tool_results_in_error() {
        let mcp = given_sse_mcp_server().await;

        let client = McpClientImpl::new();
        let response = client
            .invoke_tool(
                "saboteur",
                vec![],
                &mcp.address().into(),
                TracingContext::dummy(),
            )
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

        let client = McpClientImpl::new();
        client.initialize(&mcp.address().into()).await.unwrap();
    }

    #[test]
    fn parse_tool_call_result_with_empty_list() {
        let result = ToolCallResult {
            content: ToolOutput::empty(),
            is_error: false,
        };

        let result = McpClientImpl::parse_tool_call_result(result);

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn json_rpc_response_can_span_multiple_sse_chunks() {
        // Given a SSE event split across two chunks
        let chunks = vec!["event: message\ndata: 4", "2\n\n"];
        let stream = stream::iter(
            chunks
                .into_iter()
                .map(|chunk| Ok::<_, reqwest::Error>(Bytes::from(chunk))),
        );

        // Then we get the parsed result
        let parsed: i32 = McpClientImpl::json_rpc_result_from_sse_stream(stream)
            .await
            .unwrap();

        assert_eq!(parsed, 42);
    }

    #[tokio::test]
    async fn json_rpc_response_with_crlf_line_endings() {
        // Given a SSE event that contains CRLF line endings
        let sse_event = "event: message\r\ndata: 123\r\n\r\n";

        // When the stream is parsed
        let test_stream = stream::iter(vec![Ok::<_, reqwest::Error>(Bytes::from(sse_event))]);

        // Then we get the parsed result
        let parsed: i32 = McpClientImpl::json_rpc_result_from_sse_stream(test_stream)
            .await
            .unwrap();

        assert_eq!(parsed, 123);
    }

    #[tokio::test]
    async fn skips_uninteresting_events() {
        // Given a SSE stream that contains one uninteresting event and one interesting event
        let chunks = vec![
            "event: message\ndata: northern-pike\n\n",
            "event: message\ndata: 42\n\n",
        ];
        let stream = stream::iter(
            chunks
                .into_iter()
                .map(|chunk| Ok::<_, reqwest::Error>(Bytes::from(chunk))),
        );

        // When the stream is parsed
        let parsed: i32 = McpClientImpl::json_rpc_result_from_sse_stream(stream)
            .await
            .unwrap();

        // Then we get the interesting event
        assert_eq!(parsed, 42);
    }

    #[tokio::test]
    async fn multiple_events_in_single_chunk() {
        // Given a SSE stream that contains an uninteresting event and an interesting event in the
        // same chunk
        let chunks = vec!["event: message\ndata: northern-pike\n\nevent: message\ndata: 42\n\n"];
        let stream = stream::iter(
            chunks
                .into_iter()
                .map(|chunk| Ok::<_, reqwest::Error>(Bytes::from(chunk))),
        );

        // When the stream is parsed
        let parsed: i32 = McpClientImpl::json_rpc_result_from_sse_stream(stream)
            .await
            .unwrap();

        // Then we get the interesting event
        assert_eq!(parsed, 42);
    }
}
