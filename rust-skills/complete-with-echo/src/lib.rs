use anyhow::anyhow;
use exports::pharia::skill::skill_handler::{Error, Guest, SkillMetadata};
use pharia::skill::inference::{CompletionParamsV2, CompletionRequestV2, Logprobs, complete_v2};
use serde_json::json;

wit_bindgen::generate!({ path: "../../pharia-engine/wit/skill@0.3", world: "skill" });

/// A skill that completes a prompt with echo enabled.
struct Skill;

impl Guest for Skill {
    fn run(_input: Vec<u8>) -> Result<Vec<u8>, Error> {
        let prompt = "An apple a day".to_owned();
        let result = complete_v2(&[CompletionRequestV2 {
            model: "pharia-1-llm-7b-control".to_owned(),
            prompt,
            params: CompletionParamsV2 {
                echo: true,
                return_special_tokens: true,
                max_tokens: None,
                temperature: None,
                top_k: None,
                top_p: None,
                frequency_penalty: None,
                presence_penalty: None,
                stop: vec![],
                logprobs: Logprobs::No,
            },
        }])
        .remove(0);
        let output = serde_json::to_vec(&json!(result.text))
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
