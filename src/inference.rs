mod actor;
mod client;

pub use actor::{CompleteTextParameters, Inference, InferenceApi};

#[cfg(test)]
pub mod tests {
    pub use super::actor::tests::InferenceStub;
}
