use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use serde::{Deserialize, Serialize};

use crate::{
    namespace_watcher::Namespace,
    tool::{Modality, Tool, ToolError},
};

pub struct Toolbox<T> {
    // Map of which MCP servers are configured for which namespace.
    mcp_servers: McpServerStore,
    native_tools: NativeToolStore,
    pub client: Arc<T>,
}

impl<T> Toolbox<T> {
    pub fn new(client: T) -> Self {
        Self {
            mcp_servers: McpServerStore::new(),
            native_tools: NativeToolStore::new(),
            client: Arc::new(client),
        }
    }

    pub fn fetch_tool(&self, namespace: Namespace, name: &str) -> Arc<dyn Tool> {
        Arc::new(McpTool)
    }

    pub fn list_mcp_servers_in_namespace(
        &self,
        namespace: Namespace,
    ) -> impl Iterator<Item = McpServerUrl> + '_ {
        self.mcp_servers.list_in_namespace(namespace)
    }

    pub fn upsert_mcp_server(&mut self, namespace: Namespace, url: McpServerUrl) {
        self.mcp_servers.upsert(namespace, url);
    }

    pub fn remove_mcp_server(&mut self, namespace: Namespace, url: McpServerUrl) {
        self.mcp_servers.remove(namespace, url);
    }

    pub fn upsert_native_tool(&self, tool: ConfiguredNativeTool) {
        self.native_tools.upsert(tool);
    }

    pub fn remove_native_tool(&self, tool: ConfiguredNativeTool) {
        self.native_tools.remove(tool);
    }
}

#[derive(Clone, Hash, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct McpServerUrl(pub String);

impl<T> From<T> for McpServerUrl
where
    T: Into<String>,
{
    fn from(value: T) -> Self {
        Self(value.into())
    }
}

struct McpTool;

impl Tool for McpTool {
    fn invoke(&self) -> Result<Vec<Modality>, ToolError> {
        Ok(vec![Modality::Text {
            text: "success".to_string(),
        }])
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ConfiguredNativeTool {
    pub name: String,
    pub namespace: Namespace,
}

struct McpServerStore(HashMap<Namespace, HashSet<McpServerUrl>>);

impl McpServerStore {
    fn new() -> Self {
        Self(HashMap::new())
    }

    fn list_in_namespace(&self, namespace: Namespace) -> impl Iterator<Item = McpServerUrl> + '_ {
        self.0
            .get(&namespace)
            .cloned()
            .unwrap_or_default()
            .into_iter()
    }

    fn upsert(&mut self, namespace: Namespace, url: McpServerUrl) {
        self.0.entry(namespace).or_default().insert(url);
    }

    fn remove(&mut self, namespace: Namespace, url: McpServerUrl) {
        if let Some(servers) = self.0.get_mut(&namespace) {
            servers.remove(&url);
            if servers.is_empty() {
                self.0.remove(&namespace);
            }
        }
    }
}

struct NativeToolStore;

impl NativeToolStore {
    fn new() -> Self {
        Self
    }

    #[allow(clippy::unused_self)]
    fn upsert(&self, _tool: ConfiguredNativeTool) {}

    #[allow(clippy::unused_self)]
    fn remove(&self, _tool: ConfiguredNativeTool) {}
}

#[cfg(test)]
pub mod tests {
    use crate::tool::actor::ToolClientDouble;

    use super::*;

    #[test]
    fn invoke_tool_success() {
        struct ToolClientStub;
        impl ToolClientDouble for ToolClientStub {}

        let toolbox = Toolbox::new(ToolClientStub);
        let tool = toolbox.fetch_tool(Namespace::dummy(), "test");

        let modalities = tool.invoke().unwrap();

        assert_eq!(
            &modalities,
            &[Modality::Text {
                text: "success".to_string()
            }]
        );
    }
}
