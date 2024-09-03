use serde::{Deserialize, Serialize};
use text_splitter::{ChunkConfig, TextSplitter};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ChunkParams {
    pub max_tokens: u32,
    pub overlap: u32,
    pub trim: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ChunkRequest {
    pub text: String,
    pub model: String,
    pub params: ChunkParams,
}

impl ChunkRequest {
    pub fn new(text: String, model: String, params: ChunkParams) -> Self {
        Self {
            text,
            model,
            params,
        }
    }
}

pub fn chunking(text: &str, tokenizer: tokenizers::Tokenizer, params: ChunkParams) -> Vec<String> {
    let config = ChunkConfig::new(params.max_tokens as usize).with_trim(params.trim);
    let splitter = TextSplitter::new(config);
    splitter.chunks(text).map(str::to_owned).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    use tokenizers::Tokenizer;

    fn pharia_1_llm_7b_control_tokenizer() -> Tokenizer {
        let tokenizer = include_str!("pharia-1-llm-7b-control_tokenizer.json");
        Tokenizer::from_str(tokenizer).unwrap()
    }
    #[tokio::test]
    async fn chunking_splits_text() {
        // Given some text and a tokenizer
        let text = include_str!("no_silver_bullet.txt");
        let tokenizer = pharia_1_llm_7b_control_tokenizer();

        // When we chunk the text
        let params = ChunkParams {
            max_tokens: 1000,
            overlap: 0,
            trim: true,
        };
        let chunks = chunking(text, tokenizer, params);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[1], "But, as we look to the horizon of a decade hence, we see no silver bullet. \
            There is no single development, in either technology or in management technique, that by itself \
            promises even one order-of-magnitude improvement in productivity, in reliability, in simplicity. \
            In this article, I shall try to show why, by examining both the nature of the software problem and \
            the properties of the bullets proposed.\n\nSkepticism is not pessimism, however. Although we see no \
            startling breakthroughs--and indeed, I believe such to be inconsistent with the nature of software--many \
            encouraging innovations are under way. A disciplined, consistent effort to develop, propagate, and \
            exploit these innovations should indeed yield an order-of-magnitude improvement. There is no royal \
            road, but there is a road.");
    }
}
