#[cfg(test)]
mod tests {
    use client::{
        IndexPath, Modality, SearchClient, SearchRequest as ClientSearchRequest,
        SearchResult as ClientSearchResult,
    };

    /// Search a Document Index collection
    pub struct SearchRequest {
        /// Where you want to search in
        index: IndexPath,
        /// What you want to search for
        query: String,
        /// The maximum number of results to return. Defaults to 1
        max_results: usize,
        /// The minimum score each result should have to be returned.
        /// By default, all results are returned, up to the `max_results`.
        min_score: Option<f64>,
    }

    impl SearchRequest {
        pub fn new(index: IndexPath, query: impl Into<String>) -> Self {
            Self {
                index,
                query: query.into(),
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

    /// A section of a document that is returned from a search request
    pub struct SearchResult {
        /// Which document this search result can be found in
        pub document_name: String,
        /// The section of the document returned by the search
        pub section: String,
    }

    impl TryFrom<ClientSearchResult> for SearchResult {
        type Error = anyhow::Error;

        fn try_from(result: ClientSearchResult) -> Result<Self, Self::Error> {
            let ClientSearchResult {
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
                Modality::Text { text } => SearchResult {
                    document_name: document_path.name,
                    section: text,
                },
                Modality::Image { .. } => {
                    unreachable!("We should have filtered out image results")
                }
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
            let SearchRequest {
                index,
                query,
                max_results,
                min_score,
            } = request;
            self.client
                .search(
                    index,
                    ClientSearchRequest::new(
                        vec![Modality::Text { text: query }],
                        max_results,
                        min_score,
                        // Make sure we only recieve results of type text
                        true,
                    ),
                    api_token,
                )
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
        use serde::{Deserialize, Serialize, Serializer};
        use serde_json::json;

        /// Search a Document Index collection
        #[derive(Debug, Serialize)]
        pub struct SearchRequest {
            /// What you want to search for
            query: Vec<Modality>,
            /// The maximum number of results to return. Defaults to 1
            max_results: usize,
            /// The minimum score each result should have to be returned.
            /// By default, all results are returned, up to the `max_results`.
            #[serde(skip_serializing_if = "Option::is_none")]
            min_score: Option<f64>,
            /// Whether only text chunks should be returned
            #[serde(serialize_with = "filters", rename = "filters")]
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

        #[expect(clippy::trivially_copy_pass_by_ref)]
        fn filters<S>(text_only: &bool, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let filters = if *text_only {
                json!([{ "with": [{ "modality": "text" }]}])
            } else {
                json!([])
            };

            filters.serialize(serializer)
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
        #[derive(Debug, Deserialize, Serialize)]
        #[serde(rename_all = "snake_case", tag = "modality")]
        pub enum Modality {
            Text { text: String },
            Image { bytes: String },
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
                index: IndexPath,
                request: SearchRequest,
                api_token: &str,
            ) -> anyhow::Result<Vec<SearchResult>> {
                let IndexPath {
                    namespace,
                    collection,
                    index,
                } = index;

                Ok(self
                    .http
                    .post(format!(
                        "{}/collections/{namespace}/{collection}/indexes/{index}/search",
                        &self.host
                    ))
                    .bearer_auth(api_token)
                    .json(&request)
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
    }

    use crate::tests::{api_token, document_index_address};

    #[tokio::test]
    async fn search_request() {
        // Given a search client pointed at the document index
        let host = document_index_address().to_owned();
        let api_token = api_token();
        let search = Search::new(SearchClient::new(host).unwrap());

        // When making a query on an existing collection
        let request = SearchRequest::new(
            IndexPath::new("f13", "wikipedia-de", "luminous-base-asymmetric-64"),
            "What is the population of Heidelberg?",
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
        let search = Search::new(SearchClient::new(host).unwrap());
        let max_results = 5;

        // When making a query on an existing collection
        let request = SearchRequest::new(
            IndexPath::new("f13", "wikipedia-de", "luminous-base-asymmetric-64"),
            "What is the population of Heidelberg?",
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

    #[tokio::test]
    async fn min_score() {
        // Given a search client pointed at the document index
        let host = document_index_address().to_owned();
        let api_token = api_token();
        let search = Search::new(SearchClient::new(host).unwrap());
        let max_results = 5;
        let min_score = 0.725;

        // When making a query on an existing collection
        let request = SearchRequest::new(
            IndexPath::new("f13", "wikipedia-de", "luminous-base-asymmetric-64"),
            "What is the population of Heidelberg?",
        )
        .with_max_results(max_results)
        .with_min_score(min_score);
        let results = search.search(request, api_token).await.unwrap();

        // Then we get less than 5 results
        assert_eq!(results.len(), 4);
    }
}
