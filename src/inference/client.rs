use std::{
    future::Future,
    time::{Duration, SystemTime},
};

use aleph_alpha_client::{
    ChatSampling, Client, CompletionOutput, How, Prompt, Sampling, Stopping, TaskChat,
    TaskCompletion, TaskExplanation,
};
use futures::StreamExt;
use retry_policies::{RetryDecision, RetryPolicy, policies::ExponentialBackoff};
use tokio::sync::mpsc;
use tracing::{error, warn};

use thiserror::Error;

use crate::{authorization::Authentication, logging::TracingContext};

use super::{
    ChatEvent, ChatParams, ChatRequest, ChatResponse, Completion, CompletionEvent,
    CompletionParams, CompletionRequest, Distribution, Explanation, ExplanationRequest,
    Granularity, Logprob, Logprobs, Message, TextScore, TokenUsage,
};

#[cfg(test)]
use double_trait::double;

/// The inference client only takes references to the tracing context. Methods like
/// `stream_chat` return a future that in which the tracing context is dropped after
/// making the initial request. However, the lifespan of the tracing context matches
/// the length of the created span, so it may only be dropped after the stream is
/// finished. By taking only a reference, we manifest this fact in the type system.
#[cfg_attr(test, double(InferenceClientDouble))]
pub trait InferenceClient: Send + Sync + 'static {
    fn complete(
        &self,
        request: &CompletionRequest,
        auth: Authentication,
        tracing_context: &TracingContext,
    ) -> impl Future<Output = Result<Completion, InferenceError>> + Send;
    fn stream_completion(
        &self,
        request: &CompletionRequest,
        auth: Authentication,
        tracing_context: &TracingContext,
        send: mpsc::Sender<CompletionEvent>,
    ) -> impl Future<Output = Result<(), InferenceError>> + Send;
    fn chat(
        &self,
        request: &ChatRequest,
        auth: Authentication,
        tracing_context: &TracingContext,
    ) -> impl Future<Output = Result<ChatResponse, InferenceError>> + Send;
    fn stream_chat(
        &self,
        request: &ChatRequest,
        auth: Authentication,
        tracing_context: &TracingContext,
        send: mpsc::Sender<ChatEvent>,
    ) -> impl Future<Output = Result<(), InferenceError>> + Send;
    fn explain(
        &self,
        request: &ExplanationRequest,
        auth: Authentication,
        tracing_context: &TracingContext,
    ) -> impl Future<Output = Result<Explanation, InferenceError>> + Send;
}

/// Create a new [`aleph_alpha_client::How`] based on the given api token and tracing context.
fn how(auth: Authentication, tracing_context: &TracingContext) -> anyhow::Result<How> {
    let api_token = auth.into_maybe_string().ok_or_else(|| {
        anyhow::anyhow!(
            "Doing an inference request against the Aleph Alpha inference API requires a PhariaAI \
            token. Please provide a valid token in the Authorization header."
        )
    })?;
    Ok(How {
        api_token: Some(api_token),
        trace_context: tracing_context.as_inference_client_context(),
        ..Default::default()
    })
}

impl InferenceClient for Client {
    async fn explain(
        &self,
        request: &ExplanationRequest,
        auth: Authentication,
        tracing_context: &TracingContext,
    ) -> Result<Explanation, InferenceError> {
        let ExplanationRequest {
            prompt,
            target,
            model,
            granularity,
        } = request;
        let task = TaskExplanation {
            prompt: Prompt::from_text(prompt),
            target,
            granularity: granularity.into(),
        };
        let how = how(auth, tracing_context)?;
        retry(|| self.explanation(&task, model, &how), tracing_context)
            .await?
            .try_into()
            .map_err(InferenceError::Other)
    }
    async fn chat(
        &self,
        request: &ChatRequest,
        auth: Authentication,
        tracing_context: &TracingContext,
    ) -> Result<ChatResponse, InferenceError> {
        let task = request.to_task_chat();

        let how = how(auth, tracing_context)?;
        retry(|| self.chat(&task, &request.model, &how), tracing_context)
            .await?
            .try_into()
            .map_err(InferenceError::Other)
    }

