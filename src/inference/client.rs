use http::HeaderMap;
use reqwest::header::AUTHORIZATION;
use secrecy::SecretString;
use std::{
    future::Future,
    time::{Duration, SystemTime},
};

use aleph_alpha_client::{
    Client, CompletionOutput, How, Prompt, Sampling, Stopping, TaskCompletion, TaskExplanation,
};
use futures::StreamExt;
use retry_policies::{RetryDecision, RetryPolicy, policies::ExponentialBackoff};
use tokio::sync::mpsc;
use tracing::{error, warn};

use thiserror::Error;

use crate::{
    authorization::Authentication,
    inference::{FinishReason, openai::ChatResponseReasoningContent},
    logging::TracingContext,
};

use super::{
    ChatEvent, ChatRequest, ChatResponse, Completion, CompletionEvent, CompletionParams,
    CompletionRequest, Distribution, Explanation, ExplanationRequest, Granularity, Logprob,
    Logprobs, TextScore, TokenUsage,
};
use async_openai::{Client as OpenAiClient, config::Config};

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
fn how(auth: Authentication, tracing_context: &TracingContext) -> Result<How, InferenceError> {
    let api_token = auth
        .into_maybe_string()
        .ok_or(InferenceError::AlephAlphaTokenRequired)?;
    Ok(How {
        api_token: Some(api_token),
        trace_context: tracing_context.as_inference_client_context(),
        ..Default::default()
    })
}

/// The `AlephAlphaClient` is responsible for resolving inference requests in case the Aleph Alpha
/// inference API is configured. It holds on to two clients. The [`aleph_alpha_client::Client`] is
/// used to handle complete and explain requests, that are special to the Aleph Alpha inference API.
/// The [`reqwest::Client`] is passed to construct an [`async_openai::Client`] to handle chat
/// requests to the Aleph Alpha inference API. While we could also use our own client for this, we
/// do want to use the publicly available libraries, since we aim to be compatible with the `OpenAI`
/// chat API.
pub struct AlephAlphaClient {
    /// The rust client developed by us. Supports our custom features like the explanation endpoint.
    aleph_alpha: Client,
    /// The http client used to make requests by the async openai client.
    /// While we need to construct the config per request, we can store the client across requests.
    client: reqwest::Client,
    /// The url of the inference backend.
    host: String,
}

impl AlephAlphaClient {
    pub fn new(host: impl Into<String>) -> Self {
        let host = host.into();
        let aleph_alpha = Client::new(host.clone(), None).unwrap();
        let client = reqwest::Client::new();
        Self {
            aleph_alpha,
            client,
            host,
        }
    }

    /// Create a new [`OpenAiClient`].
    ///
    /// This client is short-lived, as it has knowledge about the particular authentication and
    /// tracing context for a single request.
    pub fn openai_client<'a>(
        &self,
        auth: Authentication,
        tracing_context: &'a TracingContext,
    ) -> Result<OpenAiClient<AlephAlphaConfig<'a>>, InferenceError> {
        let token = auth
            .into_maybe_string()
            .ok_or(InferenceError::AlephAlphaTokenRequired)?;
        let config = AlephAlphaConfig::new(self.host.clone(), token, tracing_context);
        Ok(OpenAiClient::build(
            self.client.clone(),
            config,
            backoff::ExponentialBackoff::default(),
        ))
    }
}

/// Implementing the [`async_openai::config::Config`] trait allows us to control the headers sent
/// to the inference backend, enabling distributed tracing across our stack.
pub struct AlephAlphaConfig<'a> {
    api_base: String,
    token: String,
    tracing_context: &'a TracingContext,
}

impl<'a> AlephAlphaConfig<'a> {
    fn new(api_base: String, token: String, tracing_context: &'a TracingContext) -> Self {
        AlephAlphaConfig {
            api_base,
            token,
            tracing_context,
        }
    }
}

