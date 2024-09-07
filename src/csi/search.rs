#[cfg(test)]
mod tests {
    use std::env;

    use anyhow::anyhow;
    use itertools::Itertools;
    use reqwest::ClientBuilder;
    use serde::Deserialize;
    use serde_json::json;

    /// Search a Document Index collection
    struct SearchRequest {
        /// What you want to search for
        query: String,
        /// Where you want to search in
        index: IndexPath,
        /// The maximum number of results to return. Defaults to 1
        max_results: usize,
    }

    impl SearchRequest {
        fn new(query: impl Into<String>, index: IndexPath) -> Self {
            Self {
                query: query.into(),
                index,
                max_results: 1,
            }
        }

        fn with_max_results(mut self, max_results: usize) -> Self {
            self.max_results = max_results;
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
        document_name: String,
        section: String,
    }

    impl TryFrom<RawSearchResult> for Option<SearchResult> {
        type Error = anyhow::Error;

        fn try_from(result: RawSearchResult) -> Result<Self, Self::Error> {
            let RawSearchResult {
                mut section,
                document_path,
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
                }),
                Modality::Image => None,
            })
        }
    }

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
    struct RawSearchResult {
        document_path: DocumentPath,
        section: Vec<Modality>,
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
            api_token: String,
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
            } = request;

            let results = self
                .http
                .post(format!(
                    "{}/collections/{namespace}/{collection}/indexes/{index}/search",
                    &self.host
                ))
                .bearer_auth(&api_token)
                .json(&json!({ "query": [{ "modality": "text", "text": query }], "max_results": max_results }))
                .send()
                .await?
                .error_for_status()?
                .json::<Vec<RawSearchResult>>()
                .await?;

            let results = results
                .into_iter()
                .map(<Option<SearchResult>>::try_from)
                .filter_map_ok(|r| r)
                .collect::<Result<_, _>>()?;

            Ok(results)
        }
    }

    #[tokio::test]
    async fn search_request() {
        // Given a search client pointed at the document index
        drop(dotenvy::dotenv());
        let host = env::var("DOCUMENT_INDEX_ADDRESS").unwrap();
        let api_token = env::var("AA_API_TOKEN").unwrap();
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
        drop(dotenvy::dotenv());
        let host = env::var("DOCUMENT_INDEX_ADDRESS").unwrap();
        let api_token = env::var("AA_API_TOKEN").unwrap();
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
}
