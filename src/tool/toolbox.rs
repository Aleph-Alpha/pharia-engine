use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    logging::TracingContext,
    namespace_watcher::Namespace,
    tool::{Argument, Modality, Tool, ToolError, actor::ToolClient},
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

    pub async fn fetch_tool(&self, namespace: Namespace, name: &str) -> Arc<dyn Tool>
    where
        T: ToolClient + 'static,
    {
        Arc::new(McpTool::new(
            name.to_owned(),
            McpServerUrl::from("http://localhost:8080"),
            self.client.clone(),
        ))
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

struct McpTool<C> {
    name: String,
    url: McpServerUrl,
    client: Arc<C>,
}

impl<C> McpTool<C> {
    pub fn new(name: String, url: McpServerUrl, client: Arc<C>) -> Self {
        Self { name, url, client }
    }
}

#[async_trait]
impl<C> Tool for McpTool<C>
where
    C: ToolClient,
{
    async fn invoke(
        &self,
        arguments: Vec<Argument>,
        tracing_context: TracingContext,
    ) -> Result<Vec<Modality>, ToolError> {
        self.client
            .invoke_tool(&self.name, arguments, &self.url, tracing_context)
            .await
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

    #[tokio::test]
    async fn invoke_tool_success() {
        struct ToolClientStub;
        impl ToolClientDouble for ToolClientStub {
            async fn invoke_tool(
                &self,
                _name: &str,
                _args: Vec<Argument>,
                _url: &McpServerUrl,
                _tracing_context: TracingContext,
            ) -> Result<Vec<Modality>, ToolError> {
                Ok(vec![Modality::Text {
                    text: "success".to_string(),
                }])
            }
        }

        let toolbox = Toolbox::new(ToolClientStub);
        let tool = toolbox.fetch_tool(Namespace::dummy(), "test").await;

        let arguments = vec![];
        let modalities = tool
            .invoke(arguments, TracingContext::dummy())
            .await
            .unwrap();

        assert_eq!(
            &modalities,
            &[Modality::Text {
                text: "success".to_string()
            }]
        );
    }
}
