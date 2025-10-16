use anyhow::anyhow;
use exports::pharia::skill::skill_handler::{Error, Guest, SkillMetadata};
use serde_json::json;

wit_bindgen::generate!({ path: "../../wit/skill@0.5", world: "skill", features: ["alpha"] });

struct Skill;

impl Guest for Skill {
    fn run(input: Vec<u8>) -> Result<Vec<u8>, Error> {
        let input_str =
            String::from_utf8(input).map_err(|e| Error::InvalidInput(anyhow!(e).to_string()))?;

        let output = json!({
            "message": format!("Test skill v0.5 received: {}", input_str)
        });

        serde_json::to_vec(&output).map_err(|e| Error::Internal(anyhow!(e).to_string()))
    }

    fn metadata() -> SkillMetadata {
        SkillMetadata {
            description: Some("A test skill for the 0.5 wit world".to_owned()),
            input_schema: serde_json::to_vec(&json!({
                "type": "string"
            }))
            .unwrap(),
            output_schema: serde_json::to_vec(&json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string" }
                }
            }))
            .unwrap(),
        }
    }
}

export!(Skill);
