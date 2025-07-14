mod actor;
mod client;

pub use self::{
    actor::{
        ChatEvent, ChatParams, ChatRequest, ChatResponse, Completion, CompletionEvent,
        CompletionParams, CompletionRequest, Distribution, Explanation, ExplanationRequest,
        FinishReason, Granularity, Inference, InferenceApi, InferenceSender, Logprob, Logprobs,
        Message, TextScore, TokenUsage,
    },
    client::InferenceError,
};

#[cfg(test)]
pub mod tests {
    pub use super::actor::{InferenceApiDouble, tests::InferenceStub};
}
