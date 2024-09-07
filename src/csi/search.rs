#[cfg(test)]
mod tests {
    use std::env;

    use reqwest::ClientBuilder;
    use serde::Deserialize;
    use serde_json::json;

    /// Search a Document Index collection
    struct SearchRequest {
        /// What you want to search for
        query: String,
        /// Where you want to search in
        index: IndexPath,
    }

    impl SearchRequest {
        fn new(query: impl Into<String>, index: IndexPath) -> Self {
            Self {
                query: query.into(),
                index,
            }
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

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "snake_case", tag = "modality")]
    enum Modality {
        Text { text: String },
        Image,
    }

    /// A result for a given search that comes back from the API
    #[derive(Debug, Deserialize)]
    struct RawSearchResult {
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
        ) -> anyhow::Result<Vec<String>> {
            let SearchRequest {
                query,
                index:
                    IndexPath {
                        namespace,
                        collection,
                        index,
                    },
            } = request;

            let results = self
                .http
                .post(format!(
                    "{}/collections/{namespace}/{collection}/indexes/{index}/search",
                    &self.host
                ))
                .bearer_auth(api_token)
                .json(&json!({ "query": [{ "modality": "text", "text": query }] }))
                .send()
                .await?
                .error_for_status()?
                .json::<Vec<RawSearchResult>>()
                .await?;

            let results = results
                .into_iter()
                .flat_map(|result| result.section)
                .filter_map(|section| match section {
                    Modality::Text { text } => Some(text),
                    Modality::Image => None,
                })
                .collect();

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
        assert!(results[0].contains("Heidelberg"));
    }
}
