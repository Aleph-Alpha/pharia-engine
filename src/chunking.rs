use text_splitter::{ChunkCharIndex, ChunkConfig, TextSplitter};
use tokio::sync::oneshot;

use crate::{authorization::Authentication, logging::TracingContext, tokenizers::TokenizerApi};

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
    auth: Authentication,
    tracing_context: TracingContext,
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

    let tokenizer = tokenizers
        .tokenizer_by_model(auth, tracing_context, model)
        .await?;

    // Push into the blocking thread pool because this can be expensive for long documents
    let (send, recv) = oneshot::channel();
    rayon::spawn(move || {
        let result = generate_chunks(&text, max_tokens, overlap, character_offsets, &tokenizer);
        drop(send.send(result));
    });

    Ok(recv.await??)
}

fn generate_chunks(
    text: &str,
    max_tokens: u32,
    overlap: u32,
    character_offsets: bool,
    tokenizer: &tokenizers::Tokenizer,
) -> Result<Vec<Chunk>, text_splitter::ChunkConfigError> {
    let config = ChunkConfig::new(max_tokens as usize)
        .with_sizer(tokenizer)
        .with_overlap(overlap as usize)?;

    let splitter = TextSplitter::new(config);
    let result = if character_offsets {
        splitter
            .chunk_char_indices(text)
            .map(
                |ChunkCharIndex {
                     chunk,
                     byte_offset,
                     char_offset,
                 }| Chunk {
                    text: chunk.to_owned(),
                    byte_offset: byte_offset as u64,
                    character_offset: Some(char_offset as u64),
                },
            )
            .collect()
    } else {
        splitter
            .chunk_indices(text)
            .map(|(byte_offset, text)| Chunk {
                text: text.to_owned(),
                byte_offset: byte_offset as u64,
                character_offset: None,
            })
            .collect()
    };
    Ok(result)
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
                model: "pharia-1-llm-7B-control".to_owned(),
                max_tokens,
                overlap: 0,
            },
            character_offsets: false,
        };

        // When we chunk the text
        let chunks = chunking(
            request,
            &FakeTokenizers,
            Authentication::dummy(),
            TracingContext::dummy(),
        )
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
                model: "pharia-1-llm-7B-control".to_owned(),
                max_tokens: 3,
                overlap: 2,
            },
            character_offsets: false,
        };

        // When we chunk the text
        let chunks = chunking(
            request,
            &FakeTokenizers,
            Authentication::dummy(),
            TracingContext::dummy(),
        )
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
                    byte_offset: 1,
                    character_offset: None
                },
                Chunk {
                    text: "34".to_owned(),
                    byte_offset: 2,
                    character_offset: None
                },
                Chunk {
                    text: "456".to_owned(),
                    byte_offset: 3,
                    character_offset: None
                }
            ]
        );
    }

    #[tokio::test]
    async fn chunking_multi_byte_character() {
        // Given some text containing multi-byte characters and a tokenizer
        let request = ChunkRequest {
            text: "Döner macht schöner ☝🏾! Remember that.".to_owned(),
            params: ChunkParams {
                model: "pharia-1-llm-7B-control".to_owned(),
                max_tokens: 7,
                overlap: 2,
            },
            character_offsets: true,
        };

        // When we chunk the text
        let chunks = chunking(
            request,
            &FakeTokenizers,
            Authentication::dummy(),
            TracingContext::dummy(),
        )
        .await
        .unwrap();
        assert_eq!(
            chunks,
            [
                Chunk {
                    text: "Döner macht".to_owned(),
                    byte_offset: 0,
                    character_offset: Some(0)
                },
                Chunk {
                    text: "macht schöner".to_owned(),
                    byte_offset: 7,
                    character_offset: Some(6)
                },
                Chunk {
                    text: "☝🏾".to_owned(),
                    byte_offset: 22,
                    character_offset: Some(20)
                },
                Chunk {
                    text: "! Remember that.".to_owned(),
                    byte_offset: 29,
                    character_offset: Some(22)
                },
            ]
        );
    }
}
