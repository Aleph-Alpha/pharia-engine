mod actor;
mod client;

pub use self::{
    actor::{
        ChatEvent, ChatParams, ChatRequest, ChatResponse, Completion, CompletionEvent,
        CompletionParams, CompletionRequest, Distribution, Explanation, ExplanationRequest,
        FinishReason, Granularity, Inference, InferenceApi, Logprob, Logprobs, Message, TextScore,
        TokenUsage,
    },
    client::{ClientConfig, ClientWithAuth, InferenceError},
};

#[cfg(test)]
pub mod tests {
    pub use super::actor::InferenceApiDouble;
    pub use super::actor::tests::InferenceStub;
}
