use std::sync::Arc;

use async_trait::async_trait;

use crate::{
    logging::TracingContext,
    mcp::{McpClient, McpServerStore, McpServerUrl},
    namespace_watcher::Namespace,
    tool::{Argument, Modality, Tool, ToolError},
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

    async fn mcp_server_for_tool(
        urls: Vec<McpServerUrl>,
        client: Arc<T>,
        name: String,
    ) -> Option<McpServerUrl>
    where
        T: McpClient + 'static,
    {
        for url in urls {
            if let Ok(tools) = client.list_tools(&url).await {
                if tools.contains(&name) {
                    return Some(url);
                }
            }
        }

        None
    }

    pub fn fetch_tool(
        &self,
        namespace: Namespace,
        name: &str,
    ) -> Option<Box<dyn Tool + Send + Sync>>
    where
        T: McpClient + 'static,
    {
        let urls = self.mcp_servers.list_in_namespace(&namespace).collect();
        let server = Toolbox::mcp_server_for_tool(urls, self.client.clone(), name.to_owned());
        Some(Box::new(McpTool::new(
            name.to_owned(),
            server,
            self.client.clone(),
        )))
    }

    pub fn list_mcp_servers_in_namespace(
        &self,
        namespace: &Namespace,
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

struct McpTool<C, U> {
    name: String,
    url: U,
    client: Arc<C>,
}

impl<C, U> McpTool<C, U> {
    pub fn new(name: String, url: U, client: Arc<C>) -> Self {
        Self { name, url, client }
    }
}

#[async_trait]
impl<C, U> Tool for McpTool<C, U>
where
    C: McpClient,
    U: Future<Output = Option<McpServerUrl>> + Send + Sync,
{
    async fn invoke(
        self: Box<Self>,
        arguments: Vec<Argument>,
        tracing_context: TracingContext,
    ) -> Result<Vec<Modality>, ToolError> {
        self.client
            .invoke_tool(
                &self.name,
                arguments,
                &self
                    .url
                    .await
                    .ok_or_else(|| ToolError::ToolNotFound(self.name.clone()))?,
                tracing_context,
            )
            .await
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ConfiguredNativeTool {
    pub name: String,
    pub namespace: Namespace,
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
    use crate::mcp::ToolClientDouble;

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

            async fn list_tools(&self, _url: &McpServerUrl) -> Result<Vec<String>, anyhow::Error> {
                Ok(vec!["test".to_string()])
            }
        }

        let mut toolbox = Toolbox::new(ToolClientStub);
        toolbox.upsert_mcp_server(
            Namespace::dummy(),
            McpServerUrl::from("http://localhost:8080"),
        );
        let tool = toolbox.fetch_tool(Namespace::dummy(), "test").unwrap();

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

    #[tokio::test]
    async fn fetch_missing_tool() {
        struct ToolClientStub;
        impl ToolClientDouble for ToolClientStub {
            async fn invoke_tool(
                &self,
                _name: &str,
                _args: Vec<Argument>,
                _url: &McpServerUrl,
                _tracing_context: TracingContext,
            ) -> Result<Vec<Modality>, ToolError> {
                Ok(vec![])
            }

            async fn list_tools(&self, _url: &McpServerUrl) -> Result<Vec<String>, anyhow::Error> {
                Ok(vec![])
            }
        }

        let mut toolbox = Toolbox::new(ToolClientStub);
        toolbox.upsert_mcp_server(
            Namespace::dummy(),
            McpServerUrl::from("http://localhost:8080"),
        );
        let tool = toolbox.fetch_tool(Namespace::dummy(), "test").unwrap();
        let result = tool.invoke(vec![], TracingContext::dummy()).await;

        assert!(matches!(
            result,
            Err(ToolError::ToolNotFound(tool_name))
            if tool_name == "test"
        ));
    }
}
