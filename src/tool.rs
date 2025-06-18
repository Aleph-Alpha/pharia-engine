mod actor;
mod native_tool;
mod tool_routes;
mod toolbox;

use async_trait::async_trait;
use serde::Deserialize;

use crate::{logging::TracingContext, namespace_watcher::Namespace};

pub use self::{
    actor::{InvokeRequest, ToolRuntime, ToolRuntimeApi, ToolRuntimeSender, ToolStoreApi},
    native_tool::NativeToolName,
    tool_routes::{ToolProvider, http_tools_v1, openapi_tools_v1},
    toolbox::ConfiguredNativeTool,
};

#[cfg(test)]
use double_trait::double;

/// Interface offered by individual tools.
///
/// Introducing this interface allows us to introduce different tool concepts like mcp tools and
/// native tools.
#[async_trait]
#[cfg_attr(test, double(ToolDouble))]
pub trait Tool {
    async fn invoke(
        &self,
        arguments: Vec<Argument>,
        tracing_context: TracingContext,
    ) -> Result<Vec<Modality>, ToolError>;
}

/// A tool name that is qualified by a namespace. It can uniquely identify a tool across different
/// namespaces.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
    /// We assume there might be something wrong with the input to the tool.
    /// This could be wrong arguments, or wrong type of arguments.
    /// The model should get the opportunity to retry.
    #[error("{0}")]
    LogicError(String),
    /// This error could mean that there is something wrong in the Skill code (the developer
    /// specified a tool that never existed). It could also mean a runtime error (the tool was
    /// removed from the server). It could also be a logic error from the model, where it chose
    /// to call a tool that is not available.
    #[error("Tool {0} not found on any server.")]
    ToolNotFound(String),
    /// We assume something to be wrong with the runtime. We would want to retry. Otherwise, we
    /// would stop skill execution. We would not pass this to the mode.
    #[error("The tool call could not be executed, original error: {0}")]
    RuntimeError(#[from] anyhow::Error),
}

#[cfg(test)]
pub mod tests {
    pub use super::actor::ToolStoreDouble;
}
