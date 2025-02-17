use crate::skills::v0_3::csi;
use exports::pharia::skill::skill_handler::SkillMetadata;
use serde_json::Value;
use wasmtime::component::bindgen;

use crate::skills;

bindgen!({
    world: "skill",
    path: "./wit/skill@0.3",
    async: true,
    with: {
        "pharia:skill/inference": csi::pharia::skill::inference,
        "pharia:skill/chunking": csi::pharia::skill::chunking,
        "pharia:skill/document-index": csi::pharia::skill::document_index,
        "pharia:skill/language": csi::pharia::skill::language,
        "pharia:skill/response": csi::pharia::skill::response,
    },
});

impl TryFrom<SkillMetadata> for skills::SkillMetadata {
    type Error = anyhow::Error;

    fn try_from(metadata: SkillMetadata) -> Result<Self, Self::Error> {
        let SkillMetadata {
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
