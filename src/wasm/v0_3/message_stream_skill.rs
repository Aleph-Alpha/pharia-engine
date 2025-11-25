use pharia::skill::streaming_output::{Host, HostStreamOutput, MessageItem};
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::error;
use wasmtime::component::{Resource, bindgen};

use crate::{
    logging::TracingContext,
    skill::BoxedCsi,
    wasm::{AnySkillManifest, Engine, LinkedCtx, SkillError, SkillEvent},
};

pub type StreamOutput = mpsc::Sender<SkillEvent>;

bindgen!({
    world: "message-stream-skill",
    path: "./wit/skill@0.3",
    async: true,
    with: {
        "pharia:skill/tool": super::csi::pharia::skill::tool,
        "pharia:skill/chunking": super::csi::pharia::skill::chunking,
        "pharia:skill/document-index": super::csi::pharia::skill::document_index,
        "pharia:skill/inference": super::csi::pharia::skill::inference,
        "pharia:skill/language": super::csi::pharia::skill::language,
        "pharia:skill/streaming-output/stream-output": StreamOutput,
    },
});

impl crate::wasm::SkillComponent for MessageStreamSkillPre<LinkedCtx> {
    async fn manifest(
        &self,
        _engine: &Engine,
        _ctx: BoxedCsi,
        _tracing_context: &TracingContext,
    ) -> Result<AnySkillManifest, SkillError> {
        // Still need to define metadata for streaming skills
        Ok(AnySkillManifest::V0)
    }

    async fn run_as_function(
        &self,
        _engine: &Engine,
        _ctx: BoxedCsi,
        _input: Value,
        _tracing_context: &TracingContext,
    ) -> Result<Value, SkillError> {
        Err(SkillError::IsMessageStream)
    }

    async fn run_as_message_stream(
        &self,
        engine: &Engine,
        ctx: BoxedCsi,
        input: Value,
        sender: mpsc::Sender<SkillEvent>,
        tracing_context: &TracingContext,
    ) -> Result<(), SkillError> {
        let mut store = engine.store(ctx);
        let stream_output = store
            .data_mut()
            .resource_table
            .push(sender)
            .expect("Failed to push sender to resource table");
        let input = serde_json::to_vec(&input).expect("Json is always serializable");
        let bindings = self.instantiate_async(&mut store).await.map_err(|e| {
            error!(parent: tracing_context.span(), "Failed to instantiate skill: {}", e);
            SkillError::RuntimeError(e)
        })?;
        bindings
            .pharia_skill_message_stream()
            .call_run(store, &input, stream_output)
            .await
            .map_err(|e| {
                error!(parent: tracing_context.span(), "Failed to execute skill handler: {}", e);
                SkillError::RuntimeError(e)
            })?
            .map_err(|e| match e {
                exports::pharia::skill::message_stream::Error::Internal(e) => {
                    SkillError::UserCode(e)
                }
                exports::pharia::skill::message_stream::Error::InvalidInput(e) => {
                    SkillError::InvalidInput(e.clone())
                }
            })
    }
}

impl Host for LinkedCtx {}

impl HostStreamOutput for LinkedCtx {
    async fn write(&mut self, output: Resource<StreamOutput>, item: MessageItem) {
        debug_assert!(!output.owned());
        let sender = self
            .resource_table
            .get(&output)
            .inspect_err(|e| error!("Failed to push stream to resource table: {e:#}"))
            .expect("Failed to push stream to resource table");
        let event = match item {
            MessageItem::MessageBegin(_) => SkillEvent::MessageBegin,
            MessageItem::MessageAppend(text) => SkillEvent::MessageAppend { text },
            MessageItem::MessageEnd(payload) => match payload {
                Some(payload) => match serde_json::from_slice(&payload) {
                    Ok(payload) => SkillEvent::MessageEnd { payload },
                    Err(e) => SkillEvent::InvalidBytesInPayload {
                        message: e.to_string(),
                    },
                },
                None => SkillEvent::MessageEnd {
                    payload: Value::Null,
                },
            },
        };
        drop(sender.send(event).await);
    }

    async fn drop(&mut self, output: Resource<StreamOutput>) -> anyhow::Result<()> {
        debug_assert!(output.owned());
        self.resource_table
            .delete(output)
            .inspect_err(|e| error!("Failed to delete stream from resource table: {e:#}"))
            .expect("Failed to delete stream from resource table");
        Ok(())
    }
}
