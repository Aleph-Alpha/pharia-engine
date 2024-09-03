mod actor;
mod client;

pub use actor::{
    ChunkParams, ChunkRequest, Completion, CompletionParams, CompletionRequest, FinishReason,
    Inference, InferenceApi,
};

#[cfg(test)]
pub mod tests {
    pub use super::actor::tests::InferenceStub;
}
