use std::future::Future;

use aleph_alpha_client::{
    Client, CompletionOutput, How, Prompt, Sampling, Stopping, TaskChat, TaskCompletion,
};

use super::{
    ChatRequest, ChatResponse, Completion, CompletionParams, CompletionRequest, Message, Role,
};

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
        let chat_output = self
            .chat(
                &task,
                &request.model,
                &How {
                    api_token: Some(api_token),
                    ..Default::default()
                },
            )
            .await?;
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

        let completion_output = self
            .completion(
                &task,
                model,
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

impl From<aleph_alpha_client::Role> for Role {
    fn from(role: aleph_alpha_client::Role) -> Self {
        match role {
            aleph_alpha_client::Role::System => Role::System,
            aleph_alpha_client::Role::User => Role::User,
            aleph_alpha_client::Role::Assistant => Role::Assistant,
        }
    }
}

impl From<&Role> for aleph_alpha_client::Role {
    fn from(role: &Role) -> Self {
        match role {
            Role::System => aleph_alpha_client::Role::System,
            Role::User => aleph_alpha_client::Role::User,
            Role::Assistant => aleph_alpha_client::Role::Assistant,
        }
    }
}

impl From<aleph_alpha_client::Message<'_>> for Message {
    fn from(message: aleph_alpha_client::Message<'_>) -> Self {
        Message {
            role: message.role.into(),
            content: message.content.into(),
        }
    }
}

impl<'a> From<&'a Message> for aleph_alpha_client::Message<'a> {
    fn from(message: &'a Message) -> Self {
        aleph_alpha_client::Message {
            role: (&message.role).into(),
            content: (&message.content).into(),
        }
    }
}

impl TryFrom<aleph_alpha_client::ChatOutput> for ChatResponse {
    type Error = anyhow::Error;

    fn try_from(chat_output: aleph_alpha_client::ChatOutput) -> anyhow::Result<Self> {
        Ok(ChatResponse {
            message: Message::from(chat_output.message),
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

#[cfg(test)]
mod tests {
    use std::env;

    use crate::inference::ChatParams;

    use super::*;

    #[tokio::test]
    async fn test_chat_message_conversion() {
        drop(dotenvy::dotenv());

        // Given an inference client
        let api_token = env::var("AA_API_TOKEN").unwrap();
        let host = env::var("AA_INFERENCE_ADDRESS").unwrap();
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
