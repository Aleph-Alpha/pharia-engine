#[cfg(test)]
mod tests {
    use itertools::Itertools;
    use reqwest::ClientBuilder;
    use serde_json::json;

    use crate::tests::{api_token, document_index_address};

    /// Search a Document Index collection
    struct SearchRequest {
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
        fn new(query: impl Into<String>, index: IndexPath) -> Self {
            Self {
                query: query.into(),
                index,
                max_results: 1,
                min_score: None,
            }
        }

        fn with_max_results(mut self, max_results: usize) -> Self {
            self.max_results = max_results;
            self
        }

        fn with_min_score(mut self, min_score: f64) -> Self {
            self.min_score = Some(min_score);
            self
        }
    }

    /// Which documents you want to search in, and which type of index should be used
    struct IndexPath {
        /// The namespace the collection belongs to
        namespace: String,
        /// The collection you want to search in
        collection: String,
        /// The search index you want to use for the collection
        index: String,
    }

    impl IndexPath {
        fn new(
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

    /// A section of a document that is returned from a search request
    struct SearchResult {
        /// Which document this search result can be found in
        document_name: String,
        /// The section of the document returned by the search
        section: String,
        /// The semantic similarity of the text to the query
        score: f64,
    }

    /// Sends HTTP Request to Document Index API
    struct SearchClient {
        /// The base host to use for all API requests
        host: String,
        /// Shared client to reuse connections
        http: reqwest::Client,
    }

    impl SearchClient {
        fn new(host: String) -> anyhow::Result<Self> {
            Ok(Self {
                host,
                http: ClientBuilder::new().build()?,
            })
        }

        async fn search(
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

            let mut payload = json!({ "query": [{ "modality": "text", "text": query }], "max_results": max_results });
            if let Some(min_score) = min_score {
                payload["min_score"] = json!(min_score);
            }

            let results = self
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
                .json::<Vec<parsing::RawSearchResult>>()
                .await?;

            let results = results
                .into_iter()
                .map(<Option<SearchResult>>::try_from)
                .filter_map_ok(|r| r)
                .collect::<Result<_, _>>()?;

            Ok(results)
        }
    }

    /// The following structs are just for parsing the results of the API before being transformed
    /// into the public API.
    mod parsing {
        use anyhow::anyhow;
        use serde::Deserialize;

        use super::SearchResult;

        /// Modality of the search result in the API
        #[derive(Debug, Deserialize)]
        #[serde(rename_all = "snake_case", tag = "modality")]
        enum Modality {
            Text {
                text: String,
            },
            /// Also returns a `bytes` field that we are ignoring for now
            Image,
        }

        /// The name of a given document
        #[derive(Debug, Deserialize)]
        struct DocumentPath {
            name: String,
        }

        /// A result for a given search that comes back from the API
        #[derive(Debug, Deserialize)]
        pub struct RawSearchResult {
            document_path: DocumentPath,
            section: Vec<Modality>,
            score: f64,
        }

        impl TryFrom<RawSearchResult> for Option<SearchResult> {
            type Error = anyhow::Error;

            fn try_from(result: RawSearchResult) -> Result<Self, Self::Error> {
                let RawSearchResult {
                    mut section,
                    document_path,
                    score,
                } = result;
                // Current behavior is that chunking only ever happens within an item
                if section.len() > 1 {
                    return Err(anyhow!(
                        "Document Index result has more than one item in a section."
                    ));
                }

                Ok(match section.remove(0) {
                    Modality::Text { text } => Some(SearchResult {
                        document_name: document_path.name,
                        section: text,
                        score,
                    }),
                    Modality::Image => None,
                })
            }
        }
    }

    #[tokio::test]
    async fn search_request() {
        // Given a search client pointed at the document index
        let host = document_index_address().to_owned();
        let api_token = api_token();
        let client = SearchClient::new(host).unwrap();

        // When making a query on an existing collection
        let request = SearchRequest::new(
            "What is the population of Heidelberg?",
            IndexPath::new("f13", "wikipedia-de", "luminous-base-asymmetric-64"),
        );
        let results = client.search(request, api_token).await.unwrap();

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
        let max_results = 5;

        // When making a query on an existing collection
        let request = SearchRequest::new(
            "What is the population of Heidelberg?",
            IndexPath::new("f13", "wikipedia-de", "luminous-base-asymmetric-64"),
        )
        .with_max_results(max_results);
        let results = client.search(request, api_token).await.unwrap();

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
