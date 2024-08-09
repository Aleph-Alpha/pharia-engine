mod actor;
mod client;

pub use actor::{CompletionParams, CompletionRequest, Inference, InferenceApi};

#[cfg(test)]
pub mod tests {
    pub use super::actor::tests::InferenceStub;
}
