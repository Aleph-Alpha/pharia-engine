mod actor;
mod client;

pub use actor::{
    ChatEvent, ChatParams, ChatRequest, ChatResponse, Completion, CompletionEvent,
    CompletionParams, CompletionRequest, Distribution, Explanation, ExplanationRequest,
    FinishReason, Granularity, Inference, InferenceApi, Logprob, Logprobs, Message, TextScore,
    TokenUsage,
};

#[cfg(test)]
pub mod tests {
    pub use super::actor::tests::InferenceStub;
}