impl Config for AlephAlphaConfig<'_> {
    fn headers(&self) -> HeaderMap {
        let mut headers = self.tracing_context.w3c_headers();
        headers.insert(
            AUTHORIZATION,
            format!("Bearer {}", self.token).as_str().parse().unwrap(),
        );
        headers
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.api_base, path)
    }

    fn api_base(&self) -> &str {
        unreachable!(
            "We do not make use of this method. It is unclear why it is part of the trait."
        )
    }

    fn api_key(&self) -> &SecretString {
        unreachable!(
            "We do not make use of this method. It is unclear why it is part of the trait."
        )
    }

    fn query(&self) -> Vec<(&str, &str)> {
        vec![]
    }
}

/// Validate our assumptions about the chat response.
///
/// We have an implicit assumption: If no tools are specified in the request, we do not
/// expect a tool call in the response. This means two things: The finish reason cannot be
/// `ToolCalls` and the content must be present. We also have consumers of this functions
/// (old WIT worlds) that never provide tools and always expect a content in the response.
/// One option in Rust would be to specify this in the type system. This could mean having
/// two chat functions, one for requests with tools and one for requests without tools.
/// However, this would mean introducing multiple chat functions in the CSI traits. For now,
/// we believe the best way forward is to check the condition here, and unwrap the content
/// at the consumer side.
pub fn validate_chat_response(
    request: &ChatRequest,
    response: &ChatResponse,
) -> Result<(), InferenceError> {
    if request.params.tools.is_none() {
        if response.message.content.is_none() {
            return Err(InferenceError::EmptyContent);
        }
        if matches!(response.finish_reason, FinishReason::ToolCalls) {
            return Err(InferenceError::UnexpectedToolCall);
        }
    }
    Ok(())
}

/// Validate our assumptions about chat events.
///
/// See [`validate_chat_response`] for more details.
pub fn validate_chat_event(
    request: &ChatRequest,
    response: &ChatEvent,
) -> Result<(), InferenceError> {
    if request.params.tools.is_none() {
        if matches!(response, ChatEvent::ToolCall(_)) {
            return Err(InferenceError::UnexpectedToolCall);
        }
        if matches!(
            response,
            ChatEvent::MessageEnd {
                finish_reason: FinishReason::ToolCalls
            }
        ) {
            return Err(InferenceError::UnexpectedToolCall);
        }
    }

    Ok(())
}

