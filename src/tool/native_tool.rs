use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    logging::TracingContext,
    tool::{Argument, Modality, Tool, ToolError},
};

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

#[async_trait]
impl Tool for NativeTool {
    async fn invoke(
        &self,
        _args: Vec<Argument>,
        _tracing_context: TracingContext,
    ) -> Result<Vec<Modality>, ToolError> {
        unimplemented!()
    }
}
