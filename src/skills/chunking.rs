use serde::{Deserialize, Serialize};
use text_splitter::{ChunkConfig, TextSplitter};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ChunkParams {
    pub max_tokens: u32,
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

pub fn chunking(
    text: &str,
    tokenizer: &tokenizers::Tokenizer,
    params: &ChunkParams,
) -> Vec<String> {
    let config = ChunkConfig::new(params.max_tokens as usize).with_sizer(tokenizer);
    let splitter = TextSplitter::new(config);
    splitter.chunks(text).map(str::to_owned).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use tokenizers::Tokenizer;

    fn pharia_1_llm_7b_control_tokenizer() -> Tokenizer {
        let tokenizer = include_bytes!("pharia-1-llm-7b-control_tokenizer.json");
        Tokenizer::from_bytes(tokenizer).unwrap()
    }
    #[tokio::test]
    async fn chunking_splits_text() {
        // Given some text and a tokenizer
        let text = include_str!("no_silver_bullet.txt");
        let tokenizer = pharia_1_llm_7b_control_tokenizer();

        // When we chunk the text
        let params = ChunkParams { max_tokens: 100 };
        let chunks = chunking(text, &tokenizer, &params);
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
