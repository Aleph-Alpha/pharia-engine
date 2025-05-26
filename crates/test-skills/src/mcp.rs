use anyhow::{Ok, anyhow};
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::json;
use std::{
    process::{Child, Command},
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use tempfile::TempDir;

use crate::assert_uv_installed;

const MCP_SERVER_ADDRESS: &str = "http://localhost:8000/mcp";

/// A mutex to a weak lock allows us to never have more then one instance of the MCP server
/// running, but don't keep it alive if no one is using it.
static MCP_SERVER: Mutex<Weak<Mcp>> = Mutex::new(Weak::new());

/// A handle to a running MCP server.
pub async fn given_mcp_server() -> Arc<Mcp> {
    let mcp = {
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
    };

    mcp.wait_for_ready().await;
    mcp
}

/// A guard that ensures the MCP server is running.
///
/// When dropped, the MCP server is shutdown, as dropping the Child shuts down the process.
pub struct Mcp {
    child: Child,
    _dir: TempDir,
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
            Self {
                child,
                _dir: temp_dir,
            }
        }
    }

    pub fn address(&self) -> String {
        MCP_SERVER_ADDRESS.to_owned()
    }

    async fn wait_for_ready(&self) {
        for _ in 0..50 {
            if self.ping().await.is_ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("MCP server did not boot up after 5 seconds");
    }

    pub async fn ping(&self) -> anyhow::Result<()> {
        let client = Client::new();
        let body = json!({
          "jsonrpc": "2.0",
          "id": 1,
          "method": "ping"
        });
        let response = client
            .post(MCP_SERVER_ADDRESS)
            // MCP server want exactly these two headers, even a wildcard is not accepted
            .header("accept", "application/json,text/event-stream")
            .json(&body)
            .send()
            .await?;
        let mut stream = response.bytes_stream();
        drop(
            stream
                .next()
                .await
                .ok_or_else(|| anyhow!("No item received."))??,
        );
        Ok(())
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        // The server itself is only a child process of `uv`. Therefore, we can not kill
        // the child process, because it wouldn't necessarily kill the server.
        // Therefore, we give a graceful shutdown signal to the server.
        let pid = self.child.id();
        Command::new("kill")
            .arg("-15")
            .arg(pid.to_string())
            .spawn()
            .unwrap()
            .wait()
            .unwrap();

        // Wait for the child process to exit
        self.child.wait().unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mcp_server_pingable() {
        let _mcp = given_mcp_server().await;
    }
}
