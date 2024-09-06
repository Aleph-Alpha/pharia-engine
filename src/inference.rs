mod actor;
mod client;

pub use actor::{
    Completion, CompletionParams, CompletionRequest, FinishReason, Inference, InferenceApi,
};

#[cfg(test)]
pub mod tests {
    pub use super::actor::tests::InferenceStub;
}
