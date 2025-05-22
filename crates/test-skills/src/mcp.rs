use std::{
    process::{Child, Command},
    sync::{Arc, Mutex, Weak},
};

use crate::assert_uv_installed;

/// A mutex to a weak lock allows us to never have more then one instance of the MCP server
/// running, but don't keep it alive if no one is using it.
static MCP_SERVER: Mutex<Weak<Mcp>> = Mutex::new(Weak::new());

/// A handle to a running MCP server.
pub fn given_mcp_server() -> Arc<Mcp> {
    let mut guard = MCP_SERVER.lock().unwrap();

    if let Some(mcp) = guard.upgrade() {
        // The MCP server is already running
        mcp
    } else {
        // Start the MCP server
        let mcp = Mcp::new();
        let mcp = Arc::new(mcp);
        *guard = Arc::downgrade(&mcp);
        mcp
    }
}

/// A guard that ensures the MCP server is running.
///
/// When dropped, the MCP server is shutdown, as dropping the Child shuts down the process.
pub struct Mcp {
    _child: Child,
}

const PYTHON_SRC: &str = include_str!("mcp_server.py");

impl Mcp {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        assert_uv_installed();

        // Only a temp file did not make the file executable, so we do a temp directory
        let temp_dir = tempfile::tempdir().unwrap();
        let temp_file_path = temp_dir.path().join("mcp_server.py");
        std::fs::write(&temp_file_path, PYTHON_SRC).unwrap();

        let mut child = Command::new("uv")
            .args(["run", temp_file_path.to_str().unwrap()])
            .spawn()
            .unwrap();

        // Give it some time before checking if it exited with an error
        std::thread::sleep(std::time::Duration::from_millis(10));

        if child.try_wait().unwrap().is_some() {
            panic!("MCP server failed to start");
        } else {
            Self { _child: child }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_up_and_shutdown_mcp_server() {
        drop(Mcp::new());
    }
}
