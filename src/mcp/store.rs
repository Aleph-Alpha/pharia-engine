use std::collections::{HashMap, HashSet};

use crate::{mcp::McpServerUrl, namespace_watcher::Namespace};

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

    pub async fn upsert(&mut self, namespace: Namespace, server_to_upsert: McpServerUrl) {
        if self.tools.get(&server_to_upsert).is_none() {
            // If the server is new, initialize its tool list.
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
