use std::future::Future;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::HttpClient;

pub trait SearchClient: Send + Sync + 'static {
    fn search(
        &self,
        index: IndexPath,
        request: SearchRequest,
        api_token: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<SearchResult>>> + Send;

    fn document_metadata(
        &self,
        document_path: DocumentPath,
        api_token: &str,
    ) -> impl Future<Output = anyhow::Result<Option<Value>>> + Send;

    fn document(
        &self,
        document_path: DocumentPath,
        api_token: &str,
    ) -> impl Future<Output = anyhow::Result<Document>> + Send;
}

/// Search a Document Index collection
pub struct SearchRequest {
    /// What you want to search for
    query: Vec<Modality>,
    /// The maximum number of results to return. Defaults to 1
    max_results: u32,
    /// The minimum score each result should have to be returned.
    /// By default, all results are returned, up to the `max_results`.
    min_score: Option<f64>,
    /// Whether only text chunks should be returned
    text_only: bool,
}

impl SearchRequest {
    pub fn new(
        query: Vec<Modality>,
        max_results: u32,
        min_score: Option<f64>,
        text_only: bool,
    ) -> Self {
        Self {
            query,
            max_results,
            min_score,
            text_only,
        }
    }
}

/// Which documents you want to search in, and which type of index should be used
#[derive(Clone, Debug)]
pub struct IndexPath {
    /// The namespace the collection belongs to
    pub namespace: String,
    /// The collection you want to search in
    pub collection: String,
    /// The search index you want to use for the collection
    pub index: String,
}

/// Modality of the search result in the API
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "modality")]
pub enum Modality {
    Text { text: String },
    Image { bytes: String },
}

/// A position within a document. The cursor is always inclusive of the current position, in both start and end positions.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", tag = "modality")]
#[expect(dead_code)]
pub enum Cursor {
    Text {
        /// Index of the item in the document
        item: usize,
        /// The character position the cursor can be found at within the string.
        position: usize,
    },
    Image {
        /// Index of the item in the document
        item: usize,
    },
}

