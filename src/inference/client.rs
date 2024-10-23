use std::future::Future;

use aleph_alpha_client::{
    Client, CompletionOutput, How, Prompt, Sampling, Stopping, TaskChat, TaskCompletion,
};
use tracing::{error, warn};

use super::{ChatRequest, ChatResponse, Completion, CompletionParams, CompletionRequest, Message};

pub trait InferenceClient: Send + Sync + 'static {
    fn complete_text(
        &self,
        request: &CompletionRequest,
        api_token: String,
    ) -> impl Future<Output = anyhow::Result<Completion>> + Send;
    fn chat(
        &self,
        request: &ChatRequest,
        api_token: String,
    ) -> impl Future<Output = anyhow::Result<ChatResponse>> + Send;
}

impl InferenceClient for Client {
    async fn chat(&self, request: &ChatRequest, api_token: String) -> anyhow::Result<ChatResponse> {
        let task = request.into();
        let how = How {
            api_token: Some(api_token),
            ..Default::default()
        };
        let fetch_chat_output = || self.chat(&task, &request.model, &how);
        let chat_output = retry(fetch_chat_output).await?;
        chat_output.try_into()
    }

    async fn complete_text(
        &self,
        request: &CompletionRequest,
        api_token: String,
    ) -> anyhow::Result<Completion> {
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

        let completion_output = retry(fetch_completion_output).await?;
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

async fn retry<T, F, Fut>(mut f: F) -> anyhow::Result<T>
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
                return Err(e.into());
            }
            Err(e) => {
                warn!("Retrying operation: {e}");
                remaining_retries -= 1;
            }
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
}
