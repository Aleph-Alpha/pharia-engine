mod actor;
mod client;

pub use self::{
    actor::{
        ChatEvent, ChatParams, ChatRequest, ChatResponse, Completion, CompletionEvent,
        CompletionParams, CompletionRequest, Distribution, Explanation, ExplanationRequest,
        FinishReason, Granularity, Inference, InferenceApi, InferenceConfig, Logprob, Logprobs,
        Message, TextScore, TokenUsage,
    },
    client::InferenceError,
};

#[cfg(test)]
pub mod tests {
    pub use super::actor::InferenceApiDouble;
    pub use super::actor::tests::InferenceStub;
}