/// The name of a given document
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DocumentPath {
    /// The namespace the collection belongs to
    pub namespace: String,
    /// The collection the document belongs to
    pub collection: String,
    /// The name of the document
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Document {
    pub path: DocumentPath,
    pub contents: Vec<Modality>,
    pub metadata: Option<Value>,
}

/// A result for a given search that comes back from the API
#[derive(Debug, Deserialize)]
pub struct SearchResult {
    pub document_path: DocumentPath,
    pub section: Vec<Modality>,
    pub score: f64,
    pub start: Cursor,
    pub end: Cursor,
}

/// Sends HTTP Request to Document Index API
pub struct Client {
    /// The base host to use for all API requests
    host: String,
    /// Shared client to reuse connections
    http: HttpClient,
}

impl Client {
    pub fn new(host: String) -> Self {
        Self {
            host,
            http: HttpClient::default(),
        }
    }
}

impl SearchClient for Client {
    async fn search(
        &self,
        index: IndexPath,
        request: SearchRequest,
        api_token: &str,
    ) -> anyhow::Result<Vec<SearchResult>> {
        let IndexPath {
            namespace,
            collection,
            index,
        } = index;
        let SearchRequest {
            query,
            max_results,
            min_score,
            text_only,
        } = request;

        let mut body = json!({
            "query": query,
            "max_results": max_results,
            "min_score": min_score,
        });
        if text_only {
            body["filters"] = json!([{ "with": [{ "modality": "text" }]}]);
        }

        // Namespaces, collections and indexes must match regex ^[a-zA-Z0-9\-\.]+$
        // therefore, we do not need to url encode them
        Ok(self
            .http
            .post(format!(
                "{}/collections/{namespace}/{collection}/indexes/{index}/search",
                &self.host
            ))
            .bearer_auth(api_token)
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    async fn document_metadata(
        &self,
        document_path: DocumentPath,
        api_token: &str,
    ) -> anyhow::Result<Option<Value>> {
        #[derive(Deserialize)]
        struct Document {
            metadata: Option<Value>,
        }

        let DocumentPath {
            namespace,
            collection,
            name,
        } = document_path;

        // Namespaces and collections must match regex ^[a-zA-Z0-9\-\.]+$
        // therefore, we do not need to url encode them
        // A document name can contain characters like `/`, which need to be url encoded
        let encoded_name = urlencoding::encode(&name);
        let document = self
            .http
            .get(format!(
                "{}/collections/{namespace}/{collection}/docs/{encoded_name}",
                &self.host
            ))
            .bearer_auth(api_token)
            .send()
            .await?
            .error_for_status()?
            .json::<Document>()
            .await?;

        Ok(document.metadata)
    }

    async fn document(
        &self,
        document_path: DocumentPath,
        api_token: &str,
    ) -> anyhow::Result<Document> {
        #[derive(Deserialize)]
        struct JsonDocument {
            contents: Vec<Modality>,
            metadata: Option<Value>,
        }

        let DocumentPath {
            namespace,
            collection,
            name,
        } = &document_path;

        // Namespaces and collections must match regex ^[a-zA-Z0-9\-\.]+$
        // therefore, we do not need to url encode them
        // A document name can contain characters like `/`, which need to be url encoded
        let encoded_name = urlencoding::encode(name);
        let document = self
            .http
            .get(format!(
                "{}/collections/{namespace}/{collection}/docs/{encoded_name}",
                &self.host
            ))
            .bearer_auth(api_token)
            .send()
            .await?
            .error_for_status()?
            .json::<JsonDocument>()
            .await?;

        Ok(Document {
            path: document_path,
            contents: document.contents,
            metadata: document.metadata,
        })
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    use crate::tests::{api_token, document_index_url};

    pub struct StubClient;

    impl Document {
        pub fn dummy() -> Self {
            Self {
                path: DocumentPath::new("Kernel", "test", "kernel-docs"),
                contents: vec![Modality::Text {
                    text: "Hello Homer".to_owned(),
                }],
                metadata: Some(json!({ "url": "http://example.de" })),
            }
        }
    }

    impl SearchClient for StubClient {
        async fn search(
            &self,
            _index: IndexPath,
            _request: SearchRequest,
            _api_token: &str,
        ) -> anyhow::Result<Vec<SearchResult>> {
            Ok(vec![])
        }

        async fn document_metadata(
            &self,
            _document_path: DocumentPath,
            _api_token: &str,
        ) -> anyhow::Result<Option<Value>> {
            Ok(None)
        }

        async fn document(
            &self,
            _document_path: DocumentPath,
            _api_token: &str,
        ) -> anyhow::Result<Document> {
            Ok(Document::dummy())
        }
    }

    impl IndexPath {
        pub fn new(
            namespace: impl Into<String>,
            collection: impl Into<String>,
            index: impl Into<String>,
        ) -> Self {
            Self {
                namespace: namespace.into(),
                collection: collection.into(),
                index: index.into(),
            }
        }
    }

    impl DocumentPath {
        pub fn new(
            namespace: impl Into<String>,
            collection: impl Into<String>,
            name: impl Into<String>,
        ) -> Self {
            Self {
                namespace: namespace.into(),
                collection: collection.into(),
                name: name.into(),
            }
        }
    }

    #[tokio::test]
    async fn document_exists() {
        // Given a search client pointed at the document index
        let host = document_index_url().to_owned();
        let api_token = api_token();
        let client = Client::new(host);

        // When requesting a document
        let document_path = DocumentPath::new("Kernel", "test", "kernel-docs");
        let document = client.document(document_path, api_token).await.unwrap();

        // Then we get the expected document
        assert!(!document.contents.is_empty());
    }

    #[tokio::test]
    async fn document_not_found_is_err() {
        // Given a search client pointed at the document index
        let host = document_index_url().to_owned();
        let api_token = api_token();
        let client = Client::new(host);

        // When requesting a document that does not exist
        let document_path = DocumentPath::new("Kernel", "test", "kernel-docs-not-found");
        let maybe_document = client.document(document_path, api_token).await;

        // Then we get no document
        assert!(maybe_document.is_err());
    }

    #[tokio::test]
    async fn search_request() {
        // Given a search client pointed at the document index
        let host = document_index_url().to_owned();
        let api_token = api_token();
        let client = Client::new(host);

        // When making a query on an existing collection
        let index = IndexPath::new("Kernel", "test", "asym-64");
        let request = SearchRequest::new(
            vec![Modality::Text {
                text: "What is the Pharia Kernel?".to_owned(),
            }],
            1,
            None,
            true,
        );
        let results = client.search(index, request, api_token).await.unwrap();

        // Then we get at least one result
        assert_eq!(results.len(), 1);
        assert!(results[0]
            .document_path
            .name
            .to_lowercase()
            .contains("kernel"));
        let Modality::Text { text } = &results[0].section[0] else {
            panic!("invalid entry");
        };
        assert!(text.contains("Kernel"));
    }

    #[tokio::test]
    async fn request_metadata() {
        // Given a search client pointed at the document index
        let host = document_index_url().to_owned();
        let api_token = api_token();
        let client = Client::new(host);

        // When requesting metadata of an existing document
        let document_path = DocumentPath::new("Kernel", "test", "kernel/docs");
        let maybe_metadata = client
            .document_metadata(document_path, api_token)
            .await
            .unwrap();

        // Then we get the expected metadata
        if let Some(metadata) = maybe_metadata {
            assert!(metadata.is_array());
            assert_eq!(
                metadata[0]["url"].as_str().unwrap(),
                "https://pharia-kernel.product.pharia.com/"
            );
        } else {
            panic!("metadata not found");
        }
    }

    #[tokio::test]
    async fn min_score() {
        // Given a search client pointed at the document index
        let host = document_index_url().to_owned();
        let api_token = api_token();
        let client = Client::new(host);
        let max_results = 5;
        let min_score = 0.99;

        // When making a query on an existing collection with a high min score
        let index = IndexPath::new("Kernel", "test", "asym-64");
        let request = SearchRequest::new(
            vec![Modality::Text {
                text: "What is the Pharia Kernel?".to_owned(),
            }],
            max_results,
            Some(min_score),
            true,
        );
        let results = client.search(index, request, api_token).await.unwrap();

        // Then we don't get any results
        assert!(results.is_empty());
    }
}
