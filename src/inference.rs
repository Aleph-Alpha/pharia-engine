mod actor;
mod client;

pub use actor::{
    ChatParams, ChatRequest, ChatResponse, Completion, CompletionParams, CompletionRequest,
    Distribution, FinishReason, Inference, InferenceApi, Logprob, Logprobs, Message,
};

#[cfg(test)]
pub mod tests {
    pub use super::actor::tests::{AssertConcurrentClient, InferenceStub};
}
