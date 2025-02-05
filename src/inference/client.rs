use std::{
    future::Future,
    time::{Duration, SystemTime},
};

use aleph_alpha_client::{
    ChatSampling, Client, CompletionOutput, How, Prompt, Sampling, Stopping, TaskChat,
    TaskCompletion,
};
use retry_policies::{policies::ExponentialBackoff, RetryDecision, RetryPolicy};
use tracing::{error, warn};

use thiserror::Error;

use super::{ChatRequest, ChatResponse, Completion, CompletionParams, CompletionRequest, Message};

pub trait InferenceClient: Send + Sync + 'static {
    fn complete_text(
        &self,
        request: &CompletionRequest,
        api_token: String,
    ) -> impl Future<Output = Result<Completion, InferenceClientError>> + Send;
    fn chat(
        &self,
        request: &ChatRequest,
        api_token: String,
    ) -> impl Future<Output = Result<ChatResponse, InferenceClientError>> + Send;
}

impl InferenceClient for Client {
    async fn chat(
        &self,
        request: &ChatRequest,
        api_token: String,
    ) -> Result<ChatResponse, InferenceClientError> {
        let task = request.into();
        let how = How {
            api_token: Some(api_token),
            ..Default::default()
        };
        let chat_result = retry(|| self.chat(&task, &request.model, &how)).await;
        match chat_result {
            Ok(chat_output) => chat_output.try_into().map_err(InferenceClientError::Other),
            Err(e) => Err(e.into()),
        }
    }

    async fn complete_text(
        &self,
        request: &CompletionRequest,
        api_token: String,
    ) -> Result<Completion, InferenceClientError> {
        let CompletionRequest {
            model,
            prompt,
            params:
                CompletionParams {
                    return_special_tokens,
                    max_tokens,
                    temperature,
                    top_k,
                    top_p,
                    stop,
                    frequency_penalty,
                    presence_penalty,
                },
        } = &request;

        let task = TaskCompletion {
            prompt: Prompt::from_text(prompt),
            stopping: Stopping {
                maximum_tokens: *max_tokens,
                stop_sequences: &stop.iter().map(String::as_str).collect::<Vec<_>>(),
            },
            sampling: Sampling {
                temperature: *temperature,
                top_k: *top_k,
                top_p: *top_p,
                frequency_penalty: *frequency_penalty,
                presence_penalty: *presence_penalty,
            },
            special_tokens: *return_special_tokens,
        };
        let how = How {
            api_token: Some(api_token),
            ..Default::default()
        };

        let completion_result = retry(|| self.completion(&task, model, &how)).await;
        match completion_result {
            Ok(completion_output) => completion_output
                .try_into()
                .map_err(InferenceClientError::Other),
            Err(e) => Err(e.into()),
        }
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

impl From<aleph_alpha_client::Message<'_>> for Message {
    fn from(message: aleph_alpha_client::Message<'_>) -> Self {
        Message::new(message.role, message.content)
    }
}

impl<'a> From<&'a Message> for aleph_alpha_client::Message<'a> {
    fn from(message: &'a Message) -> Self {
        aleph_alpha_client::Message {
            role: message.role.to_string().into(),
            content: (&message.content).into(),
        }
    }
}

impl TryFrom<aleph_alpha_client::ChatOutput> for ChatResponse {
    type Error = anyhow::Error;

    fn try_from(chat_output: aleph_alpha_client::ChatOutput) -> anyhow::Result<Self> {
        Ok(ChatResponse {
            message: chat_output.message.into(),
            finish_reason: chat_output.finish_reason.parse()?,
        })
    }
}

impl<'a> From<&'a ChatRequest> for TaskChat<'a> {
    fn from(request: &'a ChatRequest) -> Self {
        TaskChat {
            messages: request
                .messages
                .iter()
                .map(aleph_alpha_client::Message::from)
                .collect(),
            stopping: Stopping {
                maximum_tokens: request.params.max_tokens,
                stop_sequences: &[],
            },
            sampling: ChatSampling {
                temperature: request.params.temperature,
                top_p: request.params.top_p,
                frequency_penalty: request.params.frequency_penalty,
                presence_penalty: request.params.presence_penalty,
            },
        }
    }
}

