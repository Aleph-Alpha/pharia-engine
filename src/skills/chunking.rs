
use serde::{Deserialize, Serialize};

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
        Self { text, model, params }
    }
}


pub async fn chunking(text: &str, tokenizer: tokenizers::Tokenizer, params: ChunkParams) -> Vec<String> {
    vec![text.to_owned()]
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use super::*;

    use tokenizers::Tokenizer;

    fn pharia_1_llm_7b_control_tokenizer() -> Tokenizer {
        let tokenizer = include_str!("pharia-1-llm-7b-control_tokenizer.json");
        Tokenizer::from_str(&tokenizer).unwrap()

    }
    #[tokio::test]
    async fn chunking_splits_text() {
        // Given some text and a tokenizer
        let text = "We are out of cheese. Cheese is no more. Cheese has ceased to be. It is an ex-cheese.";
        let tokenizer = pharia_1_llm_7b_control_tokenizer();

        // When we chunk the text
        let params = ChunkParams {
            max_tokens: 50,
            overlap: 0,
            trim: true,
        };
        let result = chunking(text, tokenizer, params).await;
        assert_eq!(result.len(), 1);
    }
}