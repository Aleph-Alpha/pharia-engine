//! Fetching and instatiating tokenizers
use anyhow::{anyhow, Context};
use async_trait::async_trait;
use tokenizers::Tokenizer;

/// Allows to a tokenizer for a given model
#[async_trait]
pub trait TokenizerProvider {
    /// Tokenizer best used for this model
    async fn tokenizer_for_model(&mut self, model: &str) -> Result<Tokenizer, anyhow::Error>;
}

/// A [`TokenizerProvider`] which fetches the fitting tokenizer for a model by asking the Aleph
/// Alpha Inference API fro the correct one.
pub struct TokenizerFromAAInference {
    api_base_url: String,
    api_token: String,
}

impl TokenizerFromAAInference {
    pub fn new(api_base_url: String, api_token: String) -> Self {
        Self {
            api_base_url,
            api_token,
        }
    }
}

#[async_trait]
impl TokenizerProvider for TokenizerFromAAInference {
    async fn tokenizer_for_model(&mut self, model: &str) -> Result<Tokenizer, anyhow::Error> {
        let url = format!("{}/models/{model}/tokenizer", self.api_base_url);
        let client = reqwest::Client::new();
        let response = client
            .get(url)
            .bearer_auth(self.api_token.clone())
            .send()
            .await
            .with_context(|| format!("Error fetching tokenizer for {model}"))?;
        response
            .error_for_status_ref()
            .with_context(|| format!("Error fetching tokenizer for {model}"))?;
        let tokenizer = response
            .bytes()
            .await
            .with_context(|| format!("Error fetching tokenizer for {model}"))?;

        let tokenizer = Tokenizer::from_bytes(tokenizer).map_err(|e| {
            anyhow!(
                "Error deserializing tokenizer for {model}: {}",
                e.to_string()
            )
        })?;
        Ok(tokenizer)
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::api_token;

    use super::{TokenizerFromAAInference, TokenizerProvider};

    #[tokio::test]
    async fn fetch_pharia_1_llm_7b_control_tokenizer() {
        // Given a model name and the actual inference API
        let model_name = "Pharia-1-LLM-7B-control";
        let base_url = "https://api.aleph-alpha.com".to_owned();
        let api_token = api_token();

        // When we can request a tokenizer from the AA API
        let mut provider = TokenizerFromAAInference::new(base_url, api_token.to_owned());
        let tokenizer = provider.tokenizer_for_model(model_name).await.unwrap();

        // Then we can use the tokenizer
        assert_eq!(tokenizer.get_vocab_size(true), 128_000);
    }
}
