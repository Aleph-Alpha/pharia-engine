use anyhow::anyhow;
use exports::pharia::skill::skill_handler::{Error, Guest, SkillMetadata};
use pharia::skill::inference::{ExplanationRequest, Granularity, TextScore, explain};
use serde::{Deserialize, Serialize};

wit_bindgen::generate!({path: "../../pharia-engine/wit/skill@0.3", world: "skill"});

struct Skill;

#[derive(Serialize)]
struct SkillOutput {
    start: u32,
    length: u32,
}

impl From<&TextScore> for SkillOutput {
    fn from(value: &TextScore) -> Self {
        let TextScore { start, length, .. } = value;
        Self {
            start: *start,
            length: *length,
        }
    }
}

#[derive(Deserialize)]
struct SkillInput {
    prompt: String,
    target: String,
}

impl Guest for Skill {
    fn run(input: Vec<u8>) -> Result<Vec<u8>, Error> {
        let input: SkillInput = serde_json::from_slice(&input)
            .map_err(|e| Error::InvalidInput(anyhow!(e).to_string()))?;
        let request = ExplanationRequest {
            prompt: input.prompt,
            target: input.target,
            model: "pharia-1-llm-7b-control".to_string(),
            granularity: Granularity::Auto,
        };
        let explanation = explain(&[request]).remove(0);
        let items = explanation
            .iter()
            .map(SkillOutput::from)
            .collect::<Vec<_>>();
        serde_json::to_vec(&items).map_err(|e| Error::Internal(anyhow!(e).to_string()))
    }

    fn metadata() -> SkillMetadata {
        unimplemented!("We are testing explanation, not metadata.")
    }
}

export!(Skill);
