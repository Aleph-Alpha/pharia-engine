// Allow because it is part of the bindgen generated code
#![expect(unsafe_op_in_unsafe_fn)]

use exports::pharia::skill::skill_handler::{Error, Guest, SkillMetadata};

wit_bindgen::generate!({ path: "../../wit/skill@0.3", world: "skill" });

struct Skill;

impl Guest for Skill {
    fn run(_input: Vec<u8>) -> Result<Vec<u8>, Error> {
        Ok(b"{".to_vec())
    }

    fn metadata() -> SkillMetadata {
        SkillMetadata {
            description: None,
            input_schema: b"{".to_vec(),
            output_schema: b"{".to_vec(),
        }
    }
}

export!(Skill);
