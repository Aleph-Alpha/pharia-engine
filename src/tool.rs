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
    // is_error: bool,
}

#[derive(Deserialize)]
struct ToolCallResponse {
    // jsonrpc: String,
    // id: u64,
    result: ToolCallResponseResult,
}

pub async fn invoke_tool(request: InvokeRequest) -> Vec<u8> {
    let client = Client::new();
    let arguments = request
        .arguments
        .into_iter()
        .map(|argument| {
            (
                argument.name,
                serde_json::from_slice::<Value>(&argument.value).unwrap(),
            )
        })
        .collect::<HashMap<_, _>>();
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
        .post(MCP_SERVER_ADDRESS)
        // MCP server want exactly these two headers, even a wildcard is not accepted
        .header("accept", "application/json,text/event-stream")
        .json(&body)
        .send()
        .await
        .unwrap();
    let mut stream = response.bytes_stream();
    let item = stream.next().await.unwrap().unwrap();
    let item = String::from_utf8(item.to_vec()).unwrap();
    let data = item.split("data: ").nth(1).unwrap();
    let value = serde_json::from_str::<ToolCallResponse>(data).unwrap();
    let ToolCallResponseContent::Text { text } = value.result.content.first().unwrap();
    text.to_owned().into_bytes()
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
        let response = invoke_tool(request).await;
        let response = String::from_utf8(response).unwrap();

        assert_eq!(response, "3");
    }
}
