use std::future::Future;

use aleph_alpha_client::{
    Client, CompletionOutput, How, Prompt, Sampling, Stopping, TaskCompletion,
};

use super::{Completion, CompletionRequest};

pub trait InferenceClient {
    fn complete_text(
        &mut self,
        request: &CompletionRequest,
        api_token: String,
    ) -> impl Future<Output = anyhow::Result<Completion>> + Send;
}

impl InferenceClient for Client {
    async fn complete_text(
        &mut self,
        request: &CompletionRequest,
        api_token: String,
    ) -> anyhow::Result<Completion> {
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
        let completion_output = self
            .completion(
                &task,
                &request.model,
                &How {
                    api_token: Some(api_token),
                    ..Default::default()
                },
            )
            .await?;
        completion_output.try_into()
    }
}

impl TryFrom<CompletionOutput> for Completion {
    type Error = anyhow::Error;

    fn try_from(completion_output: CompletionOutput) -> anyhow::Result<Self> {
        Ok(Self {
            text: completion_output.completion,
            finish_reason: completion_output.finish_reason.parse()?,
        })
    }
}
