use text_splitter::{ChunkConfig, TextSplitter};
use tokio::sync::oneshot;

use crate::tokenizers::TokenizerApi;

#[derive(Debug, PartialEq, Eq)]
pub struct ChunkRequest {
    pub text: String,
    pub params: ChunkParams,
    pub character_offsets: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Chunk {
    pub text: String,
    pub byte_offset: u64,
    pub character_offset: Option<u64>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ChunkParams {
    pub model: String,
    pub max_tokens: u32,
    pub overlap: u32,
}

pub async fn chunking(
    request: ChunkRequest,
    tokenizers: &impl TokenizerApi,
    auth: String,
) -> anyhow::Result<Vec<Chunk>> {
    let ChunkRequest {
        text,
        params:
            ChunkParams {
                model,
                max_tokens,
                overlap,
            },
        character_offsets,
    } = request;

    let tokenizer = tokenizers.tokenizer_by_model(auth, model).await?;

    // Push into the blocking thread pool because this can be expensive for long documents
    let (send, recv) = oneshot::channel();
    rayon::spawn(move || {
        let result = ChunkConfig::new(max_tokens as usize)
            .with_sizer(tokenizer.as_ref())
            .with_overlap(overlap as usize)
            .map(|config| {
                TextSplitter::new(config)
                    .chunks(&text)
                    .map(|text| Chunk {
                        text: text.to_owned(),
                        byte_offset: 0,
                        character_offset: None,
                    })
                    .collect()
            });

        drop(send.send(result));
    });

    Ok(recv.await??)
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
            character_offsets: false,
        };

        // When we chunk the text
        let chunks = chunking(request, &FakeTokenizers, "dummy".to_owned())
            .await
            .unwrap();
        assert_eq!(chunks.len(), 5);
        assert_eq!(
            chunks[1].text,
            "The familiar software project, at least as seen by the nontechnical \
            manager, has something of this character; it is usually innocent and straightforward, \
            but is capable of becoming a monster of missed schedules, blown budgets, and flawed \
            products. So we hear desperate cries for a silver bullet--something to make software \
            costs drop as rapidly as computer hardware costs do."
        );
    }

    #[tokio::test]
    async fn chunking_with_overlap() {
        // Given some text and a tokenizer
        let request = ChunkRequest {
            text: "123456".to_owned(),
            params: ChunkParams {
                model: "Pharia-1-LLM-7B-control".to_owned(),
                max_tokens: 3,
                overlap: 2,
            },
            character_offsets: false,
        };

        // When we chunk the text
        let chunks = chunking(request, &FakeTokenizers, "dummy".to_owned())
            .await
            .unwrap();
        assert_eq!(
            chunks,
            [
                Chunk {
                    text: "12".to_owned(),
                    byte_offset: 0,
                    character_offset: None
                },
                Chunk {
                    text: "23".to_owned(),
                    byte_offset: 0,
                    character_offset: None
                },
                Chunk {
                    text: "34".to_owned(),
                    byte_offset: 0,
                    character_offset: None
                },
                Chunk {
                    text: "456".to_owned(),
                    byte_offset: 0,
                    character_offset: None
                }
            ]
        );
    }
}
