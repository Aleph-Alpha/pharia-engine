use crate::{
    logging::TracingContext,
    tool::{Argument, Modality, Tool, ToolError},
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum NativeTool {
    Add,
    Subtract,
}

impl NativeTool {
    pub fn name(&self) -> &str {
        match self {
            NativeTool::Add => "add",
            NativeTool::Subtract => "subtract",
        }
    }
}

struct Arguments(Vec<Argument>);

impl Arguments {
    fn get<'de, D>(&'de self, name: &str) -> Result<D, ToolError>
    where
        D: Deserialize<'de>,
    {
        let arg = self
            .0
            .iter()
            .find(|arg| arg.name == name)
            .ok_or(ToolError::LogicError(format!(
                "Argument {name} not specified"
            )))?;
        serde_json::from_slice(&arg.value).map_err(|e| {
            ToolError::LogicError(format!("Error deserializing argument {name}: {e:#}"))
        })
    }
}

#[async_trait]
impl Tool for NativeTool {
    async fn invoke(
        &self,
        args: Vec<Argument>,
        _tracing_context: TracingContext,
    ) -> Result<Vec<Modality>, ToolError> {
        let args = Arguments(args);
        match self {
            NativeTool::Add => {
                let a: i32 = args.get("a")?;
                let b: i32 = args.get("b")?;
                Ok(vec![Modality::Text {
                    text: (a + b).to_string(),
                }])
            }
            NativeTool::Subtract => {
                let a: i32 = args.get("a")?;
                let b: i32 = args.get("b")?;
                Ok(vec![Modality::Text {
                    text: (a - b).to_string(),
                }])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn add_tool_can_add_two_numbers() {
        // Given a request to add two numbers
        let args = vec![
            Argument {
                name: "a".to_string(),
                value: json!(1).to_string().into_bytes(),
            },
            Argument {
                name: "b".to_string(),
                value: json!(2).to_string().into_bytes(),
            },
        ];

        // When the tool is invoked
        let result = NativeTool::Add
            .invoke(args, TracingContext::dummy())
            .await
            .unwrap();

        // Then the result is the sum of the two numbers
        assert_eq!(
            result,
            vec![Modality::Text {
                text: "3".to_string()
            }]
        );
    }
}
