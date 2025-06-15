use std::collections::{HashMap, HashSet};

use tracing::error;

use crate::{
    mcp::{McpServerUrl, client::McpClient},
    namespace_watcher::Namespace,
    tool::QualifiedToolName,
};

/// Remembers MCP servers configured for each namespace, as well as the tools provided by each
/// server.
pub struct McpServerStore {
    /// All configuerd MCP servers grouped by namespace.
    servers: HashMap<Namespace, HashSet<McpServerUrl>>,
    /// Names of tools provided by each MCP server. Tool contains one entry for each unique MCP
    /// server URL.
    tools: HashMap<McpServerUrl, Vec<String>>,
}

impl McpServerStore {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            servers: HashMap::new(),
        }
    }

    pub fn list_in_namespace(
        &self,
        namespace: &Namespace,
    ) -> impl Iterator<Item = McpServerUrl> + '_ {
        self.servers
            .get(namespace)
            .cloned()
            .unwrap_or_default()
            .into_iter()
    }

    /// Updates the tool list for all configured MCP servers. `true` if the list of tools has
    /// changed.
    pub async fn update_tool_list(&mut self, client: &impl McpClient) -> bool {
        let mut any_changes_so_far = false;
        for (server, tools_known) in self.tools.iter_mut() {
            let Ok(up_to_date_tools) = Self::fetch_tools_for(server, client).await else {
                // If we cannot fetch the tools, we will not update the list. We will try again
                // later.
                continue;
            };
            if up_to_date_tools != *tools_known {
                // If the tool list has changed, we update it.
                *tools_known = up_to_date_tools;
                any_changes_so_far = true;
            }
        }
        any_changes_so_far
    }

    pub async fn upsert(
        &mut self,
        namespace: Namespace,
        server_to_upsert: McpServerUrl,
        client: &impl McpClient,
    ) {
        if self.tools.get(&server_to_upsert).is_none() {
            // If the server is new, initialize its tool list.
            let tools = if let Ok(tools) = Self::fetch_tools_for(&server_to_upsert, client).await {
                tools
            } else {
                // If we cannot fetch the tools, we still need to keep track of the server. We will
                // provide only an empty tool list. If the error is temporary we will eventually be
                // able to fetch the tools, using our regular update.
                Vec::new()
            };
            self.tools.insert(server_to_upsert.clone(), tools);
        }
        self.servers
            .entry(namespace)
            .or_default()
            .insert(server_to_upsert);
    }

    pub fn remove(&mut self, namespace: Namespace, server_to_remove: McpServerUrl) {
        if let Some(servers) = self.servers.get_mut(&namespace) {
            servers.remove(&server_to_remove);
            if servers.is_empty() {
                self.servers.remove(&namespace);
            }
        }
        if self
            .servers
            .values()
            .all(|servers_in_namespace| !servers_in_namespace.contains(&server_to_remove))
        {
            // If the server is no longer used in any namespace, we do not need to keep track of the
            // tools it provides.
            self.tools.remove(&server_to_remove);
        }
    }

    /// A complete list of all tools across all namespaces indexed by their qualified name.
    pub fn all_tools_by_name(&self) -> impl Iterator<Item = (QualifiedToolName, McpToolDesc)> + '_ {
        self.servers
            .iter()
            .flat_map(|(namespace, servers)| {
                servers.iter().cloned().map(|s| (namespace.clone(), s))
            })
            .flat_map(|(namespace, server)| {
                self.tools
                    .get(&server)
                    .expect("Every MCP server stored must have a tool list.")
                    .iter()
                    .cloned()
                    .map(move |t| (namespace.clone(), server.clone(), t))
            })
            .map(|(namespace, server, tool_name)| {
                (
                    QualifiedToolName {
                        namespace,
                        // Currently the tool name used to invoke the tool via CSI is the same as the
                        // name reported by the MCP server, but this may change in the future, to avoid
                        // name collisions
                        name: tool_name.clone(),
                    },
                    McpToolDesc {
                        name: tool_name,
                        server,
                    },
                )
            })
    }

    /// Fetches the list of tools for the given MCP server. The returned list is sorted, so that the
    /// list can be compared trivially later.
    async fn fetch_tools_for(
        server: &McpServerUrl,
        client: &impl McpClient,
    ) -> Result<Vec<String>, anyhow::Error> {
        client
            .list_tools(&server)
            .await
            .inspect_err(|e| {
                error!(
                    target: "pharia_kernel::mcp",
                    "Failed to fetch tools for server: {}\n caused by: {e:#}",
                    server.0
                );
            })
            .map(|mut list| {
                list.sort();
                list
            })
    }
}

/// Descripes an MCP tool, it should hold all the information needed to connect and invoke the tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpToolDesc {
    /// The name of the tool, as reported by the MCP server.
    pub name: String,
    /// The URL of the MCP server providing the tool.
    pub server: McpServerUrl,
}
