use std::future::Future;

use aleph_alpha_client::{
    Client, CompletionOutput, How, Prompt, Sampling, Stopping, TaskCompletion,
};

use super::{Completion, CompletionParams, CompletionRequest};

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
        let mut maximum_tokens = None;
        let mut stop_sequences = vec![];

        let sampling = if let Some(params) = &request.params {
            let CompletionParams {
                max_tokens,
                temperature,
                top_k,
                top_p,
                stop,
            } = params;
            maximum_tokens = *max_tokens;
            stop_sequences = stop.iter().map(String::as_str).collect::<Vec<_>>();
            Sampling {
                temperature: *temperature,
                top_k: *top_k,
                top_p: *top_p,
                start_with_one_of: &[],
            }
        } else {
            Sampling::MOST_LIKELY
        };
        let task = TaskCompletion {
            prompt,
            stopping: Stopping {
                maximum_tokens,
                stop_sequences: &stop_sequences,
            },
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
