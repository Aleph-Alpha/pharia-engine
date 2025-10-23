use std::time::Duration;

use derive_more::Deref;
use reqwest::Client;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::{RetryTransientMiddleware, policies::ExponentialBackoff};

#[cfg(test)]
use reqwest_vcr::VCRMiddleware;

/// A common implementation of an HTTP Client.
/// A wrapper around a reqwest client with common middleware applicable to most services.
/// Includes:
/// - Retries
/// - Timeout
#[derive(Deref)]
pub struct HttpClient(ClientWithMiddleware);

impl HttpClient {
    /// Create an `HttpClient` with retry middleware (standard configuration).
    pub fn with_retry() -> Self {
        let retry_middleware = RetryTransientMiddleware::new_with_policy(
            ExponentialBackoff::builder().build_with_max_retries(3),
        );
        let client = Self::client();
        let with_middleware = ClientBuilder::new(client).with(retry_middleware).build();
        Self(with_middleware)
    }

    /// Create an `HttpClient` without any middleware.
    pub fn without_retry() -> Self {
        let client = Self::client();
        Self(ClientBuilder::new(client).build())
    }

    #[cfg(test)]
    pub fn with_vcr(path_to_cassette: std::path::PathBuf, vcr_mode: reqwest_vcr::VCRMode) -> Self {
        let vcr_middleware = VCRMiddleware::try_from(path_to_cassette)
            .unwrap()
            .with_mode(vcr_mode)
            .with_modify_request(|request| {
                if let Some(header) = request.headers.get_mut("authorization") {
                    *header = vec!["TOKEN_REMOVED".to_owned()];
                }
            });

        let client = Self::client();
        let with_middleware = ClientBuilder::new(client).with(vcr_middleware).build();
        Self(with_middleware)
    }

    fn client() -> Client {
        Client::builder()
            .use_rustls_tls()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("Invalid reqwest client")
    }
}

impl Default for HttpClient {
    fn default() -> Self {
        Self::with_retry()
    }
}
