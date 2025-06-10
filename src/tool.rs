mod actor;
mod client;
mod mcp_routes;
mod tool_routes;

pub use self::{
    actor::{
        Argument, ConfiguredMcpServer, InvokeRequest, McpServerStoreApi, McpServerUrl, Tool,
        ToolApi, ToolError,
    },
    mcp_routes::{McpServerStoreProvider, http_mcp_servers_v1, openapi_mcp_servers_v1},
    tool_routes::{ToolProvider, http_tools_v1, openapi_tools_v1},
};

#[cfg(test)]
pub mod tests {
    pub use super::actor::McpServerStoreDouble;
}
