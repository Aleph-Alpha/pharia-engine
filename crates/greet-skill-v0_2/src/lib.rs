// Allow because it is part of the bindgen generated code
#![allow(unsafe_op_in_unsafe_fn)]

use anyhow::anyhow;
use exports::pharia::skill::skill_handler::{Error, Guest};
use pharia::skill::csi::{complete, CompletionParams};
use serde_json::json;

wit_bindgen::generate!({ path: "../../wit/skill@0.2", world: "skill" });

struct Skill;

impl Guest for Skill {
    fn run(input: Vec<u8>) -> Result<Vec<u8>, Error> {
        let name = serde_json::from_slice::<String>(&input)
            .map_err(|e| Error::InvalidInput(anyhow!(e).to_string()))?;
        let prompt = format!(
            "<|begin_of_text|><|start_header_id|>system<|end_header_id|>

Cutting Knowledge Date: December 2023
Today Date: 23 Jul 2024

You are a helpful assistant.<|eot_id|><|start_header_id|>user<|end_header_id|>

Provide a nice greeting for the person named: {name}<|eot_id|><|start_header_id|>assistant<|end_header_id|>"
        );
        let result = complete(
            "llama-3.1-8b-instruct",
            &prompt,
            &CompletionParams {
                max_tokens: None,
                temperature: None,
                top_k: None,
                top_p: None,
                stop: vec![
                    "<|start_header_id|>".to_owned(),
                    "<|eom_id|>".to_owned(),
                    "<|eot_id|>".to_owned(),
                ],
            },
        );
        let output = serde_json::to_vec(&json!(result.text))
            .map_err(|e| Error::Internal(anyhow!(e).to_string()))?;
        Ok(output)
    }
}

export!(Skill);
