#[cfg(test)]
mod tests {
    use client::{IndexPath, SearchClient, SearchRequest};

    /// A section of a document that is returned from a search request
    struct SearchResult {
        /// Which document this search result can be found in
        document_name: String,
        /// The section of the document returned by the search
        section: String,
    }

    impl TryFrom<client::SearchResult> for SearchResult {
        type Error = anyhow::Error;

        fn try_from(result: client::SearchResult) -> Result<Self, Self::Error> {
            let client::SearchResult {
                mut section,
                document_path,
                score: _,
            } = result;
            // Current behavior is that chunking only ever happens within an item
            if section.len() > 1 {
                return Err(anyhow::anyhow!(
                    "Document Index result has more than one item in a section."
                ));
            }

            Ok(match section.remove(0) {
                client::Modality::Text { text } => SearchResult {
                    document_name: document_path.name,
                    section: text,
                },
            })
        }
    }

    /// Allows for searching different collections in the Document Index
    struct Search {
        /// Internal client to interact with the Document Index API
        client: SearchClient,
    }

    impl Search {
        fn new(client: SearchClient) -> Self {
            Self { client }
        }

        async fn search(
            &self,
            request: SearchRequest,
            api_token: &str,
        ) -> anyhow::Result<Vec<SearchResult>> {
            self.client
                .search(request, api_token)
                .await?
                .into_iter()
                .map(SearchResult::try_from)
                .collect()
        }
    }

    /// The following structs are just for parsing the results of the API before being transformed
    /// into the public API.
    mod client {
        use reqwest::ClientBuilder;
        use serde::Deserialize;
        use serde_json::json;

        /// Search a Document Index collection
        pub struct SearchRequest {
            /// What you want to search for
            query: String,
            /// Where you want to search in
            index: IndexPath,
            /// The maximum number of results to return. Defaults to 1
            max_results: usize,
            /// The minimum score each result should have to be returned.
            /// By default, all results are returned, up to the `max_results`.
            min_score: Option<f64>,
        }

        impl SearchRequest {
            pub fn new(query: impl Into<String>, index: IndexPath) -> Self {
                Self {
                    query: query.into(),
                    index,
                    max_results: 1,
                    min_score: None,
                }
            }

            pub fn with_max_results(mut self, max_results: usize) -> Self {
                self.max_results = max_results;
                self
            }

            pub fn with_min_score(mut self, min_score: f64) -> Self {
                self.min_score = Some(min_score);
                self
            }
        }

        /// Which documents you want to search in, and which type of index should be used
        pub struct IndexPath {
            /// The namespace the collection belongs to
            pub namespace: String,
            /// The collection you want to search in
            pub collection: String,
            /// The search index you want to use for the collection
            pub index: String,
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

        /// Modality of the search result in the API
        #[derive(Debug, Deserialize)]
        #[serde(rename_all = "snake_case", tag = "modality")]
        pub enum Modality {
            // Based on the filters, we will only ever get text for now.
            Text { text: String },
        }

        /// The name of a given document
        #[derive(Debug, Deserialize)]
        pub struct DocumentPath {
            pub name: String,
        }

        /// A result for a given search that comes back from the API
        #[derive(Debug, Deserialize)]
        pub struct SearchResult {
            pub document_path: DocumentPath,
            pub section: Vec<Modality>,
            pub score: f64,
        }

        /// Sends HTTP Request to Document Index API
        pub struct SearchClient {
            /// The base host to use for all API requests
            host: String,
            /// Shared client to reuse connections
            http: reqwest::Client,
        }

        impl SearchClient {
            pub fn new(host: String) -> anyhow::Result<Self> {
                Ok(Self {
                    host,
                    http: ClientBuilder::new().build()?,
                })
            }

            pub async fn search(
                &self,
                request: SearchRequest,
                api_token: &str,
            ) -> anyhow::Result<Vec<SearchResult>> {
                let SearchRequest {
                    query,
                    index:
                        IndexPath {
                            namespace,
                            collection,
                            index,
                        },
                    max_results,
                    min_score,
                } = request;

                let mut payload = json!({
                    "query": [{ "modality": "text", "text": query }],
                    "max_results": max_results,
                    // Make sure we only get text results
                    "filters": [{ "with": [{ "modality": "text" }]}]
                });
                if let Some(min_score) = min_score {
                    payload["min_score"] = json!(min_score);
                }

                Ok(self
                    .http
                    .post(format!(
                        "{}/collections/{namespace}/{collection}/indexes/{index}/search",
                        &self.host
                    ))
                    .bearer_auth(api_token)
                    .json(&payload)
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?)
            }
        }

        #[cfg(test)]
        mod tests {
            use super::*;

            use crate::tests::{api_token, document_index_address};

            #[tokio::test]
            async fn min_score() {
                // Given a search client pointed at the document index
                let host = document_index_address().to_owned();
                let api_token = api_token();
                let client = SearchClient::new(host).unwrap();
                let max_results = 5;
                let min_score = 0.725;

                // When making a query on an existing collection
                let request = SearchRequest::new(
                    "What is the population of Heidelberg?",
                    IndexPath::new("f13", "wikipedia-de", "luminous-base-asymmetric-64"),
                )
                .with_max_results(max_results)
                .with_min_score(min_score);
                let results = client.search(request, api_token).await.unwrap();

                // Then we get at least one result
                assert_eq!(results.len(), 4);
                assert!(results.iter().all(|r| r.score >= min_score));
            }
        }
    }

    use crate::tests::{api_token, document_index_address};

    #[tokio::test]
    async fn search_request() {
        // Given a search client pointed at the document index
        let host = document_index_address().to_owned();
        let api_token = api_token();
        let client = SearchClient::new(host).unwrap();
        let search = Search::new(client);

        // When making a query on an existing collection
        let request = SearchRequest::new(
            "What is the population of Heidelberg?",
            IndexPath::new("f13", "wikipedia-de", "luminous-base-asymmetric-64"),
        );
        let results = search.search(request, api_token).await.unwrap();

        // Then we get at least one result
        assert_eq!(results.len(), 1);
        assert!(results[0].document_name.contains("Heidelberg"));
        assert!(results[0].section.contains("Heidelberg"));
    }

    #[tokio::test]
    async fn multiple_results() {
        // Given a search client pointed at the document index
        let host = document_index_address().to_owned();
        let api_token = api_token();
        let client = SearchClient::new(host).unwrap();
        let search = Search::new(client);
        let max_results = 5;

        // When making a query on an existing collection
        let request = SearchRequest::new(
            "What is the population of Heidelberg?",
            IndexPath::new("f13", "wikipedia-de", "luminous-base-asymmetric-64"),
        )
        .with_max_results(max_results);
        let results = search.search(request, api_token).await.unwrap();

        // Then we get at least one result
        assert_eq!(results.len(), max_results);
        assert!(results
            .iter()
            .all(|r| r.document_name.contains("Heidelberg")));
        assert!(results.iter().all(|r| r.section.contains("Heidelberg")));
    }
}