async fn retry<T, F, Fut>(mut f: F) -> Result<T, aleph_alpha_client::Error>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, aleph_alpha_client::Error>>,
{
    // Retry with backoff for a total of 5 times.
    let backoff = ExponentialBackoff::builder().build_with_max_retries(5);
    let request_start_time = SystemTime::now();
    let mut n_past_retries = 0;

    loop {
        match f().await {
            Ok(value) => return Ok(value),
            Err(
                e @ (aleph_alpha_client::Error::Busy
                | aleph_alpha_client::Error::Unavailable
                | aleph_alpha_client::Error::TooManyRequests),
            ) => match backoff.should_retry(request_start_time, n_past_retries) {
                RetryDecision::Retry { execute_after } => {
                    let duration = execute_after
                        .duration_since(SystemTime::now())
                        .unwrap_or_else(|_| Duration::default());
                    warn!("Retrying operation: {e} attempt #{n_past_retries}. Sleeping {duration:?} before the next attempt");
                    tokio::time::sleep(duration).await;
                    n_past_retries += 1;
                }
                RetryDecision::DoNotRetry => {
                    error!("Error after all retries: {e}");
                    return Err(e);
                }
            },
            Err(
                e @ (aleph_alpha_client::Error::Http { .. }
                | aleph_alpha_client::Error::Other(_)
                | aleph_alpha_client::Error::ClientTimeout(_)
                | aleph_alpha_client::Error::InvalidStream { .. }
                | aleph_alpha_client::Error::InvalidTokenizer { .. }),
            ) => {
                error!("Unrecoverable inference error: {e}");
                return Err(e);
            }
        }
    }
}

