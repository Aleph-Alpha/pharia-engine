mod actor;
mod client;
mod routes;

pub use self::{
    actor::{
        Argument, ConfiguredMcpServer, InvokeRequest, McpServerStoreApi, McpServerUrl, Tool,
        ToolApi, ToolError,
    },
    routes::{McpServerStoreProvider, http_tools_v1, openapi_tools_v1},
};

#[cfg(test)]
pub mod tests {
    pub use super::actor::McpServerStoreDouble;
}
