mod engine;
mod provider;
mod wasm;

use async_trait::async_trait;
use serde_json::Value;
use std::future::Future;

pub use provider::SkillProvider;
pub use wasm::WasmRuntime;

use crate::{
    configuration_observer::NamespaceDescriptionError,
    inference::{Completion, CompletionRequest},
};

use super::{actor::ExecuteSkillError, chunking::ChunkRequest, SkillPath};

/// Responsible for loading and executing skills.
pub trait Runtime {
    // We are returning a Future explicitly here instead of using the `async` syntax. This has the
    // following reason: The async syntax is ambiguous with regards to whether or not the Future is
    // `Send`. The Rust compiler figures out the lifetime and `Send`ness of the future implicitly
    // via type inference. Yet this for example can never work across crate bounds, and sometimes
    // hits its limits even within a crate. To give an example:
    //
    // `fn async f() -> i32` could be a shortcut for both `fn f() -> impl Future<Output=i32>` **or**
    // `fn f() -> impl Future<Output=i32> + Send`. It is also ambiguous over lifetime and `Sync`ness
    // of the future, but we do not need these traits here.

    /// Executes a skill and return its result.
    fn run(
        &mut self,
        skill_path: &SkillPath,
        input: Value,
        ctx: Box<dyn CsiForSkills + Send>,
    ) -> impl Future<Output = Result<Value, ExecuteSkillError>> + Send;

    fn upsert_skill(&mut self, skill: SkillPath, tag: Option<String>);

    fn remove_skill(&mut self, skill: &SkillPath);

    fn skills(&self) -> impl Iterator<Item = &SkillPath>;

    fn loaded_skills(&self) -> impl Iterator<Item = &SkillPath>;

    /// The runtime may handle cache invalidation of skills by itself in the future. For now we cut
    /// it a bit of slack and just tell it that a skill might have changed.
    fn invalidate_cached_skill(&mut self, skill_path: &SkillPath) -> bool;

    fn mark_namespace_as_invalid(&mut self, namespace: String, e: NamespaceDescriptionError);

    fn mark_namespace_as_valid(&mut self, namespace: &str);
}

/// Cognitive System Interface (CSI) as consumed by Skill developers. In particular some accidential
/// complexity has been stripped away, by implementations due to removing accidental errors from the
/// interface. It also assumes all authentication and authorization is handled behind the scenes.
/// This is the CSI as passed to user defined code in WASM.
#[async_trait]
pub trait CsiForSkills {
    async fn complete_text(&mut self, request: CompletionRequest) -> Completion;
    async fn chunk(&mut self, request: ChunkRequest) -> Vec<String>;
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use anyhow::anyhow;

    pub struct SaboteurRuntime {
        err_msg: String,
    }

    impl SaboteurRuntime {
        pub fn new(err_msg: String) -> Self {
            Self { err_msg }
        }
    }

    impl Runtime for SaboteurRuntime {
        async fn run(
            &mut self,
            _skill_path: &SkillPath,
            _input: Value,
            _ctx: Box<dyn CsiForSkills + Send>,
        ) -> Result<Value, ExecuteSkillError> {
            Err(ExecuteSkillError::Other(anyhow!(self.err_msg.clone())))
        }

        fn upsert_skill(&mut self, _skill: SkillPath, _tag: Option<String>) {
            panic!("Saboteur runtime does not add skill")
        }

        fn remove_skill(&mut self, _skill: &SkillPath) {
            panic!("Saboteur runtime does not remove skill")
        }

        fn skills(&self) -> impl Iterator<Item = &SkillPath> {
            std::iter::empty()
        }

        fn loaded_skills(&self) -> impl Iterator<Item = &SkillPath> {
            std::iter::empty()
        }

        fn invalidate_cached_skill(&mut self, _skill_path: &SkillPath) -> bool {
            panic!("Saboteur runtime does not drop skills from cache")
        }

        fn mark_namespace_as_invalid(&mut self, _namespace: String, _e: NamespaceDescriptionError) {
            panic!("Saboteur runtime does not add invalid namespace")
        }

        fn mark_namespace_as_valid(&mut self, _namespace: &str) {
            panic!("Saboteur runtime does not remove invalid namespace")
        }
    }
}
