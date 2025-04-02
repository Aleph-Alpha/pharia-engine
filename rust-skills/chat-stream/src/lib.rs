use anyhow::anyhow;
use exports::pharia::skill::skill_handler::{Error, Guest, SkillMetadata};
use pharia::skill::inference::{
    ChatEvent, ChatParams, ChatRequest, ChatStream, Logprobs, Message, MessageAppend,
};
use serde_json::json;

wit_bindgen::generate!({ path: "../../wit/skill@0.3", world: "skill", features: ["streaming"] });

struct Skill;

impl Guest for Skill {
    fn run(input: Vec<u8>) -> Result<Vec<u8>, Error> {
        let query = serde_json::from_slice::<String>(&input)
            .map_err(|e| Error::InvalidInput(anyhow!(e).to_string()))?;

        let model = "pharia-1-llm-7b-control".to_owned();
        let messages = vec![Message {
            role: "user".to_owned(),
            content: query,
        }];
        let params = ChatParams {
            max_tokens: None,
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

        let mut events = vec![];
        while let Some(event) = stream.next() {
            events.push(match event {
                ChatEvent::MessageBegin(role) => role,
                ChatEvent::MessageAppend(MessageAppend { content, .. }) => content,
                ChatEvent::MessageEnd(finish_reason) => format!("{finish_reason:?}"),
                ChatEvent::Usage(token_usage) => format!(
                    "prompt: {}, completion: {}",
                    token_usage.prompt, token_usage.completion
                ),
            });
        }

        let output = serde_json::to_vec(&json!(events))
            .map_err(|e| Error::Internal(anyhow!(e).to_string()))?;
        Ok(output)
    }

    fn metadata() -> SkillMetadata {
        SkillMetadata {
            description: Some("A friendly greeting skill".to_owned()),
            input_schema: serde_json::to_vec(&json!({
                "type": "string",
                "description": "The name of the person to greet",
            }))
            .unwrap(),
            output_schema: serde_json::to_vec(&json!({
                "type": "string",
                "description": "A friendly greeting message"
            }))
            .unwrap(),
        }
    }
}

export!(Skill);
