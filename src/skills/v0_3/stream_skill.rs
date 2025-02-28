use crate::skills::v0_3::csi;
use exports::pharia::skill::stream_skill_handler::StreamSkillMetadata;
use pharia::skill::chat_response::Host as ChatResponseHost;
use serde_json::Value;
use wasmtime::component::bindgen;

use crate::skills;

bindgen!({
    world: "stream-skill",
    path: "./wit/skill@0.3",
    async: true,
    with: {
        "pharia:skill/inference": csi::pharia::skill::inference,
        "pharia:skill/chunking": csi::pharia::skill::chunking,
        "pharia:skill/document-index": csi::pharia::skill::document_index,
        "pharia:skill/language": csi::pharia::skill::language,
    },
});

impl TryFrom<StreamSkillMetadata> for skills::SkillMetadata {
    type Error = anyhow::Error;

    fn try_from(metadata: StreamSkillMetadata) -> Result<Self, Self::Error> {
        let StreamSkillMetadata {
            description,
            input_schema,
            output_schema,
        } = metadata;
        Ok(Self::V1(skills::SkillMetadataV1 {
            description,
            input_schema: serde_json::from_slice::<Value>(&input_schema)?.try_into()?,
            output_schema: serde_json::from_slice::<Value>(&output_schema)?.try_into()?,
        }))
    }
}

impl ChatResponseHost for skills::LinkedCtx {
    async fn write(&mut self, data: Vec<u8>) {
        self.skill_ctx.write(data).await;
    }
}
