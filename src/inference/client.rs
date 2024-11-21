use std::future::Future;

use aleph_alpha_client::{
    Client, CompletionOutput, How, Prompt, Sampling, Stopping, TaskChat, TaskCompletion,
};
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
        let fetch_chat_output = || self.chat(&task, &request.model, &how);
        let chat_result = retry(fetch_chat_output).await;
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
                    max_tokens,
                    temperature,
                    top_k,
                    top_p,
                    stop,
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
                start_with_one_of: &[],
            },
        };
        let how = How {
            api_token: Some(api_token),
            ..Default::default()
        };
        let fetch_completion_output = || self.completion(&task, model, &how);

        let completion_result = retry(fetch_completion_output).await;
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

impl TryFrom<aleph_alpha_client::Message<'_>> for Message {
    type Error = anyhow::Error;

    fn try_from(message: aleph_alpha_client::Message<'_>) -> anyhow::Result<Self> {
        Ok(Message {
            role: message.role.parse()?,
            content: message.content.into(),
        })
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
            message: Message::try_from(chat_output.message)?,
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
            maximum_tokens: request.params.max_tokens,
            temperature: request.params.temperature,
            top_p: request.params.top_p,
        }
    }
}

async fn retry<T, F, Fut>(mut f: F) -> Result<T, aleph_alpha_client::Error>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, aleph_alpha_client::Error>>,
{
    let mut remaining_retries = 5;
    loop {
        match f().await {
            Ok(value) => return Ok(value),
            Err(e) if remaining_retries <= 0 => {
                error!("Error after all retries: {e}");
                return Err(e);
            }
            Err(e) => {
                warn!("Retrying operation: {e}");
                remaining_retries -= 1;
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
    use crate::{
        inference::{ChatParams, Role},
        tests::{api_token, inference_address},
    };

    use super::*;

    #[tokio::test]
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

    #[tokio::test]
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

    #[tokio::test]
    async fn future_which_always_returns_error() {
        // Given a future that always returns error
        let mut counter = 0;
        let ref_counter = &mut counter;
        let future = || {
            *ref_counter += 1;
            std::future::ready(Err::<(), _>(aleph_alpha_client::Error::Unavailable))
        };

        // When retrying the future
        let result = retry(future).await;

        // Then the future is invoked six times and returns an error
        assert!(result.is_err());
        assert_eq!(counter, 6);
    }

    #[tokio::test]
    async fn test_chat_message_conversion() {
        // Given an inference client
        let api_token = api_token().to_owned();
        let host = inference_address().to_owned();
        let client = Client::new(host, None).unwrap();

        // and a chat request
        let chat_request = ChatRequest {
            model: "llama-3.1-8b-instruct".to_owned(),
            params: ChatParams {
                max_tokens: None,
                temperature: None,
                top_p: None,
            },
            messages: vec![Message {
                role: Role::User,
                content: "Hello, world!".to_owned(),
            }],
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
        let host = inference_address().to_owned();
        let client = Client::new(host, None).unwrap();

        // and a chat request return an error
        let chat_request = ChatRequest {
            model: "llama-3.1-8b-instruct".to_owned(),
            params: ChatParams::default(),
            messages: vec![Message {
                role: Role::User,
                content: "Hello, world!".to_owned(),
            }],
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
}
