use std::future::Future;

use aleph_alpha_client::{Client, How, TaskCompletion};
use anyhow::Error;

use super::CompleteTextParameters;

pub trait InferenceClient {
    fn complete_text(
        &mut self,
        params: &CompleteTextParameters,
        api_token: String,
    ) -> impl Future<Output = Result<String, Error>> + Send;
}

impl InferenceClient for Client {
    async fn complete_text(
        &mut self,
        params: &CompleteTextParameters,
        api_token: String,
    ) -> Result<String, Error> {
        let task = TaskCompletion::from_text(&params.prompt, params.max_tokens);
        Ok(self
            .completion(
                &task,
                &params.model,
                &How {
                    api_token: Some(api_token.clone()),
                    ..Default::default()
                },
            )
            .await?
            .completion)
    }
}
