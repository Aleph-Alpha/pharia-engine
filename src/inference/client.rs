use std::future::Future;

use aleph_alpha_client::{Client, How, Prompt, Sampling, Stopping, TaskCompletion};
use anyhow::Error;

use super::CompletionRequest;

pub trait InferenceClient {
    fn complete_text(
        &mut self,
        request: &CompletionRequest,
        api_token: String,
    ) -> impl Future<Output = Result<String, Error>> + Send;
}

impl InferenceClient for Client {
    async fn complete_text(
        &mut self,
        request: &CompletionRequest,
        api_token: String,
    ) -> Result<String, Error> {
        let prompt = Prompt::from_text(&request.prompt);
        let mut stopping = Stopping::NO_TOKEN_LIMIT;

        let sampling = if let Some(params) = &request.params {
            stopping.maximum_tokens = params.max_tokens;
            Sampling {
                temperature: params.temperature,
                top_k: params.top_k,
                top_p: params.top_p,
                start_with_one_of: &[],
            }
        } else {
            Sampling::MOST_LIKELY
        };
        let task = TaskCompletion {
            prompt,
            stopping,
            sampling,
        };
        Ok(self
            .completion(
                &task,
                &request.model,
                &How {
                    api_token: Some(api_token),
                    ..Default::default()
                },
            )
            .await?
            .completion)
    }
}
