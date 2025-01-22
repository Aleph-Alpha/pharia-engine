use std::{future::Future, pin::Pin, sync::Arc};

use async_trait::async_trait;
use futures::{stream::FuturesUnordered, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::value::Value;
use tokio::{
    select,
    sync::{mpsc, oneshot},
    task::JoinHandle,
};
use tracing::error;

use super::client::{
    Client, Cursor, DocumentPath, IndexPath, Modality, SearchClient,
    SearchRequest as ClientSearchRequest, SearchResult as ClientSearchResult,
};

/// Handle to the search actor. Spin this up in order to use the Search API
pub struct Search {
    send: mpsc::Sender<DocumentIndexMessage>,
    handle: JoinHandle<()>,
}

impl Search {
    /// Starts a new search Actor. Calls to this method be balanced by calls to
    /// [`Self::shutdown`].
    pub fn new(search_addr: String) -> Self {
        let client = Client::new(search_addr).unwrap();
        Self::with_client(client)
    }

    pub fn with_client(client: impl SearchClient) -> Self {
        let (send, recv) = mpsc::channel::<DocumentIndexMessage>(1);
        let mut actor = SearchActor::new(client, recv);
        let handle = tokio::spawn(async move { actor.run().await });
        Self { send, handle }
    }

    pub fn api(&self) -> mpsc::Sender<DocumentIndexMessage> {
        self.send.clone()
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
#[async_trait]
pub trait SearchApi: Clone + Send + Sync + 'static {
    async fn search(
        &self,
        request: SearchRequest,
        api_token: String,
    ) -> anyhow::Result<Vec<SearchResult>>;
    async fn document_metadata(
        &self,
        document_path: DocumentPath,
        api_token: String,
    ) -> anyhow::Result<Option<Value>>;
}

#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub document_path: DocumentPath,
    pub content: String,
    pub score: f64,
}

#[async_trait]
impl SearchApi for mpsc::Sender<DocumentIndexMessage> {
    async fn search(
        &self,
        request: SearchRequest,
        api_token: String,
    ) -> anyhow::Result<Vec<SearchResult>> {
        let (send, recv) = oneshot::channel();
        let msg = DocumentIndexMessage::SearchMessage {
            request,
            send,
            api_token,
        };
        self.send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await
            .expect("sender must be alive when awaiting for answers")
    }

    async fn document_metadata(
        &self,
        document_path: DocumentPath,
        api_token: String,
    ) -> anyhow::Result<Option<Value>> {
        let (send, recv) = oneshot::channel();
        let msg = DocumentIndexMessage::MetadataMessage {
            document_path,
            send,
            api_token,
        };
        self.send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await
            .expect("sender must be alive when awaiting for answers")
    }
}

/// Search a Document Index collection
#[derive(Debug, Serialize, Deserialize)]
pub struct SearchRequest {
    /// Where you want to search in
    pub index_path: IndexPath,
    /// What you want to search for
    pub query: String,
    /// The maximum number of results to return. Defaults to 1
    pub max_results: u32,
    /// The minimum score each result should have to be returned.
    /// By default, all results are returned, up to the `max_results`.
    pub min_score: Option<f64>,
}

/// A section of a document that is returned from a search request
#[derive(Debug)]
pub struct DocumentIndexSearchResult {
    /// Which document this search result can be found in
    pub document_path: DocumentPath,
    /// The section of the document returned by the search
    pub section: String,
    /// How close the result is to the query, calculated based on the distance
    /// metric of the index used in the search.
    pub score: f64,
    /// The position within the document where the section begins. The cursor is always inclusive.
    #[expect(dead_code, reason = "Unused so far")]
    pub start: Cursor,
    /// The position within the document where the section ends.
    /// The cursor is always inclusive, so the section includes the position represented by this cursor.
    #[expect(dead_code, reason = "Unused so far")]
    pub end: Cursor,
}

pub struct Document;

/// Allows for searching different collections in the Document Index
struct SearchActor<C: SearchClient> {
    /// Internal client to interact with the Document Index API
    client: Arc<C>,
    recv: mpsc::Receiver<DocumentIndexMessage>,
    running_requests: FuturesUnordered<Pin<Box<dyn Future<Output = ()> + Send>>>,
}

impl<C: SearchClient> SearchActor<C> {
    fn new(client: C, recv: mpsc::Receiver<DocumentIndexMessage>) -> Self {
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

    fn act(&mut self, msg: DocumentIndexMessage) {
        let client = self.client.clone();
        self.running_requests.push(Box::pin(async move {
            msg.act(client.as_ref()).await;
        }));
    }
}

#[derive(Debug)]
pub enum DocumentIndexMessage {
    SearchMessage {
        request: SearchRequest,
        send: oneshot::Sender<anyhow::Result<Vec<SearchResult>>>,
        api_token: String,
    },
    MetadataMessage {
        document_path: DocumentPath,
        send: oneshot::Sender<anyhow::Result<Option<Value>>>,
        api_token: String,
    },
}

impl DocumentIndexMessage {
    async fn act(self, client: &impl SearchClient) {
        match self {
            Self::SearchMessage {
                request,
                send,
                api_token,
            } => {
                let results = Self::search(client, request, &api_token).await;
                let results = results.map(|v| {
                    v.into_iter()
                        .map(
                            |DocumentIndexSearchResult {
                                 document_path,
                                 section,
                                 score,
                                 ..
                             }| SearchResult {
                                document_path,
                                content: section,
                                score,
                            },
                        )
                        .collect()
                });
                drop(send.send(results));
            }
            Self::MetadataMessage {
                document_path,
                send,
                api_token,
            } => {
                let results = Self::document_metadata(client, document_path, &api_token).await;
                drop(send.send(results));
            }
        }
    }

    async fn search(
        client: &impl SearchClient,
        request: SearchRequest,
        api_token: &str,
    ) -> anyhow::Result<Vec<DocumentIndexSearchResult>> {
        let SearchRequest {
            index_path: index,
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
                    // Make sure we only receive results of type text
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
                    score,
                    start,
                    end,
                } = result;
                // Current behavior is that chunking only ever happens within an item
                if section.len() > 1 {
                    return Err(anyhow::anyhow!(
                        "Document Index result has more than one item in a section."
                    ));
                }

                match section.remove(0) {
                    Modality::Text { text } => Ok(DocumentIndexSearchResult {
                        document_path,
                        section: text,
                        score,
                        start,
                        end,
                    }),
                    Modality::Image { .. } => {
                        error!("Unexpected image result in Document Index results");
                        Err(anyhow::anyhow!("Invalid search result"))
                    }
                }
            })
            .collect()
    }

    async fn document_metadata(
        client: &impl SearchClient,
        document_path: DocumentPath,
        api_token: &str,
    ) -> anyhow::Result<Option<Value>> {
        client.document_metadata(document_path, api_token).await
    }
}

