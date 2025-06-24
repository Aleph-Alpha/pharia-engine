use std::sync::Arc;

use async_trait::async_trait;

use crate::{
    logging::TracingContext,
    mcp::McpServerUrl,
    tool::{Argument, Modality, Tool, ToolDescription, ToolError},
};

use super::client::McpClient;

/// Implementation of [`crate::tool::Tool`] specific to Model Context Protocol (MCP) tools.
pub struct McpTool<C> {
    desc: ToolDescription,
    /// The URL of the MCP server providing the tool.
    server: McpServerUrl,
    client: Arc<C>,
}

impl<C> McpTool<C> {
    pub fn new(desc: ToolDescription, server: McpServerUrl, client: Arc<C>) -> Self {
        Self {
            desc,
            server,
            client,
        }
    }
}

#[async_trait]
impl<C> Tool for McpTool<C>
where
    C: McpClient,
{
    async fn invoke(
        &self,
        arguments: Vec<Argument>,
        tracing_context: TracingContext,
    ) -> Result<Vec<Modality>, ToolError> {
        self.client
            .invoke_tool(self.desc.name(), arguments, &self.server, tracing_context)
            .await
    }

    fn description(&self) -> ToolDescription {
        self.desc.clone()
    }
}
