use serde_json::json;

pub struct Argument {
    pub name: String,
    pub value: Vec<u8>,
}

pub struct InvokeRequest {
    pub tool_name: String,
    pub arguments: Vec<Argument>,
}

pub async fn invoke_tool(request: InvokeRequest) -> Vec<u8> {
    // Determine the value to use based on the arguments provided.
    // - If exactly two arguments are provided as expected, use its value.
    // - Otherwise, response with the expectation.
    let value = match request.arguments.len() {
        2 if request.arguments[0].name == "a" && request.arguments[1].name == "b" => {
            let a = String::from_utf8(request.arguments[0].value.clone())
                .unwrap()
                .parse::<i32>()
                .unwrap();
            let b = String::from_utf8(request.arguments[1].value.clone())
                .unwrap()
                .parse::<i32>()
                .unwrap();
            let sum = a + b;
            json!(sum)
        }
        _ => {
            json!(format!(
                "Arguments a and b expected for the tool '{}'",
                request.tool_name
            ))
        }
    };
    value.to_string().into_bytes()
}
