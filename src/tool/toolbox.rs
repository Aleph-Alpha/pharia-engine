use std::{collections::HashMap, sync::Arc};

use itertools::Itertools;

use crate::{
    namespace_watcher::Namespace,
    tool::{NativeToolName, QualifiedToolName, Tool},
};

/// Registry of all tools known to the kernel.
///
/// The toolbox maintains two separate catalogs:
/// 1. `mcp_tools` – tools announced by remote MCP servers.
/// 2. `native_tools` – tools implemented directly inside the kernel.
///
/// The toolbox is periodically notified about new tools.
pub struct Toolbox {
    /// Tools reported by the MCP servers
    mcp_tools: HashMap<QualifiedToolName, Arc<dyn Tool + Send + Sync>>,
    /// Tools offered by the Kernel itself
    native_tools: HashMap<QualifiedToolName, Arc<dyn Tool + Send + Sync>>,
}

impl Toolbox {
    // The list of tools that are configured per default in the test-beta namespace.
    // This namespace allows fast testing of the tool feature.
    const NATIVE_TOOLS_IN_TEST_BETA: &[NativeToolName] = &[
        NativeToolName::Add,
        NativeToolName::Subtract,
        NativeToolName::Saboteur,
    ];

    pub fn new() -> Self {
        let mut native_tools = HashMap::new();
        for tool in Self::NATIVE_TOOLS_IN_TEST_BETA {
            native_tools.insert(
                QualifiedToolName {
                    namespace: Namespace::new("test-beta").unwrap(),
                    name: tool.name().to_owned(),
                },
                tool.tool(),
            );
        }
        Self {
            mcp_tools: HashMap::new(),
            native_tools,
        }
    }

    pub fn fetch_tool(&mut self, qtn: &QualifiedToolName) -> Option<Arc<dyn Tool + Send + Sync>> {
        self.native_tools
            .get(qtn)
            .cloned()
            .or_else(|| self.mcp_tools.get(qtn).cloned())
    }

    pub fn list_tools_in_namespace(&self, namespace: &Namespace) -> Vec<String> {
        self.mcp_tools
            .keys()
            .chain(self.native_tools.keys())
            .filter_map(|qtn| {
                if qtn.namespace == *namespace {
                    Some(qtn.name.clone())
                } else {
                    None
                }
            })
            .sorted()
            .collect()
    }

    pub fn upsert_native_tool(&mut self, tool: ConfiguredNativeTool) {
        self.native_tools
            .insert(tool.qualified_tool(), tool.name.tool());
    }

    pub fn remove_native_tool(&mut self, tool: ConfiguredNativeTool) {
        self.native_tools.remove(&tool.qualified_tool());
    }

