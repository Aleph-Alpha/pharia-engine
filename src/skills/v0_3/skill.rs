use async_trait::async_trait;
use exports::pharia::skill::skill_handler::SkillMetadata;
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use wasmtime::component::bindgen;

use crate::{
    csi::Csi,
    logging::TracingContext,
    skills::{AnySkillManifest, Engine, LinkedCtx, Signature, SkillError, SkillEvent},
};

bindgen!({
    world: "skill",
    path: "./wit/skill@0.3",
    async: true,
    with: {
        "pharia:skill/tool": super::csi::pharia::skill::tool,
        "pharia:skill/chunking": super::csi::pharia::skill::chunking,
        "pharia:skill/document-index": super::csi::pharia::skill::document_index,
        "pharia:skill/inference": super::csi::pharia::skill::inference,
        "pharia:skill/language": super::csi::pharia::skill::language,
    }
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

#[async_trait]
impl crate::skills::WasmSkill for SkillPre<LinkedCtx> {
    async fn manifest(
        &self,
        engine: &Engine,
        ctx: Box<dyn Csi + Send>,
        tracing_context: &TracingContext,
    ) -> Result<AnySkillManifest, SkillError> {
        let mut store = engine.store(ctx);
        let bindings = self.instantiate_async(&mut store).await.map_err(|e| {
            error!(parent: tracing_context.span(), "Failed to instantiate skill: {}", e);
            SkillError::RuntimeError(e)
        })?;
        let manifest = bindings
            .pharia_skill_skill_handler()
            .call_metadata(store)
            .await
            .map_err(SkillError::RuntimeError)?;
        manifest
            .try_into()
            .map(AnySkillManifest::V0_3)
            .map_err(SkillError::Any)
    }

    async fn run_as_function(
        &self,
        engine: &Engine,
        ctx: Box<dyn Csi + Send>,
        input: Value,
        tracing_context: &TracingContext,
    ) -> Result<Value, SkillError> {
        let mut store = engine.store(ctx);
        let input = serde_json::to_vec(&input).expect("Json is always serializable");
        let bindings = self.instantiate_async(&mut store).await.map_err(|e| {
            error!(parent: tracing_context.span(), "Failed to instantiate skill: {}", e);
            SkillError::RuntimeError(e)
        })?;
        let result = bindings
            .pharia_skill_skill_handler()
            .call_run(store, &input)
            .await
            .map_err(|e| {
                error!(parent: tracing_context.span(), "Failed to execute skill handler: {}", e);
                SkillError::RuntimeError(e)
            })?;
        let result = match result {
            Ok(result) => result,
            Err(e) => match e {
                exports::pharia::skill::skill_handler::Error::Internal(e) => {
                    return Err(SkillError::UserCode(e));
                }
                exports::pharia::skill::skill_handler::Error::InvalidInput(e) => {
                    info!(parent: tracing_context.span(), "Skill received invalid input: {}", e);
                    return Err(SkillError::InvalidInput(e.to_string()));
                }
            },
        };
        match serde_json::from_slice(&result) {
            Ok(result) => Ok(result),
            Err(e) => {
                warn!(parent: tracing_context.span(), "A skill returned invalid output: {}", e);
                Err(SkillError::InvalidOutput(e.to_string()))
            }
        }
    }

    async fn run_as_message_stream(
        &self,
        _engine: &Engine,
        _ctx: Box<dyn Csi + Send>,
        _input: Value,
        _sender: mpsc::Sender<SkillEvent>,
        _tracing_context: &TracingContext,
    ) -> Result<(), SkillError> {
        Err(SkillError::IsFunction)
    }
}
