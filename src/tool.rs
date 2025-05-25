use anyhow::anyhow;
use futures::StreamExt;
use reqwest::Client;
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

const MCP_SERVER_ADDRESS: &str = "http://localhost:8000/mcp";

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
pub enum ToolCallError {
    #[error("{0}")]
    ToolCallFailed(String),
    #[error("The tool call could not be executed, original error:{0}")]
    Other(#[from] anyhow::Error),
}

pub async fn invoke_tool(request: InvokeRequest) -> Result<Vec<u8>, ToolCallError> {
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
    let mut stream = client
        .post(MCP_SERVER_ADDRESS)
        // MCP server want exactly these two headers, even a wildcard is not accepted
        .header("accept", "application/json,text/event-stream")
        .json(&body)
        .send()
        .await
        .map_err(anyhow::Error::from)?
        .bytes_stream();
    let item = stream
        .next()
        .await
        .ok_or(anyhow!("No item in stream"))?
        .map(|item| String::from_utf8(item.to_vec()))
        .map_err(anyhow::Error::from)?
        .map_err(anyhow::Error::from)?;

    let data = item
        .split("data: ")
        .nth(1)
        .ok_or(anyhow!("No data in stream"))?;
    let value = serde_json::from_str::<ToolCallResponse>(data).map_err(anyhow::Error::from)?;
    match value.result {
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
            Err(ToolCallError::ToolCallFailed(text.to_owned()))
        }
    }
}

#[cfg(test)]
mod test {
    use test_skills::given_mcp_server;

    use super::*;

    #[tokio::test]
    async fn invoke_tool_given_mcp_server() {
        let _mcp = given_mcp_server().await;

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
        let response = invoke_tool(request).await.unwrap();
        let response = String::from_utf8(response).unwrap();
        assert_eq!(response, "3");
    }

    #[tokio::test]
    async fn invoke_saboteur_tool_results_in_error() {
        let _mcp = given_mcp_server().await;

        let request = InvokeRequest {
            tool_name: "saboteur".to_owned(),
            arguments: vec![],
        };
        let response = invoke_tool(request).await.unwrap_err();
        assert_eq!(
            response.to_string(),
            "Error executing tool saboteur: Out of cheese."
        );
    }
}