#[cfg(test)]
pub mod tests {
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    use serde_json::Value;
    use tokio::{time::sleep, try_join};

    use crate::tests::{api_token, document_index_url};

    use super::*;

    /// Always return the same completion
    pub struct SearchStub {
        send: mpsc::Sender<DocumentIndexMessage>,
        join_handle: JoinHandle<()>,
    }

    impl SearchStub {
        pub fn new(
            result: impl Fn(DocumentPath) -> anyhow::Result<Option<Value>> + Send + 'static,
        ) -> Self {
            let (send, mut recv) = mpsc::channel::<DocumentIndexMessage>(1);
            let join_handle = tokio::spawn(async move {
                while let Some(msg) = recv.recv().await {
                    match msg {
                        DocumentIndexMessage::MetadataMessage {
                            document_path,
                            send,
                            ..
                        } => {
                            send.send(result(document_path)).unwrap();
                        }
                        DocumentIndexMessage::SearchMessage { .. } => {
                            unimplemented!()
                        }
                    }
                }
            });

            Self { send, join_handle }
        }

        pub async fn wait_for_shutdown(self) {
            drop(self.send);
            self.join_handle.await.unwrap();
        }

        pub fn api(&self) -> mpsc::Sender<DocumentIndexMessage> {
            self.send.clone()
        }
    }

    impl SearchRequest {
        pub fn new(index: IndexPath, query: impl Into<String>) -> Self {
            Self {
                index_path: index,
                query: query.into(),
                max_results: 1,
                min_score: None,
            }
        }

        pub fn with_max_results(mut self, max_results: u32) -> Self {
            self.max_results = max_results;
            self
        }

        pub fn with_min_score(mut self, min_score: f64) -> Self {
            self.min_score = Some(min_score);
            self
        }
    }

    #[tokio::test]
    async fn search_request() {
        // Given a search client pointed at the document index
        let host = document_index_url().to_owned();
        let api_token = api_token().to_owned();
        let search = Search::new(host);

        // When making a query on an existing collection
        let index = IndexPath::new("Kernel", "test", "asym-64");
        let request = SearchRequest::new(index, "What is the Pharia Kernel?");
        let results = search.api().search(request, api_token).await.unwrap();
        search.wait_for_shutdown().await;

        // Then we get at least one result
        assert_eq!(results.len(), 1);
        assert!(results[0]
            .document_path
            .name
            .to_lowercase()
            .contains("kernel"));
        assert!(results[0].content.contains("Kernel"));
    }

    #[tokio::test]
    async fn request_metadata() {
        // Given a search client pointed at the document index
        let host = document_index_url().to_owned();
        let api_token = api_token().to_owned();
        let search = Search::new(host);

        // When requesting metadata of an existing document
        let document_path = DocumentPath::new("Kernel", "test", "kernel-docs");
        let result = search
            .api()
            .document_metadata(document_path, api_token)
            .await
            .unwrap();
        search.wait_for_shutdown().await;

        // Then we get the expected metadata
        if let Some(metadata) = result {
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
        let api_token = api_token().to_owned();
        let search = Search::new(host);
        let max_results = 5;
        let min_score = 0.99;

        // When making a query on an existing collection
        let index = IndexPath::new("Kernel", "test", "asym-64");
        let request = SearchRequest::new(index, "What is the Pharia Kernel?")
            .with_max_results(max_results)
            .with_min_score(min_score);
        let results = search.api().search(request, api_token).await.unwrap();
        search.wait_for_shutdown().await;

        // Then we don't get any results
        assert!(results.is_empty());
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

        async fn document_metadata(
            &self,
            _document_path: DocumentPath,
            _api_token: &str,
        ) -> anyhow::Result<Option<Value>> {
            unimplemented!()
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
