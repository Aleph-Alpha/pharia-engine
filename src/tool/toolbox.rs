use std::sync::Arc;

use crate::{
    mcp::{McpClient, McpServerStore, McpServerUrl, McpTool},
    namespace_watcher::Namespace,
    tool::{QualifiedToolName, Tool},
};

pub struct Toolbox<T> {
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
            mcp_servers: McpServerStore::new(),
            native_tools: NativeToolStore::new(),
            client: Arc::new(client),
        }
    }

    pub async fn fetch_tool(
        &mut self,
        qtn: &QualifiedToolName,
    ) -> Option<Box<dyn Tool + Send + Sync>>
    where
        T: McpClient + 'static,
    {
        self.mcp_servers
            .update_tool_list(self.client.as_ref())
            .await;
        let tools = self.mcp_servers.list_tools_for_namespace(&qtn.namespace);
        let tool_desc = tools.into_iter().find(|tool| tool.name == qtn.name)?;
        Some(Box::new(McpTool::new(tool_desc, self.client.clone())))
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
        mcp::ToolClientDouble,
        tool::{Argument, Modality, ToolError},
    };

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
