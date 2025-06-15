mod actor;
mod tool_routes;
mod toolbox;

use async_trait::async_trait;
use serde::Deserialize;

use crate::{logging::TracingContext, namespace_watcher::Namespace};

pub use self::{
    actor::{InvokeRequest, ToolRuntime, ToolRuntimeApi, ToolRuntimeSender, ToolStoreApi},
    tool_routes::{ToolProvider, http_tools_v1, openapi_tools_v1},
    toolbox::ConfiguredNativeTool,
};

/// A tool name that is qualified by a namespace. It can uniquely identify a tool across different
/// namespaces.
pub struct QualifiedToolName {
    /// The namespace in which the tool is defined.
    pub namespace: Namespace,
    /// The name of the tool as it is known within the namespace. Currently this is identical to the
    /// name of the tool as reported by the MCP server, yet it may diverge in the future in order to
    /// disambiguate tools calls, in the case of name collisions.
    pub name: String,
}

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
        &self,
        arguments: Vec<Argument>,
        tracing_context: TracingContext,
    ) -> Result<Vec<Modality>, ToolError>;
}

#[cfg(test)]
pub mod tests {
    pub use super::actor::ToolStoreDouble;
}
