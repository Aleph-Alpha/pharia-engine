use anyhow::anyhow;
use exports::pharia::skill::skill_handler::{Error, Guest};
use pharia::skill::csi::{DocumentPath, document_metadata};

wit_bindgen::generate!({ path: "../../wit/skill@0.2", world: "skill" });

struct Skill;

impl Guest for Skill {
    fn run(input: Vec<u8>) -> Result<Vec<u8>, Error> {
        // initially ignore input, may access one key in a dict?
        let _query = serde_json::from_slice::<String>(&input)
            .map_err(|e| Error::InvalidInput(anyhow!(e).to_string()))?;
        let document_path = DocumentPath {
            namespace: "Kernel".to_owned(),
            collection: "test".to_owned(),
            name: "kernel-docs".to_owned(),
        };
        let results = document_metadata(&document_path);
        match results {
            None => Ok(vec![]),
            Some(value) => Ok(value),
        }
    }
}

export!(Skill);
