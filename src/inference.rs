mod actor;
mod client;

pub use actor::{
    ChatParams, ChatRequest, ChatResponse, Completion, CompletionParams, CompletionRequest,
    Distribution, ExplainRequest, Explanation, FinishReason, Granularity, Inference, InferenceApi,
    Logprob, Logprobs, Message, TextScore, TokenUsage,
};

#[cfg(test)]
pub mod tests {
    pub use super::actor::tests::{AssertConcurrentClient, InferenceStub};
}
