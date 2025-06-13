use std::collections::{HashMap, HashSet};

use crate::{mcp::McpServerUrl, namespace_watcher::Namespace};

pub struct McpServerStore {
    urls: HashMap<Namespace, HashSet<McpServerUrl>>,
}

impl McpServerStore {
    pub fn new() -> Self {
        Self {
            urls: HashMap::new(),
        }
    }

    pub fn list_in_namespace(
        &self,
        namespace: &Namespace,
    ) -> impl Iterator<Item = McpServerUrl> + '_ {
        self.urls
            .get(namespace)
            .cloned()
            .unwrap_or_default()
            .into_iter()
    }

    pub fn upsert(&mut self, namespace: Namespace, url: McpServerUrl) {
        self.urls.entry(namespace).or_default().insert(url);
    }

    pub fn remove(&mut self, namespace: Namespace, url: McpServerUrl) {
        if let Some(servers) = self.urls.get_mut(&namespace) {
            servers.remove(&url);
            if servers.is_empty() {
                self.urls.remove(&namespace);
            }
        }
    }
}