#[derive(Error, Debug)]
pub enum InferenceClientError {
    #[error("Unauthorized")]
    Unauthorized,
    #[error(transparent)]
    Other(#[from] anyhow::Error), // default is an anyhow error
}

impl From<aleph_alpha_client::Error> for InferenceClientError {
    fn from(err: aleph_alpha_client::Error) -> Self {
        match err {
            aleph_alpha_client::Error::Http { status: 401, .. } => {
                InferenceClientError::Unauthorized
            }
            _ => InferenceClientError::Other(err.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::time::Instant;

    use crate::{
        inference::ChatParams,
        tests::{api_token, inference_url},
    };

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn future_which_returns_okay_on_first_try() {
        // Given a future that always returns okay
        let mut counter = 0;
        let ref_counter = &mut counter;
        let future = || {
            *ref_counter += 1;
            std::future::ready(Ok(()))
        };

        // When retrying the future
        let result = retry(future).await;

        // Then the future is invoked only once
        assert!(result.is_ok());
        assert_eq!(counter, 1);
    }

    #[tokio::test(start_paused = true)]
    async fn future_which_returns_okay_on_third_try() {
        // Given a future that always returns okay
        let mut counter = 0;
        let ref_counter = &mut counter;
        let future = || {
            if *ref_counter == 2 {
                std::future::ready(Ok(()))
            } else {
                *ref_counter += 1;
                std::future::ready(Err(aleph_alpha_client::Error::Unavailable))
            }
        };

        // When retrying the future
        let result = retry(future).await;

        // Then the future is invoked three times and returns okay
        assert!(result.is_ok());
        assert_eq!(counter, 2);
    }

    #[tokio::test(start_paused = true)]
    async fn future_which_always_returns_error() {
        // Given a future that always returns error
        let mut counter = 0;
        let ref_counter = &mut counter;
        let future = || {
            *ref_counter += 1;
            std::future::ready(Err::<(), _>(aleph_alpha_client::Error::Unavailable))
        };

        // When retrying the future
        let start = Instant::now();
        let result = retry(future).await;
        let duration = start.elapsed();

        // Then the future is invoked six times and returns an error
        assert!(result.is_err());
        assert_eq!(counter, 6);
        // Some backoff applied
        assert!(duration > Duration::from_secs(1));
    }

    #[tokio::test]
    async fn test_chat_message_conversion() {
        // Given an inference client
        let api_token = api_token().to_owned();
        let host = inference_url().to_owned();
        let client = Client::new(host, None).unwrap();

        // and a chat request
        let chat_request = ChatRequest {
            model: "pharia-1-llm-7b-control".to_owned(),
            params: ChatParams::default(),
            messages: vec![Message::new("user", "Hello, world!")],
        };

        // When chatting with inference client
        let chat_response = <Client as InferenceClient>::chat(&client, &chat_request, api_token)
            .await
            .unwrap();

        // Then a chat response is returned
        assert!(!chat_response.message.content.is_empty());
    }

    #[tokio::test]
    async fn test_bad_token_gives_inference_client_error() {
        // Given an inference client and a bad token
        let bad_api_token = "bad_api_token".to_owned();
        let host = inference_url().to_owned();
        let client = Client::new(host, None).unwrap();

        // and a chat request return an error
        let chat_request = ChatRequest {
            model: "pharia-1-llm-7b-control".to_owned(),
            params: ChatParams::default(),
            messages: vec![Message::new("user", "Hello, world!")],
        };

        // When chatting with inference client
        let chat_result =
            <Client as InferenceClient>::chat(&client, &chat_request, bad_api_token).await;

        // Then an InferenceClientError Unauthorized is returned
        assert!(chat_result.is_err());
        assert!(matches!(
            chat_result.unwrap_err(),
            InferenceClientError::Unauthorized
        ));
    }

    #[tokio::test]
    async fn test_complete_response_with_special_tokens() {
        // Given an inference client
        let api_token = api_token().to_owned();
        let host = inference_url().to_owned();
        let client = Client::new(host, None).unwrap();

        // and a completion request
        let completion_request = CompletionRequest {
            prompt: "<|begin_of_text|><|start_header_id|>system<|end_header_id|>

Environment: ipython<|eot_id|><|start_header_id|>user<|end_header_id|>

Write code to check if number is prime, use that to see if the number 7 is prime<|eot_id|><|start_header_id|>assistant<|end_header_id|>".to_owned(),
            model: "llama-3.1-8b-instruct".to_owned(),
            params:CompletionParams {return_special_tokens: true, ..CompletionParams ::default()}
        };

        // When completing text with inference client
        let completion_response =
            <Client as InferenceClient>::complete_text(&client, &completion_request, api_token)
                .await
                .unwrap();

        // Then a completion response is returned
        assert!(completion_response.text.contains("<|python_tag|>"));
    }

    #[tokio::test]
    async fn frequency_penalty_for_chat() {
        // Given
        let api_token = api_token().to_owned();
        let host = inference_url().to_owned();
        let client = Client::new(host, None).unwrap();

        // When
        let chat_request = ChatRequest {
            model: "pharia-1-llm-7b-control".to_owned(),
            params: ChatParams {
                max_tokens: Some(20),
                temperature: None,
                top_p: None,
                frequency_penalty: Some(-10.0),
                presence_penalty: None,
            },
            messages: vec![Message::new("user", "Haiku about oat milk!")],
        };
        let chat_response = <Client as InferenceClient>::chat(&client, &chat_request, api_token)
            .await
            .unwrap();

        // Then we expect the word oat, to appear at least five times
        let number_oat_mentioned = chat_response
            .message
            .content
            .to_lowercase()
            .split_whitespace()
            .filter(|word| *word == "oat")
            .count();
        assert!(number_oat_mentioned >= 5);
    }

    #[tokio::test]
    async fn sampling_parameters_for_completion() {
        // Given
        let api_token = api_token().to_owned();
        let host = inference_url().to_owned();
        let client = Client::new(host, None).unwrap();

        // When
        let completion_request = CompletionRequest {
            prompt: "An apple a day".to_owned(),
            model: "pharia-1-llm-7b-control".to_owned(),
            params: CompletionParams {
                return_special_tokens: false,
                max_tokens: Some(24),
                temperature: None,
                top_k: Some(10),
                top_p: None,
                frequency_penalty: None,
                presence_penalty: None,
                stop: Vec::new(),
            },
        };
        let completion_response =
            <Client as InferenceClient>::complete_text(&client, &completion_request, api_token)
                .await
                .unwrap();

        // Then we expect the word oat, to appear at least five times
        let completion = completion_response.text;
        // Since for every token we pick one of the top 10 tokens, yet not the most likely it should
        // be fairly certain it is different from the completion we would expect if we would sample
        // deterministically. Sadly we can not specify a seed to make sure the randomness of the
        // test is not leading to a false positive, nor can we narrow down the assertion.
        let deterministic = " keeps the doctor away.\nThis old saying has been around for a long \
            time, and it’s true";
        assert_ne!(deterministic, completion);
    }
}
