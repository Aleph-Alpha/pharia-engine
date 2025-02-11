// Allow because it is part of the bindgen generated code
#![expect(unsafe_op_in_unsafe_fn)]

use anyhow::anyhow;
use exports::pharia::skill::skill_handler::{Error, Guest, SkillMetadata};
use pharia::skill::document_index::{documents, search, DocumentPath, IndexPath, SearchRequest};
use serde_json::json;

wit_bindgen::generate!({ path: "../../wit/skill@0.3", world: "skill" });

struct Skill;

impl Guest for Skill {
    fn run(input: Vec<u8>) -> Result<Vec<u8>, Error> {
        let query = serde_json::from_slice::<String>(&input)
            .map_err(|e| Error::InvalidInput(anyhow!(e).to_string()))?;
        let index_path = IndexPath {
            namespace: "Kernel".to_owned(),
            collection: "test".to_owned(),
            index: "asym-64".to_owned(),
        };
        let request = SearchRequest {
            index_path,
            query,
            max_results: 10,
            min_score: None,
            filters: Vec::new(),
        };
        let mut results = search(&[request]);
        let results = results
            .remove(0)
            .iter()
            .map(|r| r.content.clone())
            .collect::<Vec<_>>();
        let output = serde_json::to_vec(&json!(results))
            .map_err(|e| Error::Internal(anyhow!(e).to_string()))?;

        // Test invocation of documents
        let request = DocumentPath {
            namespace: "Kernel".to_owned(),
            collection: "test".to_owned(),
            name: "kernel-docs".to_owned(),
        };
        let result = documents(&[request]);
        assert!(!result.is_empty());
        Ok(output)
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