    async fn stream_chat(
        &self,
        request: &ChatRequest,
        auth: Authentication,
        tracing_context: &TracingContext,
        send: mpsc::Sender<ChatEvent>,
    ) -> Result<(), InferenceError> {
        let task = request.to_task_chat();
        let how = how(auth, tracing_context)?;
        let mut stream = retry(
            || self.stream_chat(&task, &request.model, &how),
            tracing_context,
        )
        .await?;

        while let Some(event) = stream.next().await {
            drop(send.send(ChatEvent::try_from(event?)?).await);
        }
        Ok(())
    }

    async fn complete(
        &self,
        request: &CompletionRequest,
        auth: Authentication,
        tracing_context: &TracingContext,
    ) -> Result<Completion, InferenceError> {
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
                    logprobs,
                    echo,
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
            logprobs: (*logprobs).into(),
            echo: *echo,
        };
        let how = how(auth, tracing_context)?;
        retry(|| self.completion(&task, model, &how), tracing_context)
            .await?
            .try_into()
            .map_err(InferenceError::Other)
    }

    async fn stream_completion(
        &self,
        request: &CompletionRequest,
        auth: Authentication,
        tracing_context: &TracingContext,
        send: mpsc::Sender<CompletionEvent>,
    ) -> Result<(), InferenceError> {
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
                    logprobs,
                    echo,
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
            logprobs: (*logprobs).into(),
            echo: *echo,
        };
        let how = how(auth, tracing_context)?;
        let mut stream = retry(
            || self.stream_completion(&task, model, &how),
            tracing_context,
        )
        .await?;

        while let Some(event) = stream.next().await {
            drop(send.send(CompletionEvent::try_from(event?)?).await);
        }
        Ok(())
    }
}

impl TryFrom<aleph_alpha_client::CompletionEvent> for CompletionEvent {
    type Error = anyhow::Error;

    fn try_from(value: aleph_alpha_client::CompletionEvent) -> Result<Self, Self::Error> {
        Ok(match value {
            aleph_alpha_client::CompletionEvent::Delta {
                completion,
                logprobs,
            } => CompletionEvent::Append {
                text: completion,
                logprobs: logprobs.into_iter().map(Into::into).collect(),
            },
            aleph_alpha_client::CompletionEvent::Finished { reason } => CompletionEvent::End {
                finish_reason: reason.parse()?,
            },
            aleph_alpha_client::CompletionEvent::Summary { usage } => CompletionEvent::Usage {
                usage: usage.into(),
            },
        })
    }
}

impl TryFrom<aleph_alpha_client::ChatEvent> for ChatEvent {
    type Error = anyhow::Error;

    fn try_from(value: aleph_alpha_client::ChatEvent) -> Result<Self, Self::Error> {
        Ok(match value {
            aleph_alpha_client::ChatEvent::MessageStart { role } => {
                ChatEvent::MessageBegin { role }
            }
            aleph_alpha_client::ChatEvent::MessageDelta { content, logprobs } => {
                ChatEvent::MessageAppend {
                    content,
                    logprobs: logprobs.into_iter().map(Into::into).collect(),
                }
            }
            aleph_alpha_client::ChatEvent::MessageEnd { stop_reason } => ChatEvent::MessageEnd {
                finish_reason: stop_reason.parse()?,
            },
            aleph_alpha_client::ChatEvent::Summary { usage } => ChatEvent::Usage {
                usage: usage.into(),
            },
        })
    }
}

impl TryFrom<aleph_alpha_client::ExplanationOutput> for Explanation {
    type Error = anyhow::Error;

    fn try_from(explanation_output: aleph_alpha_client::ExplanationOutput) -> anyhow::Result<Self> {
        match explanation_output.items.first() {
            Some(aleph_alpha_client::ItemExplanation::Text { scores: text }) => {
                Ok(Explanation::new(text.iter().map(Into::into).collect()))
            }
            _ => Err(anyhow::anyhow!(
                "We do not support multi-model prompts, so the first item will always be text."
            )),
        }
    }
}

impl From<&aleph_alpha_client::TextScore> for TextScore {
    fn from(text_score: &aleph_alpha_client::TextScore) -> Self {
        let aleph_alpha_client::TextScore {
            start,
            length,
            score,
        } = text_score;
        TextScore {
            start: *start,
            length: *length,
            score: f64::from(*score),
        }
    }
}

impl TryFrom<CompletionOutput> for Completion {
    type Error = anyhow::Error;

