mod engine;
mod provider;
mod wasm;

use async_trait::async_trait;
use serde_json::Value;
use std::future::Future;

pub use provider::OperatorProvider;
pub use wasm::WasmRuntime;

use crate::inference::{Completion, CompletionRequest};

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
        skill: &str,
        input: Value,
        ctx: Box<dyn Csi + Send>,
    ) -> impl Future<Output = anyhow::Result<Value>> + Send;

    fn skills(&self) -> impl Iterator<Item = String>;

    /// The runtime may handle cache invalidation of skills by itself in the future. For now we cut
    /// it a bit of slack and just tell it that a skill might have changed.
    fn invalidate_cached_skill(&mut self, skill: &str) -> bool;
}

#[async_trait]
pub trait Csi {
    async fn complete_text(&mut self, request: CompletionRequest) -> Completion;
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
            _skill: &str,
            _input: Value,
            _ctx: Box<dyn Csi + Send>,
        ) -> anyhow::Result<Value> {
            Err(anyhow!(self.err_msg.clone()))
        }

        fn skills(&self) -> impl Iterator<Item = String> {
            std::iter::empty()
        }
        fn invalidate_cached_skill(&mut self, _skill: &str) -> bool {
            panic!("SaboteurRuntime does not drop skills from cache")
        }
    }
}
