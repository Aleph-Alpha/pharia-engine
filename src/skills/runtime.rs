mod provider;
mod wasm;

use anyhow::Error;
use async_trait::async_trait;
use std::future::Future;

pub use wasm::WasmRuntime;

use crate::inference::CompleteTextParameters;

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
        name: String,
        ctx: Box<dyn Csi + Send>,
    ) -> impl Future<Output = Result<String, Error>> + Send;

    fn skills(&self) -> impl Iterator<Item = &str>;

    /// The runtime may handle cache invalidation of skills by itself in the future. For now we cut
    /// it a bit of slack and just tell it that a skill might have changed.
    fn invalidate_cached_skill(&mut self, skill: &str) -> bool;
}

#[async_trait]
pub trait Csi {
    async fn complete_text(&mut self, params: CompleteTextParameters) -> String;
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
            _name: String,
            _ctx: Box<dyn Csi + Send>,
        ) -> Result<String, Error> {
            Err(anyhow!(self.err_msg.clone()))
        }

        fn skills(&self) -> impl Iterator<Item = &str> {
            std::iter::empty()
        }
        fn invalidate_cached_skill(&mut self, _skill: &str) -> bool {
            panic!("SaboteurRuntime does not drop skills from cache")
        }
    }
}
