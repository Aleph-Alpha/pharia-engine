use client::{
    IndexPath, Modality, SearchClient, SearchRequest as ClientSearchRequest,
    SearchResult as ClientSearchResult,
};

mod client;

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

/// Allows for searching different collections in the Document Index
struct SearchActor<C: SearchClient> {
    /// Internal client to interact with the Document Index API
    client: C,
}

impl<C: SearchClient> SearchActor<C> {
    fn new(client: C) -> Self {
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
            .map(|result| {
                let ClientSearchResult {
                    mut section,
                    document_path,
                    score: _,
                    start: _start,
                    end: _end,
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
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use client::Client;

    use crate::tests::{api_token, document_index_address};

    use super::*;

    #[tokio::test]
    async fn search_request() {
        // Given a search client pointed at the document index
        let host = document_index_address().to_owned();
        let api_token = api_token();
        let search = SearchActor::new(Client::new(host).unwrap());

        // When making a query on an existing collection
        let index = IndexPath::new("f13", "wikipedia-de", "luminous-base-asymmetric-64");
        let request = SearchRequest::new(index, "What is the population of Heidelberg?");
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
        let search = SearchActor::new(Client::new(host).unwrap());
        let max_results = 5;

        // When making a query on an existing collection
        let index = IndexPath::new("f13", "wikipedia-de", "luminous-base-asymmetric-64");
        let request = SearchRequest::new(index, "What is the population of Heidelberg?")
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
        let search = SearchActor::new(Client::new(host).unwrap());
        let max_results = 5;
        let min_score = 0.725;

        // When making a query on an existing collection
        let index = IndexPath::new("f13", "wikipedia-de", "luminous-base-asymmetric-64");
        let request = SearchRequest::new(index, "What is the population of Heidelberg?")
            .with_max_results(max_results)
            .with_min_score(min_score);
        let results = search.search(request, api_token).await.unwrap();

        // Then we get less than 5 results
        assert_eq!(results.len(), 4);
    }
}