    fn try_from(completion_output: CompletionOutput) -> anyhow::Result<Self> {
        let CompletionOutput {
            completion,
            finish_reason,
            logprobs,
            usage,
        } = completion_output;
        Ok(Self {
            text: completion,
            finish_reason: finish_reason.parse()?,
            logprobs: logprobs.into_iter().map(Into::into).collect(),
            usage: usage.into(),
        })
    }
}

impl From<&Granularity> for aleph_alpha_client::Granularity {
    fn from(granularity: &Granularity) -> Self {
        let prompt = match granularity {
            Granularity::Auto => aleph_alpha_client::PromptGranularity::Auto,
            Granularity::Word => aleph_alpha_client::PromptGranularity::Word,
            Granularity::Sentence => aleph_alpha_client::PromptGranularity::Sentence,
            Granularity::Paragraph => aleph_alpha_client::PromptGranularity::Paragraph,
        };
        aleph_alpha_client::Granularity::default().with_prompt_granularity(prompt)
    }
}

impl From<aleph_alpha_client::Usage> for TokenUsage {
    fn from(usage: aleph_alpha_client::Usage) -> Self {
        TokenUsage {
            prompt: usage.prompt_tokens,
            completion: usage.completion_tokens,
        }
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

impl From<aleph_alpha_client::Distribution> for Distribution {
    fn from(logprob: aleph_alpha_client::Distribution) -> Self {
        let aleph_alpha_client::Distribution { sampled, top } = logprob;
        Distribution {
            sampled: sampled.into(),
            top: top.into_iter().map(Logprob::from).collect(),
        }
    }
}

impl From<aleph_alpha_client::Logprob> for Logprob {
    fn from(logprob: aleph_alpha_client::Logprob) -> Self {
        let aleph_alpha_client::Logprob { token, logprob } = logprob;
        Logprob { token, logprob }
    }
}

impl TryFrom<aleph_alpha_client::ChatOutput> for ChatResponse {
    type Error = anyhow::Error;

    fn try_from(chat_output: aleph_alpha_client::ChatOutput) -> anyhow::Result<Self> {
        let aleph_alpha_client::ChatOutput {
            message,
            finish_reason,
            logprobs,
            usage,
        } = chat_output;
        Ok(ChatResponse {
            message: message.into(),
            finish_reason: finish_reason.parse()?,
            logprobs: logprobs.into_iter().map(Distribution::from).collect(),
            usage: usage.into(),
        })
    }
}

impl ChatRequest {
    pub fn to_task_chat(&self) -> TaskChat<'_> {
        let ChatRequest {
            model: _,
            messages,
            params:
                ChatParams {
                    max_tokens,
                    temperature,
                    top_p,
                    frequency_penalty,
                    presence_penalty,
                    logprobs,
                },
        } = self;
        TaskChat {
            messages: messages.iter().map(Into::into).collect(),
            stopping: Stopping {
                maximum_tokens: *max_tokens,
                stop_sequences: &[],
            },
            sampling: ChatSampling {
                temperature: *temperature,
                top_p: *top_p,
                frequency_penalty: *frequency_penalty,
                presence_penalty: *presence_penalty,
            },
            logprobs: (*logprobs).into(),
        }
    }
}

impl From<Logprobs> for aleph_alpha_client::Logprobs {
    fn from(logprobs: Logprobs) -> Self {
        match logprobs {
            Logprobs::No => aleph_alpha_client::Logprobs::No,
            Logprobs::Sampled => aleph_alpha_client::Logprobs::Sampled,
            Logprobs::Top(n) => aleph_alpha_client::Logprobs::Top(n),
        }
    }
}

async fn retry<T, F, Fut>(
    mut f: F,
    tracing_context: &TracingContext,
) -> Result<T, aleph_alpha_client::Error>
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
                    warn!(
                        parent: tracing_context.span(),
                        "Retrying operation: Attempt #{n_past_retries}.\n{e:#}\n\nSleeping {duration:?} before the next attempt"
                    );
                    tokio::time::sleep(duration).await;
                    n_past_retries += 1;
                }
                RetryDecision::DoNotRetry => {
                    error!(parent: tracing_context.span(), "Error after all retries: {e:#}");
                    return Err(e);
                }
            },
            Err(
                e @ (aleph_alpha_client::Error::Http { .. }
                | aleph_alpha_client::Error::Other(_)
                | aleph_alpha_client::Error::ClientTimeout(_)
                | aleph_alpha_client::Error::InvalidStream { .. }
                | aleph_alpha_client::Error::InvalidTokenizer { .. }
                | aleph_alpha_client::Error::ModelNotFound),
            ) => {
                error!(parent: tracing_context.span(), "Unrecoverable inference error: {e:#}");
                return Err(e);
            }
        }
    }
}

