mod actor;
mod client;
mod openai;

pub use self::{
    actor::{
        ChatEvent, ChatParams, ChatRequest, ChatResponse, Completion, CompletionEvent,
        CompletionParams, CompletionRequest, Distribution, Explanation, ExplanationRequest,
        FinishReason, Function, Granularity, Inference, InferenceApi, InferenceConfig,
        InferenceNotConfigured, InferenceSender, Logprob, Logprobs, Message, ResponseMessage,
        TextScore, TokenUsage, ToolCall, ToolChoice,
    },
    client::InferenceError,
};

#[cfg(test)]
pub mod tests {
    pub use super::actor::{InferenceApiDouble, tests::InferenceStub};
}
