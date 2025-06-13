mod actor;
mod client;
mod mcp_routes;
mod tool_routes;
mod toolbox;

use async_trait::async_trait;
use serde::Deserialize;

use crate::logging::TracingContext;

pub use self::{
    actor::{ConfiguredMcpServer, InvokeRequest, ToolApi, ToolRuntime, ToolSender, ToolStoreApi},
    mcp_routes::{McpServerStoreProvider, http_mcp_servers_v1, openapi_mcp_servers_v1},
    tool_routes::{ToolProvider, http_tools_v1, openapi_tools_v1},
    toolbox::{ConfiguredNativeTool, McpServerUrl},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Argument {
    pub name: String,
    pub value: Vec<u8>,
}

pub type ToolOutput = Vec<Modality>;

#[derive(Deserialize, Debug, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Modality {
    // There are more types of content, but we do not support these at the moment.
    // See: <https://modelcontextprotocol.io/specification/2025-03-26/server/tools#tool-result>
    Text { text: String },
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    // We plan on passing this error variant to the Skills and model.
    #[error("{0}")]
    ToolCallFailed(String),
    // This variant should also be exposed to the Skill, as the model can recover from this.
    #[error("Tool {0} not found on any server.")]
    ToolNotFound(String),
    #[error("The tool call could not be executed, original error: {0}")]
    Other(#[from] anyhow::Error),
}

#[async_trait]
pub trait Tool {
    async fn invoke(
        self: Box<Self>,
        arguments: Vec<Argument>,
        tracing_context: TracingContext,
    ) -> Result<Vec<Modality>, ToolError>;
}

#[cfg(test)]
pub mod tests {
    pub use super::actor::ToolStoreDouble;
}
