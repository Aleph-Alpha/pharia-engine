use crate::{inference::{Completion, CompletionRequest, InferenceApi}, tokenizers::TokenizersApi};

/// Collection of api handles to the actors used to implement the Cognitive System Interface (CSI)
/// 
/// For now this is just a collection of all the APIs without providing logic on its own
#[derive(Clone)]
pub struct CsiApis {
    /// We use the inference Api to complete text
    pub inference: InferenceApi,
    pub tokenizers: TokenizersApi,
}

/// Cognitive Sytem Interface (CSI) as consumed internally by Pharia Kernel, before the CSI is
/// passed to the end user in Skill code we further strip away some of the accidential complexity.
/// See its sibling trait `CsiForSkills`.
pub trait Csi {

    async fn complete_text(&mut self, auth: String, request: CompletionRequest) -> Result<Completion, anyhow::Error>;
}

impl Csi for CsiApis {
    async fn complete_text(&mut self, auth: String, request: CompletionRequest) -> Result<Completion, anyhow::Error> {
        self.inference.complete_text(request, auth).await
    }
}

#[cfg(test)]
pub mod tests {
    use tokio::sync::mpsc;

    use crate::{inference::InferenceApi, tokenizers::TokenizersApi};

    use super::CsiApis;


    pub fn dummy_csi_apis() -> CsiApis {
        let (send, _recv) = mpsc::channel(1);
        let inference = InferenceApi::new(send);

        let (send, _recv) = mpsc::channel(1);
        let tokenizers = TokenizersApi::new(send);

        CsiApis { inference, tokenizers }
    }
}