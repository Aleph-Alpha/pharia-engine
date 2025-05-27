mod actor;
mod client;

pub use self::actor::{Argument, InvokeRequest, Tool, ToolApi, ToolError};

#[cfg(test)]
pub mod tests {
    pub use super::actor::tests::ToolDouble;
}
