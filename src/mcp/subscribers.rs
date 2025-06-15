use crate::tool::ToolRuntimeSender;

#[cfg(test)]
use double_trait::double;

/// Sibling trait for [`McpApi`] that allows to send messages to the MCP actor. This trait is used
/// by the MCP actor to send messages to receipients of tools. Outside of test code this is
/// implemented by the [`crate::tool::ToolRuntimeApi`]
#[cfg_attr(test, double(McpSubscriberDouble))]
pub trait McpSubscriber {}

impl McpSubscriber for ToolRuntimeSender {}
