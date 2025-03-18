mod actor;
mod client;

pub use actor::{
    ChatParams, ChatRequest, ChatResponse, Completion, CompletionEvent, CompletionParams,
    CompletionRequest, CompletionStream, CompletionStreamWithErrors, Distribution, Explanation,
    ExplanationRequest, FinishReason, Granularity, Inference, InferenceApi, Logprob, Logprobs,
    Message, TextScore, TokenUsage,
};

#[cfg(test)]
pub mod tests {
    pub use super::actor::tests::InferenceStub;
}
