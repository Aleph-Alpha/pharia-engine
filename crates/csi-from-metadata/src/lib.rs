// Allow because it is part of the bindgen generated code
#![expect(unsafe_op_in_unsafe_fn)]

use exports::pharia::skill::skill_handler::{Error, Guest, SkillMetadata};
use pharia::skill::language::{SelectLanguageRequest, select_language};

wit_bindgen::generate!({ path: "../../wit/skill@0.3", world: "skill" });

struct Skill;

impl Guest for Skill {
    fn run(_input: Vec<u8>) -> Result<Vec<u8>, Error> {
        unimplemented!()
    }

    // While the SDK does not offer to use the `Csi` from the metadata function,
    // a developer could still produce a valid component that does this.
    // Therefore we want a component that tests `Csi` usage from the metadata function.
    fn metadata() -> SkillMetadata {
        let request = SelectLanguageRequest {
            text: "Hello".to_owned(),
            languages: Vec::new(),
        };
        select_language(&[request]);
        unreachable!("Skill metadata execution should be suspended.")
    }
}

export!(Skill);
