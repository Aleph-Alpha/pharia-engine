use std::time::Duration;

use derive_more::Deref;
use reqwest::Client;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::{RetryTransientMiddleware, policies::ExponentialBackoff};

/// A common implementation of an HTTP Client.
/// A wrapper around a reqwest client with common middleware applicable to most services.
/// Includes:
/// - Retries
/// - Timeout
#[derive(Deref)]
pub struct HttpClient(ClientWithMiddleware);

impl HttpClient {
    pub fn new(retry: bool) -> Self {
        // Make retry optional, as it is not needed for namespace observing where we do it
        // continuously anyway.
        let client = Client::builder()
            .use_rustls_tls()
            // Reasonable default (not used for inference)
            .timeout(Duration::from_secs(10))
            .build()
            .expect("Invalid reqwest client");

        let mut client = ClientBuilder::new(client);
        if retry {
            let retry_middleware = RetryTransientMiddleware::new_with_policy(
                ExponentialBackoff::builder().build_with_max_retries(3),
            );
            client = client.with(retry_middleware);
        }
        Self(client.build())
    }
}

impl Default for HttpClient {
    fn default() -> Self {
        Self::new(true)
    }
}
