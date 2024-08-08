mod actor;
mod client;

pub use actor::{CompletionRequest, Inference, InferenceApi};

#[cfg(test)]
pub mod tests {
    pub use super::actor::tests::InferenceStub;
}
