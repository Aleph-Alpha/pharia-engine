mod actor;
mod store;

pub use self::{actor::{Mcp, McpApi}, store::McpServerStore};
#[cfg(test)]
pub use self::actor::McpDouble;

use crate::namespace_watcher::Namespace;
use serde::{Deserialize, Serialize};

#[derive(Clone, Hash, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct McpServerUrl(pub String);

impl McpServerUrl {
    pub fn new(url: impl Into<String>) -> Self {
        Self(url.into())
    }
}

impl<T> From<T> for McpServerUrl
where
    T: Into<String>,
{
    fn from(value: T) -> Self {
        Self(value.into())
    }
}

/// An MCP server that is configured with a namespace.
///
/// Per namespace configuration allows different Skills to have access to different tools.
#[derive(Clone, PartialEq, Eq, Debug, Hash)]
pub struct ConfiguredMcpServer {
    pub url: McpServerUrl,
    pub namespace: Namespace,
}

impl ConfiguredMcpServer {
    pub fn new(url: impl Into<McpServerUrl>, namespace: Namespace) -> Self {
        Self {
            url: url.into(),
            namespace,
        }
    }
}
