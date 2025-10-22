use anyhow::anyhow;
use exports::pharia::skill::skill_handler::{Error, Guest, SkillMetadata};
use pharia::skill::inference::{
    CompletionAppend, CompletionEvent, CompletionParams, CompletionRequest, CompletionStream,
    Logprobs,
};
use serde_json::json;

wit_bindgen::generate!({ path: "../../pharia-engine/wit/skill@0.3", world: "skill" });

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
        let request = CompletionRequest {
            model: "pharia-1-llm-7b-control".to_owned(),
            prompt,
            params: CompletionParams {
                return_special_tokens: true,
                max_tokens: None,
                temperature: None,
                top_k: None,
                top_p: None,
                frequency_penalty: None,
                presence_penalty: None,
                stop: vec![
                    "<|start_header_id|>".to_owned(),
                    "<|eom_id|>".to_owned(),
                    "<|eot_id|>".to_owned(),
                ],
                logprobs: Logprobs::No,
            },
        };
        let stream = CompletionStream::new(&request);

        let mut events = vec![];
        while let Some(event) = stream.next() {
            events.push(match event {
                CompletionEvent::Append(CompletionAppend { text, .. }) => text,
                CompletionEvent::End(finish_reason) => format!("{finish_reason:?}"),
                CompletionEvent::Usage(token_usage) => format!(
                    "prompt: {}, completion: {}",
                    token_usage.prompt, token_usage.completion
                ),
            });
        }

        let output = serde_json::to_vec(&json!(events))
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
