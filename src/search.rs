use actor::{SearchActor, SearchMessage, SearchRequest, SearchResult};
use client::{Client, SearchClient};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

mod actor;
mod client;

/// Handle to the search actor. Spin this up in order to use the Search API
pub struct Search {
    send: mpsc::Sender<SearchMessage>,
    handle: JoinHandle<()>,
}

impl Search {
    /// Starts a new search Actor. Calls to this method be balanced by calls to
    /// [`Self::shutdown`].
    pub fn new(search_addr: String) -> Self {
        let client = Client::new(search_addr).unwrap();
        Self::with_client(client)
    }

    pub fn with_client(client: impl SearchClient + Send + Sync + 'static) -> Self {
        let (send, recv) = mpsc::channel::<SearchMessage>(1);
        let mut actor = SearchActor::new(client, recv);
        let handle = tokio::spawn(async move { actor.run().await });
        Self { send, handle }
    }

    pub fn api(&self) -> SearchApi {
        SearchApi::new(self.send.clone())
    }

    /// Inference is going to shutdown, as soon as the last instance of [`InferenceApi`] is dropped.
    pub async fn wait_for_shutdown(self) {
        drop(self.send);
        self.handle.await.unwrap();
    }
}

/// Use this to execute tasks with the Search API. The existence of this API handle implies the
/// actor is alive and running. This means this handle must be disposed of, before the search
/// actor can shut down.
#[derive(Clone)]
pub struct SearchApi {
    send: mpsc::Sender<SearchMessage>,
}

impl SearchApi {
    pub fn new(send: mpsc::Sender<SearchMessage>) -> Self {
        Self { send }
    }

    pub async fn search(
        &self,
        request: SearchRequest,
        api_token: String,
    ) -> anyhow::Result<Vec<SearchResult>> {
        let (send, recv) = oneshot::channel();
        let msg = SearchMessage {
            request,
            send,
            api_token,
        };
        self.send
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await
            .expect("sender must be alive when awaiting for answers")
    }
}

#[cfg(test)]
mod tests {
    use client::IndexPath;

    use crate::tests::{api_token, document_index_address};

    use super::*;

    #[tokio::test]
    async fn search_request() {
        // Given a search client pointed at the document index
        let host = document_index_address().to_owned();
        let api_token = api_token().to_owned();
        let search = Search::new(host);

        // When making a query on an existing collection
        let index = IndexPath::new("f13", "wikipedia-de", "luminous-base-asymmetric-64");
        let request = SearchRequest::new(index, "What is the population of Heidelberg?");
        let results = search.api().search(request, api_token).await.unwrap();
        search.wait_for_shutdown().await;

        // Then we get at least one result
        assert_eq!(results.len(), 1);
        assert!(results[0].document_name.contains("Heidelberg"));
        assert!(results[0].section.contains("Heidelberg"));
    }

    #[tokio::test]
    async fn multiple_results() {
        // Given a search client pointed at the document index
        let host = document_index_address().to_owned();
        let api_token = api_token().to_owned();
        let search = Search::new(host);
        let max_results = 5;

        // When making a query on an existing collection
        let index = IndexPath::new("f13", "wikipedia-de", "luminous-base-asymmetric-64");
        let request = SearchRequest::new(index, "What is the population of Heidelberg?")
            .with_max_results(max_results);
        let results = search.api().search(request, api_token).await.unwrap();
        search.wait_for_shutdown().await;

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
        let api_token = api_token().to_owned();
        let search = Search::new(host);
        let max_results = 5;
        let min_score = 0.725;

        // When making a query on an existing collection
        let index = IndexPath::new("f13", "wikipedia-de", "luminous-base-asymmetric-64");
        let request = SearchRequest::new(index, "What is the population of Heidelberg?")
            .with_max_results(max_results)
            .with_min_score(min_score);
        let results = search.api().search(request, api_token).await.unwrap();
        search.wait_for_shutdown().await;

        // Then we get less than 5 results
        assert_eq!(results.len(), 4);
    }
}
