use std::{collections::HashMap, sync::Arc};

use crate::tool::{QualifiedToolName, Tool, ToolRuntimeSender, ToolStoreApi};

#[cfg(test)]
use double_trait::double;

/// Sibling trait for [`McpApi`] that allows to send messages to the MCP actor. This trait is used
/// by the MCP actor to send messages to receipients of tools. Outside of test code this is
/// implemented by the [`crate::tool::ToolRuntimeApi`]
#[cfg_attr(test, double(McpSubscriberDouble))]
pub trait McpSubscriber {
    /// Let the subscriber know that the list of tools has been changed and report the new list
    fn report_updated_tools(
        &mut self,
        tools: HashMap<QualifiedToolName, Arc<dyn Tool + Send + Sync>>,
    ) -> impl Future<Output = ()> + Send;
}

impl McpSubscriber for ToolRuntimeSender {
    async fn report_updated_tools(
        &mut self,
        tools: HashMap<QualifiedToolName, Arc<dyn Tool + Send + Sync>>,
    ) {
        ToolStoreApi::report_updated_tools(self, tools).await;
    }
}