#[derive(Error, Debug)]
pub enum InferenceError {
    #[error("Unauthorized")]
    Unauthorized,
    #[error(
        "The Skill tried to access a model that was not found. Please check the provided model \
        name. You can query the list of available models at the `models` endpoint of the \
        inference API. If you believe the model should be available, contact the operator of your \
        PhariaAI instance."
    )]
    ModelNotFound,
    #[error(
        "No inference backend configured. The Kernel is running without inference capabilities. To \
        enable inference capabilities, please ask your operator to configure the Inference URL in \
        the Kernel configuration."
    )]
    NotConfigured,
    #[error("Tool calls are not supported yet: {0}")]
    ToolCallNotSupported(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error), // default is an anyhow error
}

impl From<aleph_alpha_client::Error> for InferenceError {
    fn from(err: aleph_alpha_client::Error) -> Self {
        match err {
            aleph_alpha_client::Error::Http { status: 401, .. } => InferenceError::Unauthorized,
            aleph_alpha_client::Error::ModelNotFound => InferenceError::ModelNotFound,
            _ => InferenceError::Other(err.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use core::str;

    use tokio::{sync::mpsc, time::Instant};

    use crate::{
        inference::{ChatParams, ExplanationRequest, FinishReason, Granularity},
        tests::{api_token, inference_url},
    };

    use super::*;

    #[tokio::test]
    async fn explain() {
        // Given an inference client
        let auth = Authentication::with_token(api_token());
        let host = inference_url().to_owned();
        let client = Client::new(host, None).unwrap();

        // When explaining complete
        let request = ExplanationRequest {
            prompt: "An apple a day".to_string(),
            target: " keeps the doctor away".to_string(),
            model: "pharia-1-llm-7b-control".to_string(),
            granularity: Granularity::Auto,
        };
        let explanation =
            <Client as InferenceClient>::explain(&client, &request, auth, &TracingContext::dummy())
                .await
                .unwrap();

        // Then we explanation of five items
        assert_eq!(explanation.len(), 5);

        // And the score is the highest for the third item (apple)
        let max_index = explanation
            .iter()
            .max_by(|a, b| a.score.partial_cmp(&b.score).unwrap())
            .unwrap();
        assert_eq!(max_index.start, 3);
    }

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
        let result = retry(future, &TracingContext::dummy()).await;

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
        let result = retry(future, &TracingContext::dummy()).await;

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
        let result = retry(future, &TracingContext::dummy()).await;
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
        let auth = Authentication::with_token(api_token());
        let host = inference_url().to_owned();
        let client = Client::new(host, None).unwrap();

        // and a chat request
        let chat_request = ChatRequest {
            model: "pharia-1-llm-7b-control".to_owned(),
            params: ChatParams::default(),
            messages: vec![Message::new("user", "Hello, world!")],
        };

        // When chatting with inference client
        let chat_response = <Client as InferenceClient>::chat(
            &client,
            &chat_request,
            auth,
            &TracingContext::dummy(),
        )
        .await
        .unwrap();

        // Then a chat response is returned
        assert!(!chat_response.message.content.is_empty());
    }

    #[tokio::test]
    async fn test_bad_token_gives_inference_client_error() {
        // Given an inference client and a bad token
        let bad_auth = Authentication::with_token("bad_api_token");
        let host = inference_url().to_owned();
        let client = Client::new(host, None).unwrap();

        // and a chat request return an error
        let chat_request = ChatRequest {
            model: "pharia-1-llm-7b-control".to_owned(),
            params: ChatParams::default(),
            messages: vec![Message::new("user", "Hello, world!")],
        };

        // When chatting with inference client
        let chat_result = <Client as InferenceClient>::chat(
            &client,
            &chat_request,
            bad_auth,
            &TracingContext::dummy(),
        )
        .await;

        // Then an InferenceClientError Unauthorized is returned
        assert!(chat_result.is_err());
        assert!(matches!(
            chat_result.unwrap_err(),
            InferenceError::Unauthorized
        ));
    }

    #[tokio::test]
    async fn complete_response_with_special_tokens() {
        // Given an inference client
        let auth = Authentication::with_token(api_token());
        let host = inference_url().to_owned();
        let client = Client::new(host, None).unwrap();

        // and a completion request
        let completion_request = CompletionRequest {
            prompt: "<|begin_of_text|><|start_header_id|>system<|end_header_id|>

You are a helpful assistant. You give very short and precise answers to user inquiries.<|eot_id|><|start_header_id|>user<|end_header_id|>

Yes or No?<|eot_id|><|start_header_id|>assistant<|end_header_id|>".to_owned(),
            model: "pharia-1-llm-7b-control".to_owned(),
            params:CompletionParams {return_special_tokens: true, ..CompletionParams ::default()}
        };

        // When completing text with inference client
        let completion_response = <Client as InferenceClient>::complete(
            &client,
            &completion_request,
            auth,
            &TracingContext::dummy(),
        )
        .await
        .unwrap();

        // Then a completion response is returned
        assert!(completion_response.text.contains("<|endoftext|>"));
    }

    #[tokio::test]
    async fn frequency_penalty_for_chat() {
        // Given
        let auth = Authentication::with_token(api_token());
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
                logprobs: Logprobs::No,
            },
            messages: vec![Message::new("user", "Haiku about oat milk!")],
        };
        let chat_response = <Client as InferenceClient>::chat(
            &client,
            &chat_request,
            auth,
            &TracingContext::dummy(),
        )
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
        let auth = Authentication::with_token(api_token());
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
                logprobs: Logprobs::No,
                echo: false,
            },
        };
        let completion_response = <Client as InferenceClient>::complete(
            &client,
            &completion_request,
            auth,
            &TracingContext::dummy(),
        )
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

    #[tokio::test]
    async fn top_logprobs_for_chat() {
        // Given
        let auth = Authentication::with_token(api_token());
        let host = inference_url().to_owned();
        let client = Client::new(host, None).unwrap();

        // When
        let chat_request = ChatRequest {
            model: "pharia-1-llm-7b-control".to_owned(),
            messages: vec![Message::new("user", "An apple a day, ")],
            params: ChatParams {
                max_tokens: Some(1),
                logprobs: Logprobs::Top(2),
                ..Default::default()
            },
        };
        let chat_response = <Client as InferenceClient>::chat(
            &client,
            &chat_request,
            auth,
            &TracingContext::dummy(),
        )
        .await
        .unwrap();

        // Then
        assert_eq!(chat_response.logprobs.len(), 1);
        let top_logprobs = &chat_response.logprobs[0].top;
        assert_eq!(top_logprobs.len(), 2);
        assert_eq!(str::from_utf8(&top_logprobs[0].token).unwrap(), " Keep");
        assert_eq!(str::from_utf8(&top_logprobs[1].token).unwrap(), " keeps");
    }

    #[tokio::test]
    async fn top_logprobs_for_completion() {
        // Given
        let auth = Authentication::with_token(api_token());
        let host = inference_url().to_owned();
        let client = Client::new(host, None).unwrap();

        // When
        let completion_request = CompletionRequest {
            model: "pharia-1-llm-7b-control".to_owned(),
            prompt: "An apple a day, ".to_owned(),
            params: CompletionParams {
                max_tokens: Some(1),
                logprobs: Logprobs::Top(2),
                ..Default::default()
            },
        };
        let completion_response = <Client as InferenceClient>::complete(
            &client,
            &completion_request,
            auth,
            &TracingContext::dummy(),
        )
        .await
        .unwrap();

        // Then
        assert_eq!(completion_response.logprobs.len(), 1);
        let top_logprobs = &completion_response.logprobs[0].top;
        assert_eq!(top_logprobs.len(), 2);
        assert!(top_logprobs[0].logprob > top_logprobs[1].logprob);
    }

    #[tokio::test]
    async fn usage_for_completion() {
        // Given
        let auth = Authentication::with_token(api_token());
        let host = inference_url().to_owned();
        let client = Client::new(host, None).unwrap();

        // When
        let completion_request = CompletionRequest {
            model: "pharia-1-llm-7b-control".to_owned(),
            prompt: "An apple a day, ".to_owned(),
            params: CompletionParams {
                max_tokens: Some(1),
                ..Default::default()
            },
        };
        let completion_response = <Client as InferenceClient>::complete(
            &client,
            &completion_request,
            auth,
            &TracingContext::dummy(),
        )
        .await
        .unwrap();

        // Then
        assert_eq!(completion_response.usage.prompt, 6);
        assert_eq!(completion_response.usage.completion, 1);
    }

    #[tokio::test]
    async fn echo_parameter_leads_to_echo_in_completion() {
        // Given
        let auth = Authentication::with_token(api_token());
        let host = inference_url().to_owned();
        let client = Client::new(host, None).unwrap();

        // When
        let completion_request = CompletionRequest {
            model: "pharia-1-llm-7b-control".to_owned(),
            prompt: "An apple a day, ".to_owned(),
            params: CompletionParams {
                max_tokens: Some(1),
                echo: true,
                ..Default::default()
            },
        };
        let completion_response = <Client as InferenceClient>::complete(
            &client,
            &completion_request,
            auth,
            &TracingContext::dummy(),
        )
        .await
        .unwrap();

        // Then
        assert_eq!(completion_response.text, " An apple a day, keeps");
    }

    #[tokio::test]
    async fn usage_for_chat() {
        // Given
        let auth = Authentication::with_token(api_token());
        let host = inference_url().to_owned();
        let client = Client::new(host, None).unwrap();

        // When
        let chat_request = ChatRequest {
            model: "pharia-1-llm-7b-control".to_owned(),
            messages: vec![Message::new("user", "An apple a day, ")],
            params: ChatParams {
                max_tokens: Some(1),
                ..Default::default()
            },
        };
        let chat_response = <Client as InferenceClient>::chat(
            &client,
            &chat_request,
            auth,
            &TracingContext::dummy(),
        )
        .await
        .unwrap();

        // Then
        assert_eq!(chat_response.usage.prompt, 20);
        assert_eq!(chat_response.usage.completion, 1);
    }

    #[tokio::test]
    async fn completion_stream() {
        // Given
        let auth = Authentication::with_token(api_token());
        let host = inference_url().to_owned();
        let client = Client::new(host, None).unwrap();

        // When
        let completion_request = CompletionRequest {
            model: "pharia-1-llm-7b-control".to_owned(),
            prompt: "An apple a day".to_owned(),
            params: CompletionParams {
                max_tokens: Some(1),
                ..Default::default()
            },
        };
        let (send, mut recv) = mpsc::channel(1);

        tokio::spawn(async move {
            <Client as InferenceClient>::stream_completion(
                &client,
                &completion_request,
                auth,
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
        assert_eq!(events.len(), 3);
        assert!(matches!(events[0], CompletionEvent::Append { .. }));
        assert_eq!(
            events[1],
            CompletionEvent::End {
                finish_reason: FinishReason::Length
            }
        );
        assert!(matches!(events[2], CompletionEvent::Usage { .. }));
    }

    #[tokio::test]
    async fn chat_stream() {
        // Given
        let auth = Authentication::with_token(api_token());
        let host = inference_url().to_owned();
        let client = Client::new(host, None).unwrap();

        // When
        let chat_request = ChatRequest {
            model: "pharia-1-llm-7b-control".to_owned(),
            messages: vec![Message {
                role: "user".to_owned(),
                content: "An apple a day".to_owned(),
            }],
            params: ChatParams {
                max_tokens: Some(1),
                ..Default::default()
            },
        };
        let (send, mut recv) = mpsc::channel(1);

        tokio::spawn(async move {
            <Client as InferenceClient>::stream_chat(
                &client,
                &chat_request,
                auth,
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
        assert_eq!(
            events[0],
            ChatEvent::MessageBegin {
                role: "assistant".to_owned()
            }
        );
        assert!(matches!(events[1], ChatEvent::MessageAppend { .. }));
        assert_eq!(
            events[2],
            ChatEvent::MessageEnd {
                finish_reason: FinishReason::Length
            }
        );
        assert!(matches!(events[3], ChatEvent::Usage { .. }));
    }
}
