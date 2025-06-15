use std::{collections::HashMap, sync::Arc};

use crate::tool::{QualifiedToolName, Tool, ToolRuntimeSender};

#[cfg(test)]
use double_trait::double;

/// Sibling trait for [`McpApi`] that allows to send messages to the MCP actor. This trait is used
/// by the MCP actor to send messages to receipients of tools. Outside of test code this is
/// implemented by the [`crate::tool::ToolRuntimeApi`]
#[cfg_attr(test, double(McpSubscriberDouble))]
pub trait McpSubscriber {
    /// Let the subscriber know that the list of tools has been changed and report the new list
    fn report_updated_tools(&self, tools: HashMap<QualifiedToolName, Arc<dyn Tool + Send>>);
}

impl McpSubscriber for ToolRuntimeSender {
    fn report_updated_tools(&self, tools: HashMap<QualifiedToolName, Arc<dyn Tool + Send>>) {
        // Do nothing for now
    }
}
