use exports::pharia::skill::skill_handler::SkillMetadata;
use serde_json::Value;
use wasmtime::component::bindgen;

use crate::skills::Signature;

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

/// Metadata for a skill at wit version 0.3
#[derive(Debug, Clone)]
pub struct SkillMetadataV0_3 {
    pub description: Option<String>,
    pub signature: Signature,
}

impl TryFrom<SkillMetadata> for SkillMetadataV0_3 {
    type Error = anyhow::Error;

    fn try_from(metadata: SkillMetadata) -> Result<Self, Self::Error> {
        let SkillMetadata {
            description,
            input_schema,
            output_schema,
        } = metadata;
        let signature = Signature::Function {
            input_schema: serde_json::from_slice::<Value>(&input_schema)?.try_into()?,
            output_schema: serde_json::from_slice::<Value>(&output_schema)?.try_into()?,
        };
        Ok(SkillMetadataV0_3 {
            description,
            signature,
        })
    }
}