    pub(crate) fn update_tools(
        &mut self,
        tools: HashMap<QualifiedToolName, Arc<dyn Tool + Send + Sync + 'static>>,
    ) {
        self.mcp_tools = tools;
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub struct ConfiguredNativeTool {
    pub name: NativeToolName,
    pub namespace: Namespace,
}

impl ConfiguredNativeTool {
    pub fn qualified_tool(&self) -> QualifiedToolName {
        QualifiedToolName {
            namespace: self.namespace.clone(),
            name: self.name.name().to_owned(),
        }
    }
}

#[cfg(test)]
pub mod tests {
    use double_trait::Dummy;

    use crate::{
        logging::TracingContext,
        tool::{Argument, Modality, ToolError},
    };

    use super::*;

    #[tokio::test]
    async fn invoke_tool_success() {
        struct ToolStub;

        #[async_trait::async_trait]
        impl Tool for ToolStub {
            async fn invoke(
                &self,
                _args: Vec<Argument>,
                _tracing_context: TracingContext,
            ) -> Result<Vec<Modality>, ToolError> {
                Ok(vec![Modality::Text {
                    text: "success".to_string(),
                }])
            }
        }

        let mut toolbox = Toolbox::new();
        toolbox.update_tools(HashMap::from([(
            QualifiedToolName {
                namespace: Namespace::dummy(),
                name: "test".to_owned(),
            },
            Arc::new(ToolStub) as Arc<dyn Tool + Send + Sync>,
        )]));

        let tool = toolbox
            .fetch_tool(&QualifiedToolName {
                namespace: Namespace::dummy(),
                name: "test".to_owned(),
            })
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
        // Given an empty toolbox
        let mut toolbox = Toolbox::new();

        // When we try to fetch a tool that does not exist
        let maybe_tool = toolbox.fetch_tool(&QualifiedToolName {
            namespace: Namespace::dummy(),
            name: "test".to_owned(),
        });

        // Then we expect it to return None
        assert!(maybe_tool.is_none());
    }

    #[tokio::test]
    async fn fetch_native_tool() {
        // Given a toolbox with a native tool
        let mut toolbox = Toolbox::new();
        toolbox.upsert_native_tool(ConfiguredNativeTool {
            name: NativeToolName::Add,
            namespace: Namespace::dummy(),
        });

        // When we fetch the tool
        let tool = toolbox.fetch_tool(&QualifiedToolName {
            namespace: Namespace::dummy(),
            name: "add".to_owned(),
        });

        // Then we expect it to return the tool
        assert!(tool.is_some());
    }

    #[tokio::test]
    async fn hardcoded_native_tools_are_listed_in_test_namespace() {
        // Given a toolbox
        let toolbox = Toolbox::new();

        // When we list the tools in the `test-beta` namespace
        let tools = toolbox.list_tools_in_namespace(&Namespace::new("test-beta").unwrap());

        // Then we expect the tools to be listed
        assert_eq!(tools, vec!["add", "saboteur", "subtract"]);
    }

    #[tokio::test]
    async fn upserted_native_tool_is_listed() {
        // Given a toolbox with a native tool
        let mut toolbox = Toolbox::new();
        toolbox.upsert_native_tool(ConfiguredNativeTool {
            name: NativeToolName::Add,
            namespace: Namespace::dummy(),
        });

        // When we list the tools
        let tools = toolbox.list_tools_in_namespace(&Namespace::dummy());

        // Then we expect the tool to be listed
        assert_eq!(tools, vec!["add"]);
    }

    #[tokio::test]
    async fn removed_native_tool_is_not_listed() {
        // Given a toolbox with a native tool
        let mut toolbox = Toolbox::new();
        toolbox.upsert_native_tool(ConfiguredNativeTool {
            name: NativeToolName::Add,
            namespace: Namespace::dummy(),
        });
        toolbox.remove_native_tool(ConfiguredNativeTool {
            name: NativeToolName::Add,
            namespace: Namespace::dummy(),
        });

        // When we list the tools
        let tools = toolbox.list_tools_in_namespace(&Namespace::dummy());

        // Then we expect the tool to be not listed
        assert!(tools.is_empty());
    }

    #[tokio::test]
    async fn tools_are_sorted() {
        // Given a toolbox with MCP tools and native tools
        let mut toolbox = Toolbox::new();
        toolbox.update_tools(HashMap::from([
            (
                QualifiedToolName {
                    namespace: Namespace::dummy(),
                    name: "a".to_owned(),
                },
                Arc::new(Dummy) as Arc<dyn Tool + Send + Sync>,
            ),
            (
                QualifiedToolName {
                    namespace: Namespace::dummy(),
                    name: "b".to_owned(),
                },
                Arc::new(Dummy) as Arc<dyn Tool + Send + Sync>,
            ),
        ]));
        toolbox.upsert_native_tool(ConfiguredNativeTool {
            name: NativeToolName::Add,
            namespace: Namespace::dummy(),
        });
        toolbox.upsert_native_tool(ConfiguredNativeTool {
            name: NativeToolName::Subtract,
            namespace: Namespace::dummy(),
        });

        // When we list the tools
        let tools = toolbox.list_tools_in_namespace(&Namespace::dummy());

        // Then we expect the tool list to be sorted
        assert_eq!(tools, vec!["a", "add", "b", "subtract"]);
    }
}
