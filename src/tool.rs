mod actor;
mod native_tool;
mod tool_routes;
mod toolbox;

use std::cmp::Ordering;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

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

/// The information about a tool that is returned by the MCP server.
///
/// With models making the decision on which tools to call, they need information about what the
/// tool does and what the input schema is.
#[derive(PartialEq, Eq, Clone)]
pub struct ToolInformation {
    name: String,
    description: String,
    input_schema: Value,
}

impl ToolInformation {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Tools are sorted by their name.
impl Ord for ToolInformation {
    fn cmp(&self, other: &Self) -> Ordering {
        self.name.cmp(&other.name)
    }
}

impl PartialOrd for ToolInformation {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
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
    /// The tool call was executed, but it exited with an error. The model should get the
    /// opportunity to retry. There are two general causes for this, which we can not distinguish:
    ///
    /// 1. The tool had an internal error.
    /// 2. The provided arguments were not correct.
    #[error("{0}")]
    ToolExecution(String),
    /// This error could mean that there is something wrong in the Skill code (the developer
    /// specified a tool that never existed). It could also mean a runtime error (the tool was
    /// removed from the server). It could also be a logic error from the model, where it chose
    /// to call a tool that is not available.
    #[error("Tool {0} not found on any server.")]
    ToolNotFound(String),
    /// An error was encountered connecting to the tool. An example would be a timeout to an MCP
    /// server, or a server returning an invalid payload.
    #[error("The tool call could not be executed, original error: {0}")]
    RuntimeError(#[from] anyhow::Error),
}

#[cfg(test)]
pub mod tests {
    use serde_json::Value;

    use crate::tool::ToolInformation;

    pub use super::actor::ToolStoreDouble;

    impl ToolInformation {
        pub fn with_name(name: impl Into<String>) -> Self {
            Self {
                name: name.into(),
                description: String::new(),
                input_schema: Value::Null,
            }
        }

        pub fn description(&self) -> &str {
            &self.description
        }

        pub fn input_schema(&self) -> &Value {
            &self.input_schema
        }
    }
}
