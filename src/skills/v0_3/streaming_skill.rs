use tokio::sync::mpsc;
use wasmtime::component::bindgen;

pub type StreamOutputSender = mpsc::Sender<()>;

bindgen!({
    world: "streaming-skill",
    path: "./wit/skill@0.3",
    async: true,
    with: {
        "pharia:skill/chunking": super::csi::pharia::skill::chunking,
        "pharia:skill/document-index": super::csi::pharia::skill::document_index,
        "pharia:skill/inference": super::csi::pharia::skill::inference,
        "pharia:skill/language": super::csi::pharia::skill::language,
        "pharia:skill/streaming-host/stream-output": StreamOutputSender,
    },
});
