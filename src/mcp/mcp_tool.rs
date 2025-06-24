use std::sync::Arc;

use async_trait::async_trait;

use crate::{
    logging::TracingContext,
    tool::{Argument, Modality, Tool, ToolDescription, ToolError},
};

use super::{client::McpClient, store::McpToolDesc};

/// Implementation of [`crate::tool::Tool`] specific to Model Context Protocol (MCP) tools.
pub struct McpTool<C> {
    desc: McpToolDesc,
    client: Arc<C>,
}

impl<C> McpTool<C> {
    pub fn new(desc: McpToolDesc, client: Arc<C>) -> Self {
        Self { desc, client }
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
            .invoke_tool(
                &self.desc.name,
                arguments,
                &self.desc.server,
                tracing_context,
            )
            .await
    }

    fn description(&self) -> ToolDescription {
        unimplemented!()
    }
}
