// Allow because it is part of the bindgen generated code
#![expect(unsafe_op_in_unsafe_fn, unsafe_attr_outside_unsafe)]
use pharia::skill::csi::complete_text;

wit_bindgen::generate!({ path: "../../wit/skill@unversioned", world: "skill" });

struct Skill {}

impl Guest for Skill {
    fn run(name: String) -> String {
        let prompt = format!(
            "<|begin_of_text|><|start_header_id|>system<|end_header_id|>

Cutting Knowledge Date: December 2023
Today Date: 23 Jul 2024

You are a helpful assistant.<|eot_id|><|start_header_id|>user<|end_header_id|>

Provide a nice greeting for the person named: {name}<|eot_id|><|start_header_id|>assistant<|end_header_id|>"
        );
        complete_text(&prompt, "pharia-1-llm-7b-control")
    }
}

export!(Skill);
