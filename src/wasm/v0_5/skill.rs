use serde_json::Value;
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use wasmtime::component::bindgen;

use crate::{
    logging::TracingContext,
    skill::{AnySkillManifest, BoxedCsi, Signature, SkillError, SkillEvent, SkillMetadataV0_4},
    wasm::{Engine, LinkedCtx, v0_5::skill::exports::pharia::skill::skill_handler::SkillMetadata},
};

bindgen!({
    world: "skill",
    path: "./wit/skill@0.5",
    async: true,
    with: {
        "pharia:skill/tool": super::csi::pharia::skill::tool,
        "pharia:skill/chunking": super::csi::pharia::skill::chunking,
        "pharia:skill/document-index": super::csi::pharia::skill::document_index,
        "pharia:skill/inference": super::csi::pharia::skill::inference,
        "pharia:skill/language": super::csi::pharia::skill::language,
    },
});

impl TryFrom<SkillMetadata> for SkillMetadataV0_4 {
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
        Ok(SkillMetadataV0_4 {
            description,
            signature,
        })
    }
}

impl crate::wasm::SkillComponent for SkillPre<LinkedCtx> {
    async fn manifest(
        &self,
        engine: &Engine,
        ctx: BoxedCsi,
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
            .map(AnySkillManifest::V0_4)
            .map_err(SkillError::Any)
    }

    async fn run_as_function(
        &self,
        engine: &Engine,
        ctx: BoxedCsi,
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
                    return Err(SkillError::InvalidInput(e.clone()));
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
        _ctx: BoxedCsi,
        _input: Value,
        _sender: mpsc::Sender<SkillEvent>,
        _tracing_context: &TracingContext,
    ) -> Result<(), SkillError> {
        Err(SkillError::IsFunction)
    }
}
