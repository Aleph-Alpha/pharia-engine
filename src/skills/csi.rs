use crate::inference::{CompleteTextParameters, InferenceApi};

use async_trait::async_trait;

#[async_trait]
pub trait Csi {
    async fn complete_text(
        &mut self,
        params: CompleteTextParameters,
    ) -> Result<String, anyhow::Error>;
}

#[async_trait]
impl Csi for SkillInvocationCtx {
    async fn complete_text(
        &mut self,
        params: CompleteTextParameters,
    ) -> Result<String, anyhow::Error> {
        self.inference_api
            .complete_text(params, self.api_token.clone())
            .await
    }
}

pub struct SkillInvocationCtx {
    inference_api: InferenceApi,
    api_token: String,
}

impl SkillInvocationCtx {
    pub fn new(inference_api: InferenceApi, api_token: String) -> Self {
        SkillInvocationCtx {
            inference_api,
            api_token,
        }
    }
}
