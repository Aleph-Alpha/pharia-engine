use serde::{Deserialize, Serialize};

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
