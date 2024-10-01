use std::{future::Future, pin::Pin, sync::Arc};

use futures::{stream::FuturesUnordered, StreamExt};
use tokio::{
    select,
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use super::client::{
    Client, IndexPath, Modality, SearchClient, SearchRequest as ClientSearchRequest,
    SearchResult as ClientSearchResult,
};

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

/// Search a Document Index collection
#[derive(Debug)]
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
#[derive(Debug)]
pub struct SearchResult {
    /// Which document this search result can be found in
    pub document_name: String,
    /// The section of the document returned by the search
    pub section: String,
}

/// Allows for searching different collections in the Document Index
struct SearchActor<C: SearchClient + Send + Sync + 'static> {
    /// Internal client to interact with the Document Index API
    client: Arc<C>,
    recv: mpsc::Receiver<SearchMessage>,
    running_requests: FuturesUnordered<Pin<Box<dyn Future<Output = ()> + Send>>>,
}

impl<C: SearchClient + Send + Sync + 'static> SearchActor<C> {
    fn new(client: C, recv: mpsc::Receiver<SearchMessage>) -> Self {
        Self {
            client: Arc::new(client),
            recv,
            running_requests: FuturesUnordered::new(),
        }
    }

    async fn run(&mut self) {
        loop {
            // While there are messages and searches, poll both.
            // If there is a message, add it to the queue.
            // If there are searches, make progress on them.
            select! {
                msg = self.recv.recv() => match msg {
                    Some(msg) => self.act(msg),
                    // Senders are gone, break out of the loop for shutdown.
                    None => break
                },
                // FuturesUnordered will let them run in parallel. It will
                // yield once one of them is completed.
                () = self.running_requests.select_next_some(), if !self.running_requests.is_empty()  => {}
            };
        }
    }

    fn act(&mut self, msg: SearchMessage) {
        let client = self.client.clone();
        self.running_requests.push(Box::pin(async move {
            msg.act(client.as_ref()).await;
        }));
    }
}

#[derive(Debug)]
pub struct SearchMessage {
    pub request: SearchRequest,
    pub send: oneshot::Sender<anyhow::Result<Vec<SearchResult>>>,
    pub api_token: String,
}

impl SearchMessage {
    async fn act(self, client: &impl SearchClient) {
        let SearchMessage {
            request,
            send,
            api_token,
        } = self;
        let result = Self::search(client, request, &api_token).await;
        drop(send.send(result));
    }

    async fn search(
        client: &impl SearchClient,
        request: SearchRequest,
        api_token: &str,
    ) -> anyhow::Result<Vec<SearchResult>> {
        let SearchRequest {
            index,
            query,
            max_results,
            min_score,
        } = request;
        client
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
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    use tokio::{time::sleep, try_join};

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

    /// This Client will only resolve a completion once the correct number of
    /// requests have been reached.
    pub struct AssertConcurrentClient {
        /// Number of requests we are still waiting on
        expected_concurrent_requests: AtomicUsize,
    }

    impl AssertConcurrentClient {
        pub fn new(expected_concurrent_requests: impl Into<AtomicUsize>) -> Self {
            Self {
                expected_concurrent_requests: expected_concurrent_requests.into(),
            }
        }
    }

    impl SearchClient for AssertConcurrentClient {
        async fn search(
            &self,
            _index: IndexPath,
            _request: ClientSearchRequest,
            _api_token: &str,
        ) -> anyhow::Result<Vec<ClientSearchResult>> {
            self.expected_concurrent_requests
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |e| {
                    Some(e.saturating_sub(1))
                })
                .unwrap();

            while self.expected_concurrent_requests.load(Ordering::SeqCst) > 0 {
                sleep(Duration::from_millis(1)).await;
            }
            Ok(vec![])
        }
    }

    /// We want to ensure that the actor invokes the client multiple times concurrently instead of
    /// only one inference request at a time.
    #[tokio::test(start_paused = true)]
    async fn concurrent_invocation_of_client() {
        // Given
        let client = AssertConcurrentClient::new(2);
        let search = Search::with_client(client);
        let api = search.api();

        // When
        // Schedule two tasks
        let index_path = IndexPath::new("namespace", "collection", "index");
        let resp = try_join!(
            api.search(
                SearchRequest::new(index_path.clone(), "query"),
                "0".to_owned()
            ),
            api.search(SearchRequest::new(index_path, "query"), "1".to_owned()),
        );

        // We need to drop the sender in order for `actor.run` to terminate
        drop(api);
        search.wait_for_shutdown().await;

        // Then: Both run concurrently and only return once both are completed.
        assert!(resp.is_ok());
    }
}
