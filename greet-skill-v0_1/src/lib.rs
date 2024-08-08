// Allow because it is part of the bindgen generated code
#![allow(unsafe_op_in_unsafe_fn)]

use pharia::skill::csi::complete;
use serde_json::json;

wit_bindgen::generate!({ path: "../wit/skill@0.1.0", world: "skill" });

struct Skill;

impl Skill {
    fn run(input: &[u8]) -> anyhow::Result<Vec<u8>> {
        let name = serde_json::from_slice::<String>(input)?;
        let prompt = format!(
            "### Instruction:
    Provide a nice greeting for the person utilizing its given name

    ### Input:
    Name: {name}

    ### Response:"
        );
        let result = complete("luminous-nextgen-7b", &prompt, None)?;
        Ok(serde_json::to_vec(&json!(result.text))?)
    }
}

impl Guest for Skill {
    fn run(input: Vec<u8>) -> Result<Vec<u8>, String> {
        Self::run(&input).map_err(|e| e.to_string())
    }
}

export!(Skill);
