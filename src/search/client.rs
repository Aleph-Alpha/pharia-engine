use std::future::Future;

use reqwest::ClientBuilder;
use serde::{Deserialize, Serialize};
use serde_json::json;

pub trait SearchClient: Send + Sync + 'static {
    fn search(
        &self,
        index: IndexPath,
        request: SearchRequest,
        api_token: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<SearchResult>>> + Send;
}

/// Search a Document Index collection
#[derive(Debug)]
pub struct SearchRequest {
    /// What you want to search for
    query: Vec<Modality>,
    /// The maximum number of results to return. Defaults to 1
    max_results: usize,
    /// The minimum score each result should have to be returned.
    /// By default, all results are returned, up to the `max_results`.
    min_score: Option<f64>,
    /// Whether only text chunks should be returned
    text_only: bool,
}

impl SearchRequest {
    pub fn new(
        query: Vec<Modality>,
        max_results: usize,
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
#[derive(Debug, Deserialize)]
#[expect(dead_code)]
pub struct DocumentPath {
    /// The namespace the collection belongs to
    pub namespace: String,
    /// The collection the document belongs to
    pub collection: String,
    /// The name of the document
    pub name: String,
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
    http: reqwest::Client,
}

impl Client {
    pub fn new(host: String) -> anyhow::Result<Self> {
        Ok(Self {
            host,
            http: ClientBuilder::new().build()?,
        })
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
}

#[cfg(test)]
pub mod tests {
    use super::*;

    use crate::tests::{api_token, document_index_address};

    pub struct StubClient;

    impl SearchClient for StubClient {
        async fn search(
            &self,
            _index: IndexPath,
            _request: SearchRequest,
            _api_token: &str,
        ) -> anyhow::Result<Vec<SearchResult>> {
            Ok(vec![])
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

    #[tokio::test]
    async fn search_request() {
        // Given a search client pointed at the document index
        let host = document_index_address().to_owned();
        let api_token = api_token();
        let client = Client::new(host).unwrap();

        // When making a query on an existing collection
        let index = IndexPath::new("f13", "wikipedia-de", "luminous-base-asymmetric-64");
        let request = SearchRequest::new(
            vec![Modality::Text {
                text: "What is the population of Heidelberg?".to_owned(),
            }],
            1,
            None,
            true,
        );
        let results = client.search(index, request, api_token).await.unwrap();

        // Then we get at least one result
        assert_eq!(results.len(), 1);
        assert!(results[0].document_path.name.contains("Heidelberg"));
        let Modality::Text { text } = &results[0].section[0] else {
            panic!("invalid entry");
        };
        assert!(text.contains("Heidelberg"));
    }

    #[tokio::test]
    async fn multiple_results() {
        // Given a search client pointed at the document index
        let host = document_index_address().to_owned();
        let api_token = api_token();
        let client = Client::new(host).unwrap();
        let max_results = 5;

        // When making a query on an existing collection
        let index = IndexPath::new("f13", "wikipedia-de", "luminous-base-asymmetric-64");
        let request = SearchRequest::new(
            vec![Modality::Text {
                text: "What is the population of Heidelberg?".to_owned(),
            }],
            max_results,
            None,
            true,
        );
        let results = client.search(index, request, api_token).await.unwrap();

        // Then we get at least one result
        assert_eq!(results.len(), max_results);
        assert!(results
            .iter()
            .all(|r| r.document_path.name.contains("Heidelberg")));
    }

    #[tokio::test]
    async fn min_score() {
        // Given a search client pointed at the document index
        let host = document_index_address().to_owned();
        let api_token = api_token();
        let client = Client::new(host).unwrap();
        let max_results = 5;
        let min_score = 0.725;

        // When making a query on an existing collection
        let index = IndexPath::new("f13", "wikipedia-de", "luminous-base-asymmetric-64");
        let request = SearchRequest::new(
            vec![Modality::Text {
                text: "What is the population of Heidelberg?".to_owned(),
            }],
            max_results,
            Some(min_score),
            true,
        );
        let results = client.search(index, request, api_token).await.unwrap();

        // Then we get less than 5 results
        assert_eq!(results.len(), 4);
        assert!(results.iter().all(|r| r.score >= min_score));
    }
}
