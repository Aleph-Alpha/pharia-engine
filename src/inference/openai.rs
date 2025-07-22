use aleph_alpha_client::{Client, How};
use futures::StreamExt;
use tokio::sync::mpsc;

use crate::{
    inference::{
        ChatEvent, ChatRequest, ChatResponse, Completion, CompletionEvent, CompletionRequest,
        Explanation, ExplanationRequest, InferenceError, client::InferenceClient,
    },
    logging::TracingContext,
};

pub struct OpenAiClient {
    client: Client,
}

impl OpenAiClient {
    pub fn new(host: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            client: Client::new(host, Some(api_key.into())).unwrap(),
        }
    }

    const NOT_SUPPORTED: &str = "Inference backend is currently configured to an OpenAI compatible \
    one. For this, we only support chat requests. For other request types, please ask your operator \
    to configure the inference backend to use the Aleph Alpha inference backend.";
}

impl InferenceClient for OpenAiClient {
    async fn chat(
        &self,
        request: &ChatRequest,
        _api_token: String,
        _tracing_context: &TracingContext,
    ) -> Result<ChatResponse, InferenceError> {
        self.client
            .chat(&request.to_task_chat(), &request.model, &How::default())
            .await?
            .try_into()
            .map_err(InferenceError::Other)
    }

    async fn stream_chat(
        &self,
        request: &ChatRequest,
        _api_token: String,
        _tracing_context: &TracingContext,
        send: mpsc::Sender<ChatEvent>,
    ) -> Result<(), InferenceError> {
        let task = request.to_task_chat();
        let how = How::default();
        let mut stream = self.client.stream_chat(&task, &request.model, &how).await?;

        while let Some(event) = stream.next().await {
            drop(send.send(ChatEvent::try_from(event?)?).await);
        }
        Ok(())
    }

    async fn complete(
        &self,
        _request: &CompletionRequest,
        _api_token: String,
        _tracing_context: &TracingContext,
    ) -> Result<Completion, InferenceError> {
        Err(InferenceError::Other(anyhow::anyhow!(Self::NOT_SUPPORTED)))
    }

    async fn stream_completion(
        &self,
        _request: &CompletionRequest,
        _api_token: String,
        _tracing_context: &TracingContext,
        _send: mpsc::Sender<CompletionEvent>,
    ) -> Result<(), InferenceError> {
        Err(InferenceError::Other(anyhow::anyhow!(Self::NOT_SUPPORTED)))
    }

    async fn explain(
        &self,
        _request: &ExplanationRequest,
        _api_token: String,
        _tracing_context: &TracingContext,
    ) -> Result<Explanation, InferenceError> {
        Err(InferenceError::Other(anyhow::anyhow!(Self::NOT_SUPPORTED)))
    }
}

#[cfg(test)]
mod tests {
    use super::OpenAiClient;
    use tokio::sync::mpsc;

    use crate::{
        inference::{
            ChatEvent, ChatParams, ChatRequest, InferenceError, Logprobs, Message,
            client::InferenceClient,
        },
        logging::TracingContext,
        tests::{openai_inference_url, openai_token},
    };

    #[tokio::test]
    async fn chat_message_conversion() {
        // Given an inference client
        let api_token = openai_token().to_owned();
        let host = openai_inference_url().to_owned();
        let client = OpenAiClient::new(host, api_token);

        let result = <OpenAiClient as InferenceClient>::chat(
            &client,
            &ChatRequest {
                model: "gpt-4o-mini".to_owned(),
                messages: vec![Message::new("user", "An apple a day")],
                params: ChatParams::default(),
            },
            "dummy-token".to_owned(),
            &TracingContext::dummy(),
        )
        .await
        .unwrap();

        // Then a chat response is returned
        assert!(!result.message.content.is_empty());
    }

    #[tokio::test]
    async fn top_logprobs_for_chat() {
        // Given an inference client
        let api_token = openai_token().to_owned();
        let host = openai_inference_url().to_owned();
        let client = OpenAiClient::new(host, api_token);

        let params = ChatParams {
            max_tokens: Some(1),
            logprobs: Logprobs::Top(2),
            temperature: Some(0.0),
            ..Default::default()
        };
        let result = <OpenAiClient as InferenceClient>::chat(
            &client,
            &ChatRequest {
                model: "gpt-4o-mini".to_owned(),
                messages: vec![Message::new("user", "An apple a day")],
                params,
            },
            "dummy-token".to_owned(),
            &TracingContext::dummy(),
        )
        .await
        .unwrap();

        // Then
        assert_eq!(result.logprobs.len(), 1);
        let top_logprobs = &result.logprobs[0].top;
        assert_eq!(top_logprobs.len(), 2);
        assert_eq!(str::from_utf8(&top_logprobs[0].token).unwrap(), "\"");
        assert_eq!(str::from_utf8(&top_logprobs[1].token).unwrap(), "The");
    }

    #[tokio::test]
    #[ignore = "error not mapped yet"]
    async fn model_not_found() {
        // Given
        let api_token = openai_token().to_owned();
        let host = openai_inference_url().to_owned();
        let client = OpenAiClient::new(host, api_token);

        // When doing a chat request against a model that does not exist
        let result = <OpenAiClient as InferenceClient>::chat(
            &client,
            &ChatRequest {
                model: "gpt-4o-mini-non-existent".to_owned(),
                messages: vec![Message::new("user", "An apple a day")],
                params: ChatParams::default(),
            },
            "dummy-token".to_owned(),
            &TracingContext::dummy(),
        )
        .await;

        // Then
        assert!(matches!(result, Err(InferenceError::ModelNotFound)));
    }

    #[tokio::test]
    async fn bad_token_gives_unauthorized() {
        // Given a client with a bad token
        let api_token = "bad-token".to_owned();
        let host = openai_inference_url().to_owned();
        let client = OpenAiClient::new(host, api_token);

        // When doing a chat request
        let result = <OpenAiClient as InferenceClient>::chat(
            &client,
            &ChatRequest {
                model: "gpt-4o-mini-non-existent".to_owned(),
                messages: vec![Message::new("user", "An apple a day")],
                params: ChatParams::default(),
            },
            "dummy-token".to_owned(),
            &TracingContext::dummy(),
        )
        .await;

        // Then
        assert!(matches!(result, Err(InferenceError::Unauthorized)));
    }

    #[tokio::test]
    async fn chat_stream() {
        // Given
        let api_token = openai_token().to_owned();
        let host = openai_inference_url().to_owned();
        let client = OpenAiClient::new(host, api_token);

        // When
        let params = ChatParams {
            max_tokens: Some(1),
            temperature: Some(0.0),
            ..Default::default()
        };
        let (send, mut recv) = mpsc::channel(1);
        tokio::spawn(async move {
            <OpenAiClient as InferenceClient>::stream_chat(
                &client,
                &ChatRequest {
                    model: "gpt-4o-mini".to_owned(),
                    messages: vec![Message::new("user", "An apple a day")],
                    params,
                },
                "dummy-token".to_owned(),
                &TracingContext::dummy(),
                send,
            )
            .await
            .unwrap();
        });

        let mut events = vec![];
        while let Some(event) = recv.recv().await {
            events.push(event);
        }

        // Then
        assert_eq!(events.len(), 4);
        assert!(matches!(events[0], ChatEvent::MessageBegin { .. }));
        assert!(matches!(events[1], ChatEvent::MessageAppend { .. }));
        assert!(matches!(events[2], ChatEvent::MessageEnd { .. }));
        assert!(matches!(events[3], ChatEvent::Usage { .. }));
    }
}
