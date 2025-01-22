// Allow because it is part of the bindgen generated code
#![expect(unsafe_op_in_unsafe_fn)]

use anyhow::anyhow;
use exports::pharia::skill::skill_handler::{Error, Guest};
use pharia::skill::csi::{documents, search, DocumentPath, IndexPath};
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
        let results = search(&index_path, &query, 10, None);
        let results = results
            .iter()
            .map(|r| r.content.clone())
            .collect::<Vec<_>>();
        let output = serde_json::to_vec(&json!(results))
            .map_err(|e| Error::Internal(anyhow!(e).to_string()))?;

        // Test invocation of documents
        let request = DocumentPath {
            namespace: "Kernel".to_owned(),
            collection: "test".to_owned(),
            name: "docs".to_owned(),
        };
        let result = documents(&[request]);
        assert!(result.is_empty());
        Ok(output)
    }
}

export!(Skill);
