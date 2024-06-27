mod actor;
mod runtime;
pub use actor::{SkillExecutor, SkillExecutorApi};
pub use runtime::WasmRuntime;

#[cfg(test)]
pub mod tests {
    pub use super::runtime::tests::{RustRuntime, SaboteurRuntime};
    pub use super::runtime::Runtime;
}
