mod actor;
mod client;

pub use actor::{
    ChatParams, ChatRequest, ChatResponse, Completion, CompletionParams, CompletionRequest,
    FinishReason, Inference, InferenceApi, Message, Role,
};

#[cfg(test)]
pub mod tests {
    pub use super::actor::tests::{AssertConcurrentClient, InferenceStub};
}
