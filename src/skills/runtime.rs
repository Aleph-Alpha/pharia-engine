mod engine;
mod wasm;

use async_trait::async_trait;
use serde_json::Value;
use std::future::Future;

use crate::{
    csi::ChunkRequest,
    inference::{Completion, CompletionRequest},
    language_selection::{Language, SelectLanguageRequest},
};

use super::{actor::ExecuteSkillError, SkillPath};

pub use self::{
    engine::{Engine, Skill},
    wasm::WasmRuntime,
};

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
        &self,
        skill_path: &SkillPath,
        input: Value,
        ctx: Box<dyn CsiForSkills + Send>,
    ) -> impl Future<Output = Result<Value, ExecuteSkillError>> + Send;
}

/// Cognitive System Interface (CSI) as consumed by Skill developers. In particular some accidential
/// complexity has been stripped away, by implementations due to removing accidental errors from the
/// interface. It also assumes all authentication and authorization is handled behind the scenes.
/// This is the CSI as passed to user defined code in WASM.
#[async_trait]
pub trait CsiForSkills {
    async fn complete_text(&mut self, request: CompletionRequest) -> Completion;
    async fn complete_all(&mut self, requests: Vec<CompletionRequest>) -> Vec<Completion>;
    async fn chunk(&mut self, request: ChunkRequest) -> Vec<String>;
    async fn select_language(&mut self, request: SelectLanguageRequest) -> Option<Language>;
}
