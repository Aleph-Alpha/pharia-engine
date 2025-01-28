use std::time::Duration;

use reqwest::Client;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::{policies::ExponentialBackoff, RetryTransientMiddleware};
use reqwest_tracing::TracingMiddleware;

/// A common implementation of an HTTP Client.
/// A wrapper around a reqwest client with common middleware applicable to most services.
/// Includes:
/// - Tracing
/// - Retries
/// - Timeout
pub struct HttpClient(ClientWithMiddleware);

impl HttpClient {
    pub fn new() -> Self {
        let retry_middleware = RetryTransientMiddleware::new_with_policy(
            ExponentialBackoff::builder().build_with_max_retries(3),
        );
        let tracing_middleware = TracingMiddleware::default();
        let client = Client::builder()
            .use_rustls_tls()
            // Reasonable default (not used for inference)
            .timeout(Duration::from_secs(10))
            .build()
            .expect("Invalid reqwest client");

        Self(
            ClientBuilder::new(client)
                .with(retry_middleware)
                .with(tracing_middleware)
                .build(),
        )
    }
}

impl Default for HttpClient {
    fn default() -> Self {
        Self::new()
    }
}
