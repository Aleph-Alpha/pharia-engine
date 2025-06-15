use std::collections::{HashMap, HashSet};

use tracing::error;

use crate::{
    mcp::{McpClient, McpServerUrl},
    namespace_watcher::Namespace,
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

    pub async fn upsert(
        &mut self,
        namespace: Namespace,
        server_to_upsert: McpServerUrl,
        client: &impl McpClient,
    ) {
        if self.tools.get(&server_to_upsert).is_none() {
            // If the server is new, initialize its tool list.
            match client.list_tools(&server_to_upsert).await {
                Ok(tools) => {
                    self.tools.insert(server_to_upsert.clone(), tools);
                }
                Err(e) => {
                    // If we cannot fetch the tools, we still need to keep track of the server. We
                    // will provide only an empty tool list. If the error is temporary we will
                    // eventually be able to fetch the tools, using our regular update.
                    error!(
                        target: "pharia_kernel::mcp",
                        "Failed to fetch tools for server: {}\n caused by: {e:#}",
                        server_to_upsert.0
                    );
                    self.tools.insert(server_to_upsert.clone(), Vec::new());
                }
            }

            self.tools.insert(server_to_upsert.clone(), Vec::new());
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
}
