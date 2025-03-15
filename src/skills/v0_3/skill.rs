use exports::pharia::skill::skill_handler::SkillMetadata;
use serde_json::Value;
use wasmtime::component::bindgen;

bindgen!({
    world: "skill",
    path: "./wit/skill@0.3",
    async: true,
    with: {
        "pharia:skill/chunking": super::csi::pharia::skill::chunking,
        "pharia:skill/document-index": super::csi::pharia::skill::document_index,
        "pharia:skill/inference": super::csi::pharia::skill::inference,
        "pharia:skill/language": super::csi::pharia::skill::language,
    },
});

impl TryFrom<SkillMetadata> for super::super::SkillMetadata {
    type Error = anyhow::Error;

    fn try_from(metadata: SkillMetadata) -> Result<Self, Self::Error> {
        let SkillMetadata {
            description,
            input_schema,
            output_schema,
        } = metadata;
        Ok(Self::V0_3(super::super::SkillMetadataV1 {
            description,
            input_schema: serde_json::from_slice::<Value>(&input_schema)?.try_into()?,
            output_schema: serde_json::from_slice::<Value>(&output_schema)?.try_into()?,
        }))
    }
}
