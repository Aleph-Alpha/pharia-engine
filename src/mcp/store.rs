use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use tokio::time::Instant;
use tracing::error;

use crate::{
    mcp::{McpServerUrl, client::McpClient},
    namespace_watcher::Namespace,
    tool::{QualifiedToolName, ToolDescription},
};

/// A cached list of tools from an MCP server.
struct CachedTools {
    tools: Vec<ToolDescription>,
    /// The time when the list of tools was last updated.
    last_checked: Instant,
}

impl CachedTools {
    pub fn now(tools: Vec<ToolDescription>) -> Self {
        Self {
            tools,
            last_checked: Instant::now(),
        }
    }

    pub fn update_last_checked(&mut self) {
        self.last_checked = Instant::now();
    }
}

/// Remembers MCP servers configured for each namespace, as well as the tools provided by each
/// server.
pub struct McpServerStore {
    /// All configured MCP servers grouped by namespace.
    servers: HashMap<Namespace, HashSet<McpServerUrl>>,
    /// Names of tools provided by each MCP server. Tool contains one entry for each unique MCP
    /// server URL.
    tools: HashMap<McpServerUrl, CachedTools>,
}

impl McpServerStore {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            servers: HashMap::new(),
        }
    }

    pub async fn wait_for_next_refresh(
        &mut self,
        update_interval: Duration,
    ) -> Option<McpServerUrl> {
        if let Some((server, cached_tools)) = self.next_in_line_for_refresh() {
            // Sleep until last refresh + update_interval
            let wait_until = cached_tools.last_checked + update_interval;
            tokio::time::sleep_until(wait_until).await;
            // Set the last refresh to now for this server. The correct way would be to update it
            // to the time when the refresh is completed. However, not updating would set the
            // server up for immediate requeue.
            cached_tools.update_last_checked();
            Some(server.clone())
        } else {
            // We sleep here, as there are no servers to refresh. The alternative would be
            // returning directly, but this would lead to a busy loop in the select statement.
            tokio::time::sleep(update_interval).await;
            None
        }
    }

    /// The Mcp server that is up next for refresh.
    ///
    /// Returns None if there are no servers to refresh.
    fn next_in_line_for_refresh(&mut self) -> Option<(&McpServerUrl, &mut CachedTools)> {
        self.tools
            .iter_mut()
            .min_by_key(|(_, tools)| tools.last_checked)
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

    /// Sets the tool list for a given MCP server and reports if there have been changes.
    pub fn update_tools(&mut self, server: McpServerUrl, tools: Vec<ToolDescription>) -> bool {
        let previous = self.tools.get_mut(&server);
        if let Some(previous) = previous {
            let updated = previous.tools != tools;
            // We do not update the time, as it was set when the task was fetched
            previous.tools = tools;
            updated
        } else {
            self.tools.insert(server, CachedTools::now(tools));
            true
        }
    }

    /// Add a new server for a given namespace. Returns if the server is new or already known.
    pub fn upsert(&mut self, namespace: Namespace, server_to_upsert: McpServerUrl) -> bool {
        let new_server = !self.tools.contains_key(&server_to_upsert);
        if new_server {
            // Initialize the server with an empty tool list.
            // Last updated is set to now, as there is a running request pushed for
            // this server.
            self.tools
                .insert(server_to_upsert.clone(), CachedTools::now(vec![]));
        }
        self.servers
            .entry(namespace)
            .or_default()
            .insert(server_to_upsert);
        new_server
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
    pub fn all_tools_by_name(
        &self,
    ) -> impl Iterator<Item = (QualifiedToolName, ToolDescription, McpServerUrl)> + '_ {
        self.servers
            .iter()
            .flat_map(|(namespace, servers)| {
                servers.iter().cloned().map(|s| (namespace.clone(), s))
            })
            .flat_map(|(namespace, server)| {
                self.tools
                    .get(&server)
                    .expect("Every MCP server stored must have a tool list.")
                    .tools
                    .iter()
                    .cloned()
                    .map(move |t| (namespace.clone(), server.clone(), t))
            })
            .map(|(namespace, server, tool)| {
                (
                    QualifiedToolName {
                        namespace,
                        // Currently the tool name used to invoke the tool via CSI is the same as the
                        // name reported by the MCP server, but this may change in the future, to avoid
                        // name collisions
                        name: tool.name().to_owned(),
                    },
                    tool,
                    server,
                )
            })
    }

    /// Fetches the list of tools for the given MCP server. The returned list is sorted, so that the
    /// list can be compared trivially later.
    pub async fn fetch_tools_for(
        server: &McpServerUrl,
        client: &impl McpClient,
    ) -> Result<Vec<ToolDescription>, anyhow::Error> {
        client
            .list_tools(server)
            .await
            .inspect_err(|e| {
                error!(
                    target: "pharia-kernel::mcp",
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

#[cfg(test)]
pub mod tests {
    use std::time::Duration;

    use super::*;

    impl McpServerStore {
        fn with_tool(mut self, mcp_server_url: McpServerUrl, tools: CachedTools) -> Self {
            self.tools.insert(mcp_server_url, tools);
            self
        }
    }

    impl CachedTools {
        fn last_checked(last_checked: Instant) -> Self {
            Self {
                tools: vec![],
                last_checked,
            }
        }
    }

    #[tokio::test]
    async fn next_refresh_without_any_servers() {
        let mut store = McpServerStore::new();
        assert!(store.next_in_line_for_refresh().is_none());
    }

    #[tokio::test]
    async fn oldest_server_is_returned() {
        // Given a store with two servers that have been checked at different times
        let first_tools = CachedTools::last_checked(Instant::now() - Duration::from_secs(1));
        let second_tools = CachedTools::last_checked(Instant::now() - Duration::from_secs(2));
        let mut store = McpServerStore::new()
            .with_tool(McpServerUrl::new("http://first.com/mcp"), first_tools)
            .with_tool(McpServerUrl::new("http://second.com/mcp"), second_tools);

        // When we call next_refresh
        let next = store.next_in_line_for_refresh().unwrap();

        // Then the oldest server is returned
        assert_eq!(next.0, &McpServerUrl::new("http://second.com/mcp"));
    }

    #[tokio::test]
    async fn overdue_server_is_returned_directly() {
        // Given a store with a server that is overdue for refresh
        let first_tools = CachedTools::last_checked(Instant::now() - Duration::from_secs(2));
        let mut store =
            McpServerStore::new().with_tool(McpServerUrl::new("http://first.com/mcp"), first_tools);
        let update_interval = Duration::from_secs(1);

        // When we wait for the next refresh
        let next = tokio::time::timeout(
            Duration::from_millis(10),
            store.wait_for_next_refresh(update_interval),
        )
        .await
        .unwrap()
        .unwrap();

        // Then the server is returned directly
        assert_eq!(next, McpServerUrl::new("http://first.com/mcp"));
    }

    #[tokio::test]
    async fn recently_checked_server_is_not_returned_directly() {
        // Given a store with a server that has been checked just now
        let first_tools = CachedTools::last_checked(Instant::now());
        let mut store =
            McpServerStore::new().with_tool(McpServerUrl::new("http://first.com/mcp"), first_tools);
        let update_interval = Duration::from_secs(1);

        // When we wait for the next refresh
        let next = tokio::time::timeout(
            Duration::from_millis(10),
            store.wait_for_next_refresh(update_interval),
        )
        .await;

        // Then the server is not returned
        assert!(next.is_err());
    }

    #[tokio::test]
    async fn update_tools_reports_changes() {
        // Given a store with a server that has never been checked
        let mut store = McpServerStore::new();

        // When we notify the store about an update
        let server = McpServerUrl::new("http://first.com/mcp");
        let tools = vec![
            ToolDescription::with_name("tool1"),
            ToolDescription::with_name("tool2"),
        ];
        let updated = store.update_tools(server, tools);

        // Then a change is reported
        assert!(updated);
    }

    #[tokio::test]
    async fn changes_to_existing_server_are_reported() {
        // Given a store with a server that has a list of tools
        let mut store = McpServerStore::new();
        let server = McpServerUrl::new("http://first.com/mcp");
        let tools = vec![
            ToolDescription::with_name("tool1"),
            ToolDescription::with_name("tool2"),
        ];
        store.update_tools(server.clone(), tools);

        // When we notify the store about an update
        let tools = vec![
            ToolDescription::with_name("tool1"),
            ToolDescription::with_name("tool3"),
        ];
        let updated = store.update_tools(server, tools);

        // Then a change is reported
        assert!(updated);
    }

    #[tokio::test]
    async fn no_changes_means_no_update() {
        // Given a store with a server that has a list of tools
        let mut store = McpServerStore::new();
        let server = McpServerUrl::new("http://first.com/mcp");
        let tools = vec![
            ToolDescription::with_name("tool1"),
            ToolDescription::with_name("tool2"),
        ];
        store.update_tools(server.clone(), tools.clone());

        // When we notify the store about the same tools
        let updated = store.update_tools(server, tools);

        // Then no change is reported
        assert!(!updated);
    }

    #[tokio::test]
    async fn upsert_returns_if_server_is_new() {
        // Given a store that does not know about a server
        let mut store = McpServerStore::new();

        // When upserting the server
        let server = McpServerUrl::new("http://first.com/mcp");
        let new_server = store.upsert(Namespace::new("first").unwrap(), server.clone());

        // Then the server is considered as new
        assert!(new_server);
    }

    #[tokio::test]
    async fn known_server_is_not_considered_new() {
        // Given a store that knows about a server
        let mut store = McpServerStore::new();
        let server = McpServerUrl::new("http://first.com/mcp");
        store.upsert(Namespace::new("first").unwrap(), server.clone());

        // When upserting the server for a different namespace
        let new_server = store.upsert(Namespace::new("second").unwrap(), server.clone());

        // Then the server is not considered as new
        assert!(!new_server);
    }
}
