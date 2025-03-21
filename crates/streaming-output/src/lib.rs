use anyhow::anyhow;
use exports::pharia::skill::streaming_skill_handler::{Error, Guest};
use pharia::skill::{
    inference::{ChatEvent, ChatParams, ChatRequest, ChatStream, Logprobs, Message, MessageDelta},
    streaming_host::{BeginAttributes, MessageItem, StreamOutput},
};
use serde_json::json;

wit_bindgen::generate!({ path: "../../wit/skill@0.3", world: "streaming-skill", features: ["streaming"] });

struct Skill;

impl Guest for Skill {
    fn run(input: Vec<u8>, stream_output: StreamOutput) -> Result<(), Error> {
        let query = serde_json::from_slice::<String>(&input)
            .map_err(|e| Error::InvalidInput(anyhow!(e).to_string()))?;

        let model = "pharia-1-llm-7b-control".to_owned();
        let messages = vec![Message {
            role: "user".to_owned(),
            content: query,
        }];
        let params = ChatParams {
            max_tokens: Some(10),
            temperature: None,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            logprobs: Logprobs::No,
        };
        let request = ChatRequest {
            model,
            messages,
            params,
        };

        let stream = ChatStream::new(&request);

        while let Some(event) = stream.next() {
            let item = match event {
                ChatEvent::MessageStart(role) => Some(MessageItem::MessageBegin(BeginAttributes {
                    role: Some(role),
                })),
                ChatEvent::MessageDelta(MessageDelta { content, .. }) => {
                    Some(MessageItem::MessageAppend(content))
                }
                ChatEvent::MessageEnd(finish_reason) => Some(MessageItem::MessageEnd(Some(
                    serde_json::to_vec(&json!(format!("{finish_reason:?}")))
                        .map_err(|e| Error::Internal(anyhow!(e).to_string()))?,
                ))),
                ChatEvent::Usage(_) => None,
            };

            if let Some(item) = item {
                stream_output.write_message_item(&item);
            }
        }

        Ok(())
    }
}

export!(Skill);
