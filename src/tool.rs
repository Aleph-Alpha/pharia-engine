use anyhow::anyhow;
use futures::StreamExt;
use reqwest::Client;
use reqwest::Response;
use reqwest::header;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;

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
    #[error("{0}")]
    ToolCallFailed(String),
    #[error("The proposed protocol version {0} from the MCP server is not supported.")]
    InvalidProtocolVersion(String),
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

pub async fn invoke_tool(request: InvokeRequest, mcp_address: &str) -> Result<Vec<u8>, ToolError> {
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
        return Err(ToolError::InvalidProtocolVersion(
            response.result.protocol_version,
        ));
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
mod test {
    use test_skills::{given_json_mcp_server, given_sse_mcp_server};

    use super::*;

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
        let response = invoke_tool(request, mcp.address()).await.unwrap();
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
        let response = invoke_tool(request, mcp.address()).await.unwrap();
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
        let response = invoke_tool(request, mcp.address()).await.unwrap_err();
        assert!(matches!(response, ToolError::ToolCallFailed(_)));
    }

    #[tokio::test]
    async fn invoke_saboteur_tool_results_in_error() {
        let mcp = given_sse_mcp_server().await;

        let request = InvokeRequest {
            tool_name: "saboteur".to_owned(),
            arguments: vec![],
        };
        let response = invoke_tool(request, mcp.address()).await.unwrap_err();
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
