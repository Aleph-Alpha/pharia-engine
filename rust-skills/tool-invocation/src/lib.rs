use exports::pharia::skill::skill_handler::{Error, Guest, SkillMetadata};
use pharia::skill::tool::{Argument, InvokeRequest, invoke_tool};

wit_bindgen::generate!({ path: "../../wit/skill@0.3", world: "skill", features: ["tool"] });

struct Skill;

impl Guest for Skill {
    fn run(input: Vec<u8>) -> Result<Vec<u8>, Error> {
        let name = serde_json::from_slice::<String>(&input).unwrap();
        let request = InvokeRequest {
            tool_name: "current_weather".to_owned(),
            arguments: vec![Argument {
                name: "city".to_owned(),
                value: name.as_bytes().to_vec(),
            }],
        };
        let result = invoke_tool(&[request]).pop().unwrap();
        Ok(result)
    }

    fn metadata() -> SkillMetadata {
        SkillMetadata {
            description: None,
            input_schema: vec![],
            output_schema: vec![],
        }
    }
}

export!(Skill);
