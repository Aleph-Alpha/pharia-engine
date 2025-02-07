use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::csi_shell::CsiShellError;
use crate::{
    chunking::ChunkRequest,
    csi::Csi,
    inference::{ChatRequest, CompletionRequest},
    language_selection::SelectLanguageRequest,
    search::{DocumentPath, SearchRequest},
};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "function")]
pub enum V0_3CsiRequest {
    Chunk {
        requests: Vec<ChunkRequest>,
    },
    SelectLanguage {
        requests: Vec<SelectLanguageRequest>,
    },
    Complete {
        requests: Vec<CompletionRequest>,
    },
    Search {
        requests: Vec<SearchRequest>,
    },
    Chat {
        requests: Vec<ChatRequest>,
    },
    Documents {
        requests: Vec<DocumentPath>,
    },
    DocumentMetadata {
        requests: Vec<DocumentPath>,
    },
    #[serde(untagged)]
    Unknown {
        function: Option<String>,
    },
}

impl V0_3CsiRequest {
    pub async fn act<C>(self, drivers: &C, auth: String) -> Result<Value, CsiShellError>
    where
        C: Csi + Sync,
    {
        let result = match self {
            V0_3CsiRequest::Chunk { requests } => {
                drivers.chunk(auth, requests).await.map(|r| json!(r))?
            }
            V0_3CsiRequest::SelectLanguage { requests } => {
                drivers.select_language(requests).await.map(|r| json!(r))?
            }
            V0_3CsiRequest::Complete { requests } => drivers
                .complete(auth, requests.into_iter().map(Into::into).collect())
                .await
                .map(|v| json!(v))?,
            V0_3CsiRequest::Search { requests } => {
                drivers.search(auth, requests).await.map(|v| json!(v))?
            }
            V0_3CsiRequest::Chat { requests } => {
                drivers.chat(auth, requests).await.map(|v| json!(v))?
            }
            V0_3CsiRequest::DocumentMetadata { requests } => drivers
                .document_metadata(auth, requests)
                .await
                .map(|r| json!(r))?,
            V0_3CsiRequest::Documents { requests } => {
                drivers.documents(auth, requests).await.map(|r| json!(r))?
            }
            V0_3CsiRequest::Unknown { function } => {
                return Err(CsiShellError::UnknownFunction(
                    function.unwrap_or_else(|| "specified".to_owned()),
                ));
            }
        };
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::csi_shell::VersionedCsiRequest;

    #[test]
    fn csi_v_3_request_is_deserialized() {
        // Given a request in JSON format
        let request = json!({
            "version": "0.3",
            "function": "complete",
            "prompt": "Hello",
            "model": "pharia-1-llm-7b-control",
            "params": {
                "return_special_tokens": true,
                "max_tokens": 128,
                "temperature": null,
                "top_k": null,
                "top_p": null,
                "stop": []
            }
        });

        // When it is deserialized into a `VersionedCsiRequest`
        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        // Then it should be deserialized successfully
        assert!(result.is_ok());
    }

    #[test]
    fn csi_v_3_documents_request_is_deserialized() {
        // Given a request in JSON format
        let request = json!({
            "version": "0.3",
            "function": "documents",
            "requests": [
                {
                    "namespace": "Kernel",
                    "collection": "test",
                    "name": "kernel-docs"
                }
            ]
        });

        // When it is deserialized into a `VersionedCsiRequest`
        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        // Then it should be deserialized successfully
        assert!(
            matches!(result, Ok(VersionedCsiRequest::V0_3(V0_3CsiRequest::Documents { requests })) if requests.len() == 1)
        );
    }

    #[test]
    fn csi_v_3_metadata_request_is_deserialized() {
        // Given a request in JSON format
        let request = json!({
            "version": "0.3",
            "function": "document_metadata",
            "requests": [
                {
                    "namespace": "Kernel",
                    "collection": "test",
                    "name": "asym-64"
                },
                {
                    "namespace": "Kernel",
                    "collection": "test",
                    "name": "asym-64"
                }
            ]
        });

        // When it is deserialized into a `VersionedCsiRequest`
        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        // Then it should be deserialized successfully
        assert!(
            matches!(result, Ok(VersionedCsiRequest::V0_3(V0_3CsiRequest::DocumentMetadata { requests })) if requests.len() == 2)
        );
    }
}
