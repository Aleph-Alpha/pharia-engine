use std::{collections::HashMap, sync::Arc};

use crate::{
    namespace_watcher::Namespace,
    tool::{Tool, ToolRuntimeSender, ToolStoreApi},
};

#[cfg(test)]
use double_trait::double;

/// Having the namespace as top-level key also allows to report namespaces without tools.
pub type ToolMap = HashMap<Namespace, HashMap<String, Arc<dyn Tool + Send + Sync>>>;

/// The `McpActor` watches a list of MCP servers and reports changes in the list of tools.
///
/// This trait is the interface for a recipient that is notified about new tools. Outside of test
/// code, this is implemented by the [`crate::tool::ToolRuntimeApi`].
///
/// For the actor, it represents the outgoing interface, while its sibling trait
/// [`crate::mcp::McpApi`] represents the incoming interface by which the actor is notified about
/// new MCP servers.
#[cfg_attr(test, double(McpSubscriberDouble))]
pub trait McpSubscriber {
    /// Let the subscriber know that the list of tools has been changed and report the new list
    fn report_updated_tools(&mut self, tools: ToolMap) -> impl Future<Output = ()> + Send;
}

impl McpSubscriber for ToolRuntimeSender {
    async fn report_updated_tools(&mut self, tools: ToolMap) {
        ToolStoreApi::report_updated_tools(self, tools).await;
    }
}
