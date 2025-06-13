use anyhow::anyhow;
use futures::StreamExt;
use reqwest::Response;
use reqwest::header;
use serde::Deserialize;
use std::collections::HashMap;
use tracing::error;
use tracing::info;

use crate::context;
use crate::logging::TracingContext;
use crate::tool::{Argument, Modality, ToolOutput};

use reqwest::Client;
use serde_json::{Value, json};

use super::toolbox::McpServerUrl;
use super::{ToolError, actor::ToolClient};

pub struct McpClient {
    client: Client,
}

impl McpClient {
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

impl McpClient {
    fn parse_tool_call_result(result: ToolCallResult) -> Result<ToolOutput, ToolError> {
        if result.is_error {
            let Modality::Text { text } = result
                .content
                .first()
                .ok_or(anyhow!("No content in tool call response"))?;
            Err(ToolError::ToolCallFailed(text.to_owned()))
        } else {
            Ok(result.content)
        }
    }
}

impl ToolClient for McpClient {
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

    async fn list_tools(&self, url: &McpServerUrl) -> Result<Vec<String>, anyhow::Error> {
        #[derive(Deserialize)]
        struct ToolDescription {
            // there is a lot more fields here, but we need to start somewhere
            name: String,
        }

        #[derive(Deserialize)]
        struct ListToolsResult {
            tools: Vec<ToolDescription>,
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
        Ok(result.tools.into_iter().map(|tool| tool.name).collect())
    }
}

impl McpClient {
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
            return Err(ToolError::Other(anyhow!(
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
                    if let Ok(value) = serde_json::from_str::<JsonRpcResponse<T>>(data) {
                        return Ok(value.result);
                    }
                }
                Err(anyhow!("Expected JSON-RPC response not found in stream"))
            }
            _ => Err(anyhow!("unexpected content type"))?,
        }
    }
}
#[cfg(test)]
pub mod tests {
    use test_skills::{given_json_mcp_server, given_sse_mcp_server};

    use crate::tool::Argument;

    use super::*;

    #[tokio::test]
    async fn tools_can_be_listed() {
        // Given a MCP server
        let mcp = given_sse_mcp_server().await;
        let client = McpClient::new();

        // When listing tools
        let tools = client.list_tools(&mcp.address().into()).await.unwrap();

        // Then the add tool is listed
        assert_eq!(tools.len(), 2);
        assert!(tools.contains(&"add".to_owned()));
        assert!(tools.contains(&"saboteur".to_owned()));
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
        let client = McpClient::new();
        let response = client
            .invoke_tool(
                "add",
                arguments,
                &mcp.address().into(),
                TracingContext::dummy(),
            )
            .await
            .unwrap();

        assert_eq!(response.len(), 1);
        assert!(matches!(&response[0], Modality::Text { text } if text == "3"));
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
        let client = McpClient::new();
        let response = client
            .invoke_tool(
                "add",
                arguments,
                &mcp.address().into(),
                TracingContext::dummy(),
            )
            .await
            .unwrap();

        assert_eq!(response.len(), 1);
        assert!(matches!(&response[0], Modality::Text { text } if text == "3"));
    }

    #[tokio::test]
    async fn invoke_unknown_tool_gives_error() {
        let mcp = given_sse_mcp_server().await;

        let client = McpClient::new();
        let response = client
            .invoke_tool(
                "unknown",
                vec![],
                &mcp.address().into(),
                TracingContext::dummy(),
            )
            .await
            .unwrap_err();
        assert!(matches!(response, ToolError::ToolCallFailed(_)));
    }

    #[tokio::test]
    async fn invoke_saboteur_tool_results_in_error() {
        let mcp = given_sse_mcp_server().await;

        let client = McpClient::new();
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

        let client = McpClient::new();
        client.initialize(&mcp.address().into()).await.unwrap();
    }

    #[test]
    fn parse_tool_call_result_with_empty_list() {
        let result = ToolCallResult {
            content: vec![],
            is_error: false,
        };

        let result = McpClient::parse_tool_call_result(result);

        assert!(result.is_ok());
    }
}
