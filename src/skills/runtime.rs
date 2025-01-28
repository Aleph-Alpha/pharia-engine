mod engine;
mod wasm;

use async_trait::async_trait;
use serde_json::Value;

use crate::{
    csi::ChunkRequest,
    inference::{ChatRequest, ChatResponse, Completion, CompletionRequest},
    language_selection::{Language, SelectLanguageRequest},
    search::{Document, DocumentPath, SearchRequest, SearchResult},
};

pub use self::{
    engine::{Engine, Skill, SupportedVersion},
    wasm::WasmRuntime,
};

/// Cognitive System Interface (CSI) as consumed by Skill developers. In particular some accidental
/// complexity has been stripped away, by implementations due to removing accidental errors from the
/// interface. It also assumes all authentication and authorization is handled behind the scenes.
/// This is the CSI as passed to user defined code in WASM.
#[async_trait]
pub trait CsiForSkills {
    async fn complete(&mut self, requests: Vec<CompletionRequest>) -> Vec<Completion>;
    async fn chunk(&mut self, requests: Vec<ChunkRequest>) -> Vec<Vec<String>>;
    async fn select_language(&mut self, request: SelectLanguageRequest) -> Option<Language>;
    async fn chat(&mut self, requests: Vec<ChatRequest>) -> Vec<ChatResponse>;
    async fn search(&mut self, request: SearchRequest) -> Vec<SearchResult>;
    async fn document_metadata(&mut self, document_paths: Vec<DocumentPath>) -> Vec<Option<Value>>;
    async fn documents(&mut self, document_paths: Vec<DocumentPath>) -> Vec<Document>;
}