impl InferenceClient for AlephAlphaClient {
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
        retry(
            || self.aleph_alpha.explanation(&task, model, &how),
            tracing_context,
        )
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
        let client = self.openai_client(auth, tracing_context)?;
        let openai_request = request.as_openai_request()?;
        let response: ChatResponseReasoningContent =
            client.chat().create_byot(openai_request).await?;
        let response = ChatResponse::try_from(response)?;
        validate_chat_response(request, &response)?;
        Ok(response)
    }

    async fn stream_chat(
        &self,
        request: &ChatRequest,
        auth: Authentication,
        tracing_context: &TracingContext,
        send: mpsc::Sender<ChatEvent>,
    ) -> Result<(), InferenceError> {
        let client = self.openai_client(auth, tracing_context)?;
        let openai_request = request.as_openai_stream_request()?;
        let mut stream = client.chat().create_stream(openai_request).await?;
        while let Some(event) = stream.next().await {
            let events = ChatEvent::from_stream(event?);
            for event in events {
                validate_chat_event(request, &event)?;
                drop(send.send(event).await);
            }
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
        retry(
            || self.aleph_alpha.completion(&task, model, &how),
            tracing_context,
        )
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
            || self.aleph_alpha.stream_completion(&task, model, &how),
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
    #[error(
        "Doing an inference request against the Aleph Alpha inference API requires a PhariaAI \
        token. Please provide a valid token in the Authorization header."
    )]
    AlephAlphaTokenRequired,
    #[error(
        "The inference backend returned a message with an empty content while no tools were \
        specified in the request. For such responses, we expect a content and no tool calls. This \
        seems to be a bug with the configured inference backend."
    )]
    EmptyContent,
    #[error(
        "The role {0} is not supported by the inference backend. Currently, supported roles are \
        `user`, `assistant`, `system` and `tool`."
    )]
    RoleNotSupported(String),
    #[error(
        "The inference backend returned a tool call, even though no tools were specified in the \
        chat request. This is likely a bug with the inference backend. Please report this issue to \
        the operator of your PhariaAI instance."
    )]
    UnexpectedToolCall,
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
        inference::{ChatParams, ExplanationRequest, FinishReason, Granularity, Message},
        tests::{api_token, inference_url},
    };

    use super::*;

    #[tokio::test]
    #[ignore = "Ignore due to dependency on inference v2 api"]
    async fn chat_with_reasoning_content() {
        // Given an inference client
        let auth = Authentication::from_token(api_token());
        let host = inference_url().to_owned();
        let client = AlephAlphaClient::new(host);

        // When chatting with a reasoning model
        let chat_request = ChatRequest {
            model: "qwen3-32b".to_owned(),
            params: ChatParams::default(),
            messages: vec![Message::user("Hello, world!")],
        };

        let chat_response = <AlephAlphaClient as InferenceClient>::chat(
            &client,
            &chat_request,
            auth,
            &TracingContext::dummy(),
        )
        .await
        .unwrap();

        // Then the chat response contains content and reasoning content
        assert!(chat_response.message.content.is_some());
        assert!(chat_response.message.reasoning_content.is_some());
    }

    #[tokio::test]
    async fn explain() {
        // Given an inference client
        let auth = Authentication::from_token(api_token());
        let host = inference_url().to_owned();
        let client = AlephAlphaClient::new(host);

        // When explaining complete
        let request = ExplanationRequest {
            prompt: "An apple a day".to_string(),
            target: " keeps the doctor away".to_string(),
            model: "pharia-1-llm-7b-control".to_string(),
            granularity: Granularity::Auto,
        };
        let explanation = <AlephAlphaClient as InferenceClient>::explain(
            &client,
            &request,
            auth,
            &TracingContext::dummy(),
        )
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
    async fn chat_message_conversation() {
        // Given an inference client
        let auth = Authentication::from_token(api_token());
        let host = inference_url().to_owned();
        let client = AlephAlphaClient::new(host);

        // and a chat request
        let chat_request = ChatRequest {
            model: "pharia-1-llm-7b-control".to_owned(),
            params: ChatParams::default(),
            messages: vec![Message::user("Hello, world!")],
        };

        // When chatting with inference client
        let chat_response = <AlephAlphaClient as InferenceClient>::chat(
            &client,
            &chat_request,
            auth,
            &TracingContext::dummy(),
        )
        .await
        .unwrap();

        // Then a chat response is returned
        assert!(!chat_response.message.content.unwrap().is_empty());
    }

    #[tokio::test]
    #[ignore = "Skipping: Our inference backend returns an incompatible error format."]
    async fn test_bad_token_gives_inference_client_error() {
        // Given an inference client and a bad token
        let bad_auth = Authentication::from_token("bad_api_token");
        let host = inference_url().to_owned();
        let client = AlephAlphaClient::new(host);

        // and a chat request return an error
        let chat_request = ChatRequest {
            model: "pharia-1-llm-7b-control".to_owned(),
            params: ChatParams::default(),
            messages: vec![Message::user("Hello, world!")],
        };

        // When chatting with inference client
        let chat_result = <AlephAlphaClient as InferenceClient>::chat(
            &client,
            &chat_request,
            bad_auth,
            &TracingContext::dummy(),
        )
        .await
        .unwrap_err();

        // Then an InferenceClientError Unauthorized is returned
        assert!(matches!(chat_result, InferenceError::Unauthorized));
    }

    #[tokio::test]
    async fn complete_response_with_special_tokens() {
        // Given an inference client
        let auth = Authentication::from_token(api_token());
        let host = inference_url().to_owned();
        let client = AlephAlphaClient::new(host);

        // and a completion request
        let completion_request = CompletionRequest {
            prompt: "<|begin_of_text|><|start_header_id|>system<|end_header_id|>

You are a helpful assistant. You give very short and precise answers to user inquiries.<|eot_id|><|start_header_id|>user<|end_header_id|>

Yes or No?<|eot_id|><|start_header_id|>assistant<|end_header_id|>".to_owned(),
            model: "pharia-1-llm-7b-control".to_owned(),
            params:CompletionParams {return_special_tokens: true, ..CompletionParams ::default()}
        };

        // When completing text with inference client
        let completion_response = <AlephAlphaClient as InferenceClient>::complete(
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
        let auth = Authentication::from_token(api_token());
        let host = inference_url().to_owned();
        let client = AlephAlphaClient::new(host);

        // When
        let chat_request = ChatRequest {
            model: "pharia-1-llm-7b-control".to_owned(),
            params: ChatParams {
                max_tokens: Some(20),
                max_completion_tokens: None,
                temperature: None,
                top_p: None,
                frequency_penalty: Some(-10.0),
                presence_penalty: None,
                logprobs: Logprobs::No,
                tools: None,
                tool_choice: None,
                parallel_tool_calls: None,
                response_format: None,
                reasoning_effort: None,
            },
            messages: vec![Message::user("Haiku about oat milk!")],
        };
        let chat_response = <AlephAlphaClient as InferenceClient>::chat(
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
            .unwrap()
            .to_lowercase()
            .split_whitespace()
            .filter(|word| *word == "oat")
            .count();
        assert!(number_oat_mentioned >= 5);
    }

    #[tokio::test]
    async fn sampling_parameters_for_completion() {
        // Given
        let auth = Authentication::from_token(api_token());
        let host = inference_url().to_owned();
        let client = AlephAlphaClient::new(host);

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
        let completion_response = <AlephAlphaClient as InferenceClient>::complete(
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
        let auth = Authentication::from_token(api_token());
        let host = inference_url().to_owned();
        let client = AlephAlphaClient::new(host);

        // When
        let chat_request = ChatRequest {
            model: "pharia-1-llm-7b-control".to_owned(),
            messages: vec![Message::user("An apple a day, ")],
            params: ChatParams {
                max_tokens: Some(1),
                logprobs: Logprobs::Top(2),
                ..Default::default()
            },
        };
        let chat_response = <AlephAlphaClient as InferenceClient>::chat(
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
        let auth = Authentication::from_token(api_token());
        let host = inference_url().to_owned();
        let client = AlephAlphaClient::new(host);

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
        let completion_response = <AlephAlphaClient as InferenceClient>::complete(
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
        let auth = Authentication::from_token(api_token());
        let host = inference_url().to_owned();
        let client = AlephAlphaClient::new(host);

        // When
        let completion_request = CompletionRequest {
            model: "pharia-1-llm-7b-control".to_owned(),
            prompt: "An apple a day, ".to_owned(),
            params: CompletionParams {
                max_tokens: Some(1),
                ..Default::default()
            },
        };
        let completion_response = <AlephAlphaClient as InferenceClient>::complete(
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
        let auth = Authentication::from_token(api_token());
        let host = inference_url().to_owned();
        let client = AlephAlphaClient::new(host);

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
        let completion_response = <AlephAlphaClient as InferenceClient>::complete(
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
        let auth = Authentication::from_token(api_token());
        let host = inference_url().to_owned();
        let client = AlephAlphaClient::new(host);

        // When
        let chat_request = ChatRequest {
            model: "pharia-1-llm-7b-control".to_owned(),
            messages: vec![Message::user("An apple a day, ")],
            params: ChatParams {
                max_tokens: Some(1),
                ..Default::default()
            },
        };
        let chat_response = <AlephAlphaClient as InferenceClient>::chat(
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
        let auth = Authentication::from_token(api_token());
        let host = inference_url().to_owned();
        let client = AlephAlphaClient::new(host);

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
            <AlephAlphaClient as InferenceClient>::stream_completion(
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
        let auth = Authentication::from_token(api_token());
        let host = inference_url().to_owned();
        let client = AlephAlphaClient::new(host);

        // When
        let chat_request = ChatRequest {
            model: "pharia-1-llm-7b-control".to_owned(),
            messages: vec![Message::user("An apple a day")],
            params: ChatParams {
                max_tokens: Some(1),
                temperature: Some(0.0),
                ..Default::default()
            },
        };
        let (send, mut recv) = mpsc::channel(1);

        tokio::spawn(async move {
            <AlephAlphaClient as InferenceClient>::stream_chat(
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
        assert!(matches!(
            &events[1],
            ChatEvent::MessageAppend {
                content,
                ..
            } if content == "Keep"
        ));
        assert_eq!(
            events[2],
            ChatEvent::MessageEnd {
                finish_reason: FinishReason::Length
            }
        );
        assert!(matches!(events[3], ChatEvent::Usage { .. }));
    }
}
