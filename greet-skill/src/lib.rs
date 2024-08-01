// Allow because it is part of the bindgen generated code
#![allow(unsafe_op_in_unsafe_fn)]
use pharia::skill::csi::complete_text;

wit_bindgen::generate!({ path: "../wit/skill@unversioned", world: "skill" });

struct Skill {}

impl Guest for Skill {
    fn run(name: String) -> String {
        let prompt = format!(
            "### Instruction:
Provide a nice greeting for the person utilizing its given name

### Input:
Name: {name}

### Response:"
        );
        complete_text(&prompt, "luminous-nextgen-7b")
    }
}

export!(Skill);

#[cfg(test)]
mod tests {
    // use super::*;
}
