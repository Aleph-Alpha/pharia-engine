use async_trait::async_trait;
use exports::pharia::skill::streaming_skill_handler::StreamOutput;
use pharia::skill::streaming_host::{Host, HostStreamOutput, MessageItem};
use serde_json::Value;
use tokio::sync::mpsc;
use wasmtime::component::{Resource, bindgen};

use crate::{
    csi::CsiForSkills,
    skill_runtime::StreamEvent,
    skills::{AnySkillManifest, Engine, LinkedCtx, SkillError},
};

bindgen!({
    world: "streaming-skill",
    path: "./wit/skill@0.3",
    async: true,
    with: {
        "pharia:skill/chunking": super::csi::pharia::skill::chunking,
        "pharia:skill/document-index": super::csi::pharia::skill::document_index,
        "pharia:skill/inference": super::csi::pharia::skill::inference,
        "pharia:skill/language": super::csi::pharia::skill::language,
    },
});

#[async_trait]
impl crate::skills::Skill for StreamingSkillPre<LinkedCtx> {
    async fn manifest(
        &self,
        _engine: &Engine,
        _ctx: Box<dyn CsiForSkills + Send>,
    ) -> Result<AnySkillManifest, SkillError> {
        // Still need to define metadata for streaming skills
        Ok(AnySkillManifest::V0)
    }

    async fn run_as_function(
        &self,
        _engine: &Engine,
        _ctx: Box<dyn CsiForSkills + Send>,
        _input: Value,
    ) -> Result<Value, SkillError> {
        Err(SkillError::IsFunction)
    }

    async fn run_as_generator(
        &self,
        engine: &Engine,
        ctx: Box<dyn CsiForSkills + Send>,
        input: Value,
        sender: mpsc::Sender<StreamEvent>,
    ) -> Result<(), SkillError> {
        let mut store = engine.store(LinkedCtx::new(ctx));
        let input = serde_json::to_vec(&input).expect("Json is always serializable");
        let bindings = self.instantiate_async(&mut store).await.map_err(|e| {
            tracing::error!("Failed to instantiate skill: {}", e);
            SkillError::RuntimeError(e)
        })?;
        bindings
            .pharia_skill_streaming_skill_handler()
            .call_run(store, &input, todo!())
            .await
            .map_err(|e| {
                tracing::error!("Failed to execute skill handler: {}", e);
                SkillError::RuntimeError(e)
            })?
            .map_err(|e| match e {
                exports::pharia::skill::streaming_skill_handler::Error::Internal(e) => {
                    SkillError::UserCode(e)
                }
                exports::pharia::skill::streaming_skill_handler::Error::InvalidInput(e) => {
                    SkillError::InvalidInput(e.to_string())
                }
            })
    }
}

impl Host for LinkedCtx {}

impl HostStreamOutput for LinkedCtx {
    async fn write_message_item(&mut self, output: Resource<StreamOutput>, item: MessageItem) {
        todo!()
    }

    async fn drop(&mut self, output: Resource<StreamOutput>) -> anyhow::Result<()> {
        todo!()
    }
}
