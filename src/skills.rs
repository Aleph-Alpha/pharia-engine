mod actor;
mod csi;
mod runtime;
pub use actor::{SkillExecutor, SkillExecutorApi};
pub use runtime::WasmRuntime;

#[cfg(test)]
pub mod tests {
    pub use super::actor::tests::LiarRuntime;
    pub use super::runtime::tests::{RustRuntime, SaboteurRuntime};
}
