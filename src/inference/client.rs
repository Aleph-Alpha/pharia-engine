use std::future::Future;

use aleph_alpha_client::{Client, How, TaskCompletion};

use super::CompleteTextParameters;

pub trait InferenceClient {
    fn complete_text(
        &self,
        params: &CompleteTextParameters,
        api_token: String,
    ) -> impl Future<Output = String> + Send;
}

impl InferenceClient for Client {
    async fn complete_text(&self, params: &CompleteTextParameters, api_token: String) -> String {
        let task = TaskCompletion::from_text(&params.prompt, params.max_tokens);
        self.completion(
            &task,
            &params.model,
            &How {
                api_token: Some(api_token.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .completion
    }
}
