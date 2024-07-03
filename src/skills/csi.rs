use crate::inference::{CompleteTextParameters, InferenceApi};

use anyhow::Error;
use async_trait::async_trait;
use tokio::sync::oneshot;

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
    /// This is used to send any runtime error (as opposed to logic error) back to the actor, so it
    /// can drop the future invoking the skill, and report the error appropriatly to user and
    /// operator.
    send_rt_err: oneshot::Sender<Error>,
    inference_api: InferenceApi,
    api_token: String,
}

impl SkillInvocationCtx {
    pub fn new(
        send_rt_err: oneshot::Sender<Error>,
        inference_api: InferenceApi,
        api_token: String,
    ) -> Self {
        SkillInvocationCtx {
            send_rt_err,
            inference_api,
            api_token,
        }
    }
}
