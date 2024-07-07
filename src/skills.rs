mod actor;
mod runtime;
pub use actor::{SkillExecutor, SkillExecutorApi};

#[cfg(test)]
pub mod tests {
    pub use super::actor::tests::LiarRuntime;
}
