// Allow because it is part of the bindgen generated code
#![expect(unsafe_op_in_unsafe_fn, unsafe_attr_outside_unsafe)]

use anyhow::anyhow;
use exports::pharia::skill::skill_handler::{Error, Guest};
use pharia::skill::csi::{chat, Message, ChatParams, Role};
use serde_json::json;

wit_bindgen::generate!({ path: "../../wit/skill@0.2", world: "skill" });

struct Skill;

impl Guest for Skill {
    fn run(input: Vec<u8>) -> Result<Vec<u8>, Error> {
        let query = serde_json::from_slice::<String>(&input)
            .map_err(|e| Error::InvalidInput(anyhow!(e).to_string()))?;

        let model = "pharia-1-llm-7b-control".to_owned();
        let messages = vec![Message {
            role: Role::User,
            content: query,
        }];
        let params = ChatParams {
            max_tokens: None,
            temperature: None,
            top_k: None,
            top_p: None,
        };

        let response = chat(&model, &messages, params);
        let json = json!({
            "content": response.message.content,
            "role": match response.message.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::System => "system",
            },
        });

        let output = serde_json::to_vec(&json)
            .map_err(|e| Error::Internal(anyhow!(e).to_string()))?;
        Ok(output)
    }
}

export!(Skill);
