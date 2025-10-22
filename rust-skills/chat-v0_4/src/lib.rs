use anyhow::anyhow;
use exports::pharia::skill::skill_handler::{Error, Guest, SkillMetadata};
use pharia::skill::inference::{ChatParams, ChatRequest, Logprobs, Message, OtherMessage, chat};
use serde_json::json;

wit_bindgen::generate!({ path: "../../pharia-engine/wit/skill@0.4", world: "skill"});

struct Skill;

impl Guest for Skill {
    fn run(input: Vec<u8>) -> Result<Vec<u8>, Error> {
        let query = serde_json::from_slice::<String>(&input)
            .map_err(|e| Error::InvalidInput(anyhow!(e).to_string()))?;

        let model = "pharia-1-llm-7b-control".to_owned();
        let messages = vec![Message::Other(OtherMessage {
            role: "user".to_owned(),
            content: query,
        })];
        let params = ChatParams {
            max_tokens: None,
            max_completion_tokens: None,
            temperature: None,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            logprobs: Logprobs::No,
            tools: None,
            tool_choice: None,
            parallel_tool_calls: None,
            response_format: None,
            reasoning_effort: None,
        };

        let request = ChatRequest {
            model,
            messages,
            params,
        };

        let response = chat(&[request]).remove(0);
        let message = response.message;
        let json = json!({
            "content": message.content,
            "role": "assistant"
        });

        let output =
            serde_json::to_vec(&json).map_err(|e| Error::Internal(anyhow!(e).to_string()))?;
        Ok(output)
    }

    fn metadata() -> SkillMetadata {
        SkillMetadata {
            description: Some(
                "A chat skill that uses the pharia-1-llm-7b-control model".to_owned(),
            ),
            input_schema: serde_json::to_vec(&json!({
                "model": "pharia-1-llm-7b-control",
                "query": "string"
            }))
            .unwrap(),
            output_schema: serde_json::to_vec(&json!({
                "content": "string",
                "role": "string"
            }))
            .unwrap(),
        }
    }
}

export!(Skill);
