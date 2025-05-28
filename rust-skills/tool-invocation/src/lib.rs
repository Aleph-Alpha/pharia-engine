use exports::pharia::skill::skill_handler::{Error, Guest, SkillMetadata};
use pharia::skill::tool::{Argument, InvokeRequest, invoke_tool};
use serde::Deserialize;
use serde_json::json;

wit_bindgen::generate!({ path: "../../wit/skill@0.3", world: "skill", features: ["tool"] });

#[derive(Deserialize)]
struct Arguments {
    a: i32,
    b: i32,
}

struct Skill;

impl Guest for Skill {
    fn run(input: Vec<u8>) -> Result<Vec<u8>, Error> {
        let arguments = serde_json::from_slice::<Arguments>(&input).unwrap();
        let request = InvokeRequest {
            tool_name: "add".to_owned(),
            arguments: vec![
                Argument {
                    name: "a".to_owned(),
                    value: json!(arguments.a).to_string().into_bytes(),
                },
                Argument {
                    name: "b".to_owned(),
                    value: json!(arguments.b).to_string().into_bytes(),
                },
            ],
        };
        let result = invoke_tool(&[request]).unwrap().pop().unwrap();
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
