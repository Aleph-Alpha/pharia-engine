use std::sync::Arc;

use crate::{
    logging::TracingContext,
    tool::{Argument, Modality, Tool, ToolDescription, ToolError},
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Native tool names specify which (native) tools can be configured per namespace.
/// A user may list any of these in the namespace config to make them available.
/// For the `test-beta` namespace, we offer a set of the native tools, which is defined in
/// [`crate::tool::toolbox::Toolbox`].
#[derive(Clone, Deserialize, Serialize, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum NativeToolName {
    Add,
    Subtract,
    Saboteur,
}

impl NativeToolName {
    pub fn tool(&self) -> Arc<dyn Tool + Send + Sync + 'static> {
        match self {
            NativeToolName::Add => Arc::new(Add),
            NativeToolName::Subtract => Arc::new(Subtract),
            NativeToolName::Saboteur => Arc::new(Saboteur),
        }
    }
}

struct Saboteur;

#[async_trait]
impl Tool for Saboteur {
    async fn invoke(
        &self,
        _args: Vec<Argument>,
        _tracing_context: TracingContext,
    ) -> Result<Vec<Modality>, ToolError> {
        Err(ToolError::ToolExecution("Out of cheese.".to_string()))
    }

    fn description(&self) -> ToolDescription {
        let schema = json!({
            "properties": {},
            "title": "saboteurArguments",
            "type": "object"
        });
        ToolDescription::new("saboteur", "A tool that always raises an error.", schema)
    }
}

struct Add;

#[async_trait]
impl Tool for Add {
    async fn invoke(
        &self,
        args: Vec<Argument>,
        _tracing_context: TracingContext,
    ) -> Result<Vec<Modality>, ToolError> {
        let args = Arguments(args);
        let a: i32 = args.get("a")?;
        let b: i32 = args.get("b")?;
        Ok(vec![Modality::Text {
            text: (a + b).to_string(),
        }])
    }

    fn description(&self) -> ToolDescription {
        let schema = json!(
            {
                "properties": {
                    "a": {
                        "title": "A",
                        "type": "integer"
                    },
                    "b": {
                        "title": "B",
                        "type": "integer"
                    }
                },
                "required": [
                    "a",
                    "b"
                ],
                "title": "addArguments",
                "type": "object"
            }
        );
        ToolDescription::new("add", "Add two numbers", schema)
    }
}

struct Subtract;

#[async_trait]
impl Tool for Subtract {
    async fn invoke(
        &self,
        args: Vec<Argument>,
        _tracing_context: TracingContext,
    ) -> Result<Vec<Modality>, ToolError> {
        let args = Arguments(args);
        let a: i32 = args.get("a")?;
        let b: i32 = args.get("b")?;
        Ok(vec![Modality::Text {
            text: (a - b).to_string(),
        }])
    }

    fn description(&self) -> ToolDescription {
        let schema = json!(
            {
                "properties": {
                    "a": {
                        "title": "A",
                        "type": "integer"
                    },
                    "b": {
                        "title": "B",
                        "type": "integer"
                    }
                },
                "required": [
                    "a",
                    "b"
                ],
                "title": "addArguments",
                "type": "object"
            }
        );
        ToolDescription::new("subtract", "Subtract two numbers", schema)
    }
}

impl NativeToolName {
    pub fn name(&self) -> &str {
        match self {
            NativeToolName::Add => "add",
            NativeToolName::Subtract => "subtract",
            NativeToolName::Saboteur => "saboteur",
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
            .ok_or(ToolError::ToolExecution(format!(
                "Argument {name} not specified"
            )))?;
        serde_json::from_slice(&arg.value).map_err(|e| {
            ToolError::ToolExecution(format!("Error deserializing argument {name}: {e:#}"))
        })
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
        let result = Add.invoke(args, TracingContext::dummy()).await.unwrap();

        // Then the result is the sum of the two numbers
        assert_eq!(
            result,
            vec![Modality::Text {
                text: "3".to_string()
            }]
        );
    }
}
