mod mcp;
mod skill;

use std::process::Command;

pub use mcp::given_mcp_server;
pub use skill::{
    given_chat_stream_skill, given_complete_stream_skill, given_invalid_output_skill,
    given_python_skill_greet_v0_2, given_python_skill_greet_v0_3, given_rust_skill_chat,
    given_rust_skill_doc_metadata, given_rust_skill_explain, given_rust_skill_greet_v0_2,
    given_rust_skill_greet_v0_3, given_rust_skill_search, given_skill_infinite_streaming,
    given_skill_tool_invocation, given_streaming_output_skill,
};

fn assert_uv_installed() {
    let status = Command::new("uv")
        .args(["--version"])
        .status()
        .expect("UV must be available for testing with Python Skills. Please install it");

    assert!(
        status.success(),
        "uv command exited with an error. Make sure it works in order to test Python Skills."
    );
}
