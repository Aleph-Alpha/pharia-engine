use std::{collections::HashMap, sync::Arc};

use crate::{
    mcp::{McpClient, McpServerStore, McpServerUrl, McpTool},
    namespace_watcher::Namespace,
    tool::{QualifiedToolName, Tool},
};

pub struct Toolbox<T> {
    /// Tools reported by the MCP servers
    mcp_tools: HashMap<QualifiedToolName, Arc<dyn Tool + Send + Sync>>,
    // Map of which MCP servers are configured for which namespace.
    mcp_servers: McpServerStore,
    native_tools: NativeToolStore,
    pub client: Arc<T>,
}

impl<T> Toolbox<T>
where
    T: McpClient + 'static,
{
    pub fn new(client: T) -> Self {
        Self {
            mcp_tools: HashMap::new(),
            mcp_servers: McpServerStore::new(),
            native_tools: NativeToolStore::new(),
            client: Arc::new(client),
        }
    }

    pub async fn fetch_tool(
        &mut self,
        qtn: &QualifiedToolName,
    ) -> Option<Arc<dyn Tool + Send + Sync>>
    where
        T: McpClient + 'static,
    {
        self.mcp_servers
            .update_tool_list(self.client.as_ref())
            .await;
        self.mcp_tools = self
            .mcp_servers
            .all_tools_by_name()
            .map(|(qtn, desc)| {
                let tool = McpTool::new(desc, self.client.clone());
                let tool: Arc<dyn Tool + Send + Sync> = Arc::new(tool);
                (qtn, tool)
            })
            .collect();
        let tool = self.mcp_tools.get(qtn)?.clone();
        Some(tool)
    }

    pub fn list_mcp_servers_in_namespace(
        &self,
        namespace: &Namespace,
    ) -> impl Iterator<Item = McpServerUrl> + '_ {
        self.mcp_servers.list_in_namespace(namespace)
    }

    pub async fn upsert_mcp_server(&mut self, namespace: Namespace, url: McpServerUrl) {
        self.mcp_servers.upsert(namespace, url, &*self.client).await;
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

    pub(crate) fn update_tools(
        &mut self,
        tools: HashMap<QualifiedToolName, Arc<dyn Tool + Send + Sync + 'static>>,
    ) {
        self.mcp_tools = tools;
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
    use crate::{
        logging::TracingContext,
        mcp::McpClientDouble,
        tool::{Argument, Modality, ToolError},
    };

    use super::*;

    #[tokio::test]
    async fn invoke_tool_success() {
        struct ToolClientStub;
        impl McpClientDouble for ToolClientStub {
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
        toolbox
            .upsert_mcp_server(
                Namespace::dummy(),
                McpServerUrl::from("http://localhost:8080"),
            )
            .await;
        let tool = toolbox
            .fetch_tool(&QualifiedToolName {
                namespace: Namespace::dummy(),
                name: "test".to_owned(),
            })
            .await
            .unwrap();

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
        impl McpClientDouble for ToolClientStub {
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
        toolbox
            .upsert_mcp_server(
                Namespace::dummy(),
                McpServerUrl::from("http://localhost:8080"),
            )
            .await;
        let maybe_tool = toolbox
            .fetch_tool(&QualifiedToolName {
                namespace: Namespace::dummy(),
                name: "test".to_owned(),
            })
            .await;

        assert!(maybe_tool.is_none());
    }
}
