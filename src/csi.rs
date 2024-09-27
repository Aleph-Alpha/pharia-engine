use chunking::ChunkParams;
use futures::future::try_join_all;
use tracing::trace;
use async_trait::async_trait;

use crate::{
    inference::{Completion, CompletionRequest, InferenceApi},
    language_selection::{select_language, Language, SelectLanguageRequest},
    tokenizers::TokenizersApi,
};

pub use self::chunking::ChunkRequest;

pub mod chunking;
mod search;

/// Collection of api handles to the actors used to implement the Cognitive System Interface (CSI)
///
/// For now this is just a collection of all the APIs without providing logic on its own
#[derive(Clone)]
pub struct CsiDrivers {
    /// We use the inference Api to complete text
    pub inference: InferenceApi,
    pub tokenizers: TokenizersApi,
}

/// Cognitive System Interface (CSI) as consumed internally by Pharia Kernel, before the CSI is
/// passed to the end user in Skill code we further strip away some of the accidental complexity.
/// See its sibling trait `CsiForSkills`.
#[async_trait]
pub trait Csi {
    async fn complete_text(
        &self,
        auth: String,
        request: CompletionRequest,
    ) -> Result<Completion, anyhow::Error>;

    async fn complete_all(
        &self,
        auth: String,
        requests: Vec<CompletionRequest>,
    ) -> Result<Vec<Completion>, anyhow::Error>;
    
    async fn chunk(
        &self,
        auth: String,
        request: ChunkRequest,
    ) -> Result<Vec<String>, anyhow::Error>;

    // While the implementation might not be async, we want the interface to be asynchronous.
    // It is up to the implementer whether the actual implementation is async.
    #[allow(clippy::unused_async)]
    async fn select_language(
        &self,
        request: SelectLanguageRequest,
    ) -> anyhow::Result<Option<Language>> {
        // default implementation can be provided here because language selection is stateless
        Ok(tokio::task::spawn_blocking(move || select_language(request)).await?)
    }
}

#[async_trait]
impl Csi for CsiDrivers {
    async fn complete_text(
        &self,
        auth: String,
        request: CompletionRequest,
    ) -> Result<Completion, anyhow::Error> {
        trace!(
            "complete_text: request.model={} request.params.max_tokens={}",
            request.model,
            request
                .params
                .max_tokens
                .map_or_else(|| "None".to_owned(), |val| val.to_string()),
        );
        self.inference.complete_text(request, auth).await
    }

    async fn complete_all(
        &self,
        auth: String,
        requests: Vec<CompletionRequest>,
    ) -> Result<Vec<Completion>, anyhow::Error> {
        trace!("complete_all: requests.len()={}", requests.len());
        try_join_all(
            requests
                .into_iter()
                .map(|r| self.complete_text(auth.clone(), r))
                .collect::<Vec<_>>(),
        )
        .await
    }

    async fn chunk(
        &self,
        auth: String,
        request: ChunkRequest,
    ) -> Result<Vec<String>, anyhow::Error> {
        let ChunkRequest {
            text,
            params: ChunkParams { model, max_tokens },
        } = request;
        let text_len = text.len();

        let tokenizer = self.tokenizers.tokenizer_by_model(auth, model).await?;
        // Push into the blocking thread pool because this can be expensive for long documents
        let chunks =
            tokio::task::spawn_blocking(move || chunking::chunking(&text, &tokenizer, max_tokens))
                .await?;

        trace!(
            "chunk: text_len={} max_tokens={} -> chunks.len()={}",
            text_len,
            max_tokens,
            chunks.len()
        );
        Ok(chunks)
    }
}

#[cfg(test)]
pub mod tests {
    use std::sync::Arc;

    use anyhow::bail;
    use tokio::sync::mpsc;
    use async_trait::async_trait;

    use crate::{inference::{CompletionRequest, InferenceApi, Completion}, tokenizers::TokenizersApi};

    use super::{ChunkRequest, Csi, CsiDrivers};

    pub fn dummy_csi_apis() -> CsiDrivers {
        let (send, _recv) = mpsc::channel(1);
        let inference = InferenceApi::new(send);

        let (send, _recv) = mpsc::channel(1);
        let tokenizers = TokenizersApi::new(send);

        CsiDrivers {
            inference,
            tokenizers,
        }
    }

    #[derive(Clone)]
    pub struct StubCsi {
        completion: Arc<Box<dyn Fn(CompletionRequest) -> Completion + Send + Sync + 'static>>,
    }

    impl StubCsi {
        pub fn with_completion(f: impl Fn(CompletionRequest) -> Completion + Send + Sync + 'static) -> Self {
            StubCsi {
                completion: Arc::new(Box::new(f))
            }
        }

        pub fn with_completion_from_text(text: impl Into<String>) -> Self {
            let text: String = text.into();
            let completion = Completion::from_text(text);
            StubCsi::with_completion(move |_| completion.clone())
        }
    }

    #[async_trait]
    impl Csi for StubCsi {
        async fn complete_text(
            &self,
            _auth: String,
            request: CompletionRequest,
        ) -> Result<Completion, anyhow::Error> {
            Ok((*self.completion)(request))
        }

        async fn complete_all(
            &self,
            _auth: String,
            requests: Vec<CompletionRequest>,
        ) -> Result<Vec<Completion>, anyhow::Error> {
            requests.into_iter().map(|r|Ok((*self.completion)(r))).collect()
        }

        async fn chunk(
            &self,
            _auth: String,
            _request: ChunkRequest,
        ) -> Result<Vec<String>, anyhow::Error> {
            bail!("Test error")
        }
    }
}
