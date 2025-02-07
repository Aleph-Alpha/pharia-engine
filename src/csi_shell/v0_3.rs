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
pub enum CsiRequest {
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

impl CsiRequest {
    pub async fn act<C>(self, drivers: &C, auth: String) -> Result<Value, CsiShellError>
    where
        C: Csi + Sync,
    {
        let result = match self {
            CsiRequest::Chunk { requests } => {
                drivers.chunk(auth, requests).await.map(|r| json!(r))?
            }
            CsiRequest::SelectLanguage { requests } => {
                drivers.select_language(requests).await.map(|r| json!(r))?
            }
            CsiRequest::Complete { requests } => drivers
                .complete(auth, requests.into_iter().map(Into::into).collect())
                .await
                .map(|v| json!(v))?,
            CsiRequest::Search { requests } => {
                drivers.search(auth, requests).await.map(|v| json!(v))?
            }
            CsiRequest::Chat { requests } => {
                drivers.chat(auth, requests).await.map(|v| json!(v))?
            }
            CsiRequest::DocumentMetadata { requests } => drivers
                .document_metadata(auth, requests)
                .await
                .map(|r| json!(r))?,
            CsiRequest::Documents { requests } => {
                drivers.documents(auth, requests).await.map(|r| json!(r))?
            }
            CsiRequest::Unknown { function } => {
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
    fn chunk_request() {
        let request = json!({
            "version": "0.3",
            "function": "chunk",
            "requests": [
                {
                    "text": "Hello",
                    "params": {
                        "model": "pharia-1-llm-7b-control",
                        "max_tokens": 128
                    }
                },
                {
                    "text": "Hello",
                    "params": {
                        "model": "pharia-1-llm-7b-control",
                        "max_tokens": 128
                    }
                }
            ]
        });

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(matches!(
            result,
            Ok(VersionedCsiRequest::V0_3(CsiRequest::Chunk { requests })) if requests.len() == 2
        ));
    }

    #[test]
    fn select_language_request() {
        let request = json!({
            "version": "0.3",
            "function": "select_language",
            "requests": [
                {
                    "text": "Hello",
                    "languages": ["eng", "deu"]
                },
                {
                    "text": "Hello",
                    "languages": ["eng", "deu"]
                }
            ]
        });

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(matches!(
            result,
            Ok(VersionedCsiRequest::V0_3(CsiRequest::SelectLanguage { requests })) if requests.len() == 2
        ));
    }
    #[test]
    fn complete_request() {
        let request = json!({
            "version": "0.3",
            "function": "complete",
            "requests": [
                {
                    "prompt": "Hello",
                    "model": "pharia-1-llm-7b-control",
                    "params": {
                        "return_special_tokens": true,
                        "max_tokens": 128,
                        "temperature": null,
                        "top_k": null,
                        "top_p": null,
                        "stop": [],
                        "frequency_penalty": null,
                        "presence_penalty": null,
                        "logprobs": "no"
                    }
                },
                {
                    "prompt": "Hello",
                    "model": "pharia-1-llm-7b-control",
                    "params": {
                        "return_special_tokens": true,
                        "max_tokens": 128,
                        "temperature": null,
                        "top_k": null,
                        "top_p": null,
                        "stop": [],
                        "frequency_penalty": null,
                        "presence_penalty": null,
                        "logprobs": {
                            "top": 10
                        }
                    }
                }
            ]
        });

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(matches!(
            result,
            Ok(VersionedCsiRequest::V0_3(CsiRequest::Complete { requests })) if requests.len() == 2
        ));
    }

    #[test]
    fn search_request() {
        let request = json!({
            "version": "0.3",
            "function": "search",
            "requests": [
                {
                    "query": "Hello",
                    "index_path": {
                        "namespace": "Kernel",
                        "collection": "test",
                        "index": "asym-64"
                    },
                    "max_results": 10,
                    "min_score": null
                },
                {
                    "query": "Hello",
                    "index_path": {
                        "namespace": "Kernel",
                        "collection": "test",
                        "index": "asym-64"
                    },
                    "max_results": 10,
                    "min_score": null
                }
            ]
        });

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(matches!(
            result,
            Ok(VersionedCsiRequest::V0_3(CsiRequest::Search { requests })) if requests.len() == 2
        ));
    }

    #[test]
    fn chat_request() {
        let request = json!({
            "version": "0.3",
            "function": "chat",
            "requests": [
                {
                    "model": "pharia-1-llm-7b-control",
                    "messages": [
                        {
                            "role": "user",
                            "content": "Hello"
                        }
                    ],
                    "params": {
                        "max_tokens": 128,
                        "temperature": null,
                        "top_k": null,
                        "top_p": null,
                        "stop": [],
                        "logprobs": "no"
                    },
                }
            ]
        });

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(matches!(
            result,
            Ok(VersionedCsiRequest::V0_3(CsiRequest::Chat { requests })) if requests.len() == 1
        ));
    }

    #[test]
    fn documents_request() {
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

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(
            matches!(result, Ok(VersionedCsiRequest::V0_3(CsiRequest::Documents { requests })) if requests.len() == 1)
        );
    }

    #[test]
    fn document_metadata_request() {
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

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(
            matches!(result, Ok(VersionedCsiRequest::V0_3(CsiRequest::DocumentMetadata { requests })) if requests.len() == 2)
        );
    }

    #[test]
    fn unknown_request() {
        let request = json!({
            "version": "0.3",
            "function": "whatever",
        });

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(
            matches!(result, Ok(VersionedCsiRequest::V0_3(CsiRequest::Unknown { function: Some(function) })) if function == "whatever")
        );
    }
}
