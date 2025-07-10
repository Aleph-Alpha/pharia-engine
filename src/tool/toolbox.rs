use std::{collections::HashMap, sync::Arc};

use itertools::Itertools;

use crate::{
    mcp::ToolMap,
    namespace_watcher::Namespace,
    tool::{NativeToolName, QualifiedToolName, Tool, ToolDescription},
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
    mcp_tools: ToolMap,
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
        self.native_tools.get(qtn).cloned().or_else(|| {
            self.mcp_tools
                .get(&qtn.namespace)
                .and_then(|tools| tools.get(&qtn.name).cloned())
        })
    }

    /// List all tools in a given namespace.
    ///
    /// Returns an error if the namespace does not exist.
    pub fn list_tools_in_namespace(
        &self,
        namespace: &Namespace,
    ) -> Result<Vec<ToolDescription>, NamespaceNotFound> {
        // Find all native tools for the namespace
        let native_tools = self.native_tools.iter().filter_map(|(qtn, tool)| {
            if qtn.namespace == *namespace {
                Some(tool.description())
            } else {
                None
            }
        });
        let tools = self
            .mcp_tools
            .get(namespace)
            .ok_or(NamespaceNotFound)?
            .values()
            .map(|tool| tool.description())
            .chain(native_tools)
            .sorted()
            .collect();
        Ok(tools)
    }

    pub fn upsert_native_tool(&mut self, tool: ConfiguredNativeTool) {
        self.native_tools
            .insert(tool.qualified_tool(), tool.name.tool());
    }

    pub fn remove_native_tool(&mut self, tool: ConfiguredNativeTool) {
        self.native_tools.remove(&tool.qualified_tool());
    }

    pub(crate) fn update_tools(&mut self, tools: ToolMap) {
        self.mcp_tools = tools;
    }
}

#[derive(Debug)]
pub struct NamespaceNotFound;

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
    use serde_json::Value;

    use crate::{
        logging::TracingContext,
        tool::{Argument, ToolDouble, ToolError, ToolOutput},
    };

    use super::*;

    #[tokio::test]
    async fn invoke_tool_success() {
        struct ToolStub;

        #[async_trait::async_trait]
        impl ToolDouble for ToolStub {
            async fn invoke(
                &self,
                _args: Vec<Argument>,
                _tracing_context: TracingContext,
            ) -> Result<ToolOutput, ToolError> {
                Ok(ToolOutput::from_text("success"))
            }
        }

        let mut toolbox = Toolbox::new();
        let tools = HashMap::from([(
            Namespace::dummy(),
            HashMap::from([(
                "test".to_owned(),
                Arc::new(ToolStub) as Arc<dyn Tool + Send + Sync>,
            )]),
        )]);
        toolbox.update_tools(tools);

        let tool = toolbox
            .fetch_tool(&QualifiedToolName {
                namespace: Namespace::dummy(),
                name: "test".to_owned(),
            })
            .unwrap();

        let arguments = vec![];
        let output = tool
            .invoke(arguments, TracingContext::dummy())
            .await
            .unwrap();

        assert_eq!(output.text(), "success");
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

    #[test]
    fn fetch_native_tool() {
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

    #[test]
    fn hardcoded_native_tools_are_listed_in_test_namespace() {
        // Given a toolbox that has been notified about about tools
        let toolbox = Toolbox::new();

        // When we list the tools in the `test-beta` namespace
        let tools = toolbox
            .list_tools_in_namespace(&Namespace::new("test-beta").unwrap())
            .unwrap();
        let names = tools.iter().map(ToolDescription::name).collect::<Vec<_>>();

        // Then we expect the tools to be listed
        assert_eq!(names, vec!["add", "saboteur", "subtract"]);
    }

    #[test]
    fn upserted_native_tool_is_listed() {
        // Given a toolbox with a native tool
        let mut toolbox = Toolbox::new();
        toolbox.upsert_native_tool(ConfiguredNativeTool {
            name: NativeToolName::Add,
            namespace: Namespace::dummy(),
        });

        // When we list the tools
        let tools = toolbox
            .list_tools_in_namespace(&Namespace::dummy())
            .unwrap();
        let names = tools.iter().map(ToolDescription::name).collect::<Vec<_>>();

        // Then we expect the tool to be listed
        assert_eq!(names, vec!["add"]);
    }

    #[test]
    fn removed_native_tool_is_not_listed() {
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
        let tools = toolbox
            .list_tools_in_namespace(&Namespace::dummy())
            .unwrap();

        // Then the namespace still exists, but it is empty
        assert!(tools.is_empty());
    }

    #[test]
    fn tools_are_sorted() {
        struct ToolStub {
            name: String,
        }

        impl ToolStub {
            fn new(name: impl Into<String>) -> Self {
                Self { name: name.into() }
            }
        }

        impl ToolDouble for ToolStub {
            fn description(&self) -> ToolDescription {
                ToolDescription::new(self.name.clone(), "I do nothing", Value::Null)
            }
        }
        // Given a toolbox with MCP tools and native tools
        let mut toolbox = Toolbox::new();
        let tools = HashMap::from([(
            Namespace::dummy(),
            HashMap::from([
                (
                    "a".to_owned(),
                    Arc::new(ToolStub::new("a")) as Arc<dyn Tool + Send + Sync>,
                ),
                (
                    "b".to_owned(),
                    Arc::new(ToolStub::new("b")) as Arc<dyn Tool + Send + Sync>,
                ),
            ]),
        )]);
        toolbox.update_tools(tools);

        toolbox.upsert_native_tool(ConfiguredNativeTool {
            name: NativeToolName::Add,
            namespace: Namespace::dummy(),
        });
        toolbox.upsert_native_tool(ConfiguredNativeTool {
            name: NativeToolName::Subtract,
            namespace: Namespace::dummy(),
        });

        // When we list the tools
        let tools = toolbox
            .list_tools_in_namespace(&Namespace::dummy())
            .unwrap();
        let names = tools.iter().map(ToolDescription::name).collect::<Vec<_>>();

        // Then we expect the tool list to be sorted
        assert_eq!(names, vec!["a", "add", "b", "subtract"]);
    }

    #[test]
    fn listing_tools_for_unknown_namespace_returns_error() {
        // Given a toolbox
        let toolbox = Toolbox::new();

        // When we list the tools for an unknown namespace
        let result = toolbox.list_tools_in_namespace(&Namespace::new("unknown").unwrap());

        // Then we expect an error to be returned
        assert!(matches!(result, Err(NamespaceNotFound)));
    }
}
