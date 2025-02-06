use serde::{Deserialize, Serialize};
use text_splitter::{ChunkConfig, TextSplitter};

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct ChunkRequest {
    pub text: String,
    pub params: ChunkParams,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct ChunkParams {
    pub model: String,
    pub max_tokens: u32,
}

impl ChunkRequest {
    pub fn new(text: String, model: String, max_tokens: u32) -> Self {
        Self {
            text,
            params: ChunkParams { model, max_tokens },
        }
    }
}

pub fn chunking(text: &str, tokenizer: &tokenizers::Tokenizer, max_tokens: u32) -> Vec<String> {
    let config = ChunkConfig::new(max_tokens as usize).with_sizer(tokenizer);
    let splitter = TextSplitter::new(config);
    splitter.chunks(text).map(str::to_owned).collect()
}

#[cfg(test)]
mod tests {
    use crate::tokenizers::tests::pharia_1_llm_7b_control_tokenizer;

    use super::*;

    #[tokio::test]
    async fn chunking_splits_text() {
        // Given some text and a tokenizer
        let text = include_str!("../tests/no_silver_bullet.txt");
        let tokenizer = pharia_1_llm_7b_control_tokenizer();

        // When we chunk the text
        let max_tokens = 100;
        let chunks = chunking(text, &tokenizer, max_tokens);
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
