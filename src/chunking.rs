use serde::{Deserialize, Serialize};
use text_splitter::{ChunkConfig, TextSplitter};

use crate::tokenizers::TokenizerApi;

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct ChunkRequest {
    pub text: String,
    pub params: ChunkParams,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct ChunkParams {
    pub model: String,
    pub max_tokens: u32,
    pub overlap: u32,
}

impl ChunkRequest {
    pub fn new(text: String, model: String, max_tokens: u32, overlap: u32) -> Self {
        Self {
            text,
            params: ChunkParams {
                model,
                max_tokens,
                overlap,
            },
        }
    }
}

pub async fn chunking(
    request: ChunkRequest,
    tokenizers: &impl TokenizerApi,
    auth: String,
) -> anyhow::Result<Vec<String>> {
    let ChunkRequest {
        text,
        params:
            ChunkParams {
                model,
                max_tokens,
                overlap,
            },
    } = request;

    let tokenizer = tokenizers.tokenizer_by_model(auth, model).await?;

    // Push into the blocking thread pool because this can be expensive for long documents
    let chunks = tokio::task::spawn_blocking(move || {
        let config = ChunkConfig::new(max_tokens as usize).with_sizer(tokenizer.as_ref());
        let splitter = TextSplitter::new(config);
        splitter.chunks(&text).map(str::to_owned).collect()
    })
    .await?;

    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use crate::tokenizers::tests::FakeTokenizers;

    use super::*;

    #[tokio::test]
    async fn chunking_splits_text() {
        // Given some text and a tokenizer
        let text = include_str!("../tests/no_silver_bullet.txt");
        let max_tokens = 100;
        let request = ChunkRequest {
            text: text.to_owned(),
            params: ChunkParams {
                model: "Pharia-1-LLM-7B-control".to_owned(),
                max_tokens,
                overlap: 0,
            },
        };

        // When we chunk the text
        let chunks = chunking(request, &FakeTokenizers, "dummy".to_owned())
            .await
            .unwrap();
        assert_eq!(chunks.len(), 5);
        assert_eq!(
            chunks[1],
            "The familiar software project, at least as seen by the nontechnical \
            manager, has something of this character; it is usually innocent and straightforward, \
            but is capable of becoming a monster of missed schedules, blown budgets, and flawed \
            products. So we hear desperate cries for a silver bullet--something to make software \
            costs drop as rapidly as computer hardware costs do."
        );
    }
}
