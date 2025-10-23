use std::{future::Future, pin::Pin, sync::Arc};

use futures::{StreamExt, stream::FuturesUnordered};
use serde_json::value::Value;
use tokio::{
    select,
    sync::{mpsc, oneshot},
    task::JoinHandle,
};
use tracing::{error, info};

use crate::{
    authorization::Authentication, logging::TracingContext, search::client::SearchNotConfigured,
};

use super::client::{
    Client, Cursor, Document, DocumentPath, Filter, IndexPath, Modality, SearchClient,
    SearchRequest as ClientSearchRequest, SearchResult as ClientSearchResult,
};

/// Handle to the search actor. Spin this up in order to use the Search API
pub struct Search {
    send: mpsc::Sender<SearchMsg>,
    handle: JoinHandle<()>,
}

impl Search {
    /// Starts a new search Actor. Calls to this method be balanced by calls to
    /// [`Self::shutdown`].
    ///
    /// If `search_addr` is not provided, the search actor will boot up with client that always
    /// returns an error. This can be useful if the Document Index is not available, either
    /// because the Kernel runs outside of `PhariaAI`, or because of resource constraints.
    pub fn new(search_addr: Option<&str>) -> Self {
        if let Some(search_addr) = search_addr {
            info!(target: "pharia-kernel::search", "Using Document Index at {}", search_addr);
            let client = Client::new(search_addr);
            Self::with_client(client)
        } else {
            info!(
                target: "pharia-kernel::search",
                "Document Index is not configured, running without search capabilities."
            );
            Self::with_client(SearchNotConfigured)
        }
    }

    pub fn with_client(client: impl SearchClient) -> Self {
        let (send, recv) = mpsc::channel::<SearchMsg>(1);
        let mut actor = SearchActor::new(client, recv);
        let handle = tokio::spawn(async move { actor.run().await });
        Self { send, handle }
    }

    pub fn api(&self) -> SearchSender {
        SearchSender(self.send.clone())
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
#[cfg_attr(test, double_trait::dummies)]
pub trait SearchApi {
    fn search(
        &self,
        request: SearchRequest,
        auth: Authentication,
        tracing_context: TracingContext,
    ) -> impl Future<Output = anyhow::Result<Vec<SearchResult>>> + Send;
    fn document_metadata(
        &self,
        document_path: DocumentPath,
        auth: Authentication,
        tracing_context: TracingContext,
    ) -> impl Future<Output = anyhow::Result<Option<Value>>> + Send;
    fn document(
        &self,
        document_path: DocumentPath,
        auth: Authentication,
        tracing_context: TracingContext,
    ) -> impl Future<Output = anyhow::Result<Document>> + Send;
}

/// Opaque wrapper around a sender to the search actor, so we do not need to expose our message
/// type.
#[derive(Clone)]
pub struct SearchSender(mpsc::Sender<SearchMsg>);

impl SearchApi for SearchSender {
    async fn search(
        &self,
        request: SearchRequest,
        auth: Authentication,
        tracing_context: TracingContext,
    ) -> anyhow::Result<Vec<SearchResult>> {
        let (send, recv) = oneshot::channel();
        let msg = SearchMsg::Search {
            request,
            send,
            auth,
            tracing_context,
        };
        self.0
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await
            .expect("sender must be alive when awaiting for answers")
    }

    async fn document_metadata(
        &self,
        document_path: DocumentPath,
        auth: Authentication,
        tracing_context: TracingContext,
    ) -> anyhow::Result<Option<Value>> {
        let (send, recv) = oneshot::channel();
        let msg = SearchMsg::Metadata {
            document_path,
            send,
            auth,
            tracing_context,
        };
        self.0
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await
            .expect("sender must be alive when awaiting for answers")
    }

    async fn document(
        &self,
        document_path: DocumentPath,
        auth: Authentication,
        tracing_context: TracingContext,
    ) -> anyhow::Result<Document> {
        let (send, recv) = oneshot::channel();
        let msg = SearchMsg::Document {
            document_path,
            send,
            auth,
            tracing_context,
        };
        self.0
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
    pub index_path: IndexPath,
    /// What you want to search for
    pub query: String,
    /// The maximum number of results to return. Defaults to 1
    pub max_results: u32,
    /// The minimum score each result should have to be returned.
    /// By default, all results are returned, up to the `max_results`.
    pub min_score: Option<f64>,
    /// A filter to apply to the search
    pub filters: Vec<Filter>,
}

/// A section of a document that is returned from a search request
#[derive(Debug)]
pub struct SearchResult {
    /// Which document this search result can be found in
    pub document_path: DocumentPath,
    /// The section of the document returned by the search
    pub content: String,
    /// How close the result is to the query, calculated based on the distance
    /// metric of the index used in the search.
    pub score: f64,
    /// The position within the document where the section begins. The cursor is always inclusive.
    pub start: TextCursor,
    /// The position within the document where the section ends.
    /// The cursor is always inclusive, so the section includes the position represented by this
    /// cursor.
    pub end: TextCursor,
}

#[derive(Debug)]
pub struct TextCursor {
    /// Index of the item in the document
    pub item: u32,
    /// The character position the cursor can be found at within the string.
    pub position: u32,
}

/// Allows for searching different collections in the Document Index
struct SearchActor<C: SearchClient> {
    /// Internal client to interact with the Document Index API
    client: Arc<C>,
    recv: mpsc::Receiver<SearchMsg>,
    running_requests: FuturesUnordered<Pin<Box<dyn Future<Output = ()> + Send>>>,
}

impl<C: SearchClient> SearchActor<C> {
    fn new(client: C, recv: mpsc::Receiver<SearchMsg>) -> Self {
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

    fn act(&mut self, msg: SearchMsg) {
        let client = self.client.clone();
        self.running_requests.push(Box::pin(async move {
            msg.act(client.as_ref()).await;
        }));
    }
}

#[derive(Debug)]
enum SearchMsg {
    Search {
        request: SearchRequest,
        send: oneshot::Sender<anyhow::Result<Vec<SearchResult>>>,
        auth: Authentication,
        tracing_context: TracingContext,
    },
    Metadata {
        document_path: DocumentPath,
        send: oneshot::Sender<anyhow::Result<Option<Value>>>,
        auth: Authentication,
        tracing_context: TracingContext,
    },
    Document {
        document_path: DocumentPath,
        send: oneshot::Sender<anyhow::Result<Document>>,
        auth: Authentication,
        tracing_context: TracingContext,
    },
}

impl SearchMsg {
    async fn act(self, client: &impl SearchClient) {
        match self {
            Self::Search {
                request,
                send,
                auth,
                tracing_context,
            } => {
                let results = Self::search(client, request, auth, tracing_context).await;
                drop(send.send(results));
            }
            Self::Metadata {
                document_path,
                send,
                auth,
                tracing_context,
            } => {
                let results =
                    Self::document_metadata(client, document_path, auth, tracing_context).await;
                drop(send.send(results));
            }
            Self::Document {
                document_path,
                send,
                auth,
                tracing_context,
            } => {
                let results = Self::document(client, document_path, auth, tracing_context).await;
                drop(send.send(results));
            }
        }
    }

    async fn search(
        client: &impl SearchClient,
        request: SearchRequest,
        auth: Authentication,
        tracing_context: TracingContext,
    ) -> anyhow::Result<Vec<SearchResult>> {
        let SearchRequest {
            index_path: index,
            query,
            max_results,
            min_score,
            filters,
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
                    filters,
                ),
                auth,
                &tracing_context,
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
                    let err = anyhow::anyhow!(
                        "Document Index result has more than one item in a section."
                    );
                    error!(parent: tracing_context.span(), "{}", err);
                    return Err(err);
                }

                if let (
                    Modality::Text { text },
                    Cursor::Text {
                        item: start_item,
                        position: start_position,
                    },
                    Cursor::Text {
                        item: end_item,
                        position: end_position,
                    },
                ) = (section.remove(0), start, end)
                {
                    Ok(SearchResult {
                        document_path,
                        content: text,
                        score,
                        start: TextCursor {
                            item: start_item,
                            position: start_position,
                        },
                        end: TextCursor {
                            item: end_item,
                            position: end_position,
                        },
                    })
                } else {
                    let err = anyhow::anyhow!("Invalid search result");
                    error!(parent: tracing_context.span(), "{}", err);
                    Err(err)
                }
            })
            .collect()
    }

    async fn document_metadata(
        client: &impl SearchClient,
        document_path: DocumentPath,
        auth: Authentication,
        tracing_context: TracingContext,
    ) -> anyhow::Result<Option<Value>> {
        client
            .document_metadata(document_path, auth, &tracing_context)
            .await
    }

    async fn document(
        client: &impl SearchClient,
        document_path: DocumentPath,
        auth: Authentication,
        tracing_context: TracingContext,
    ) -> anyhow::Result<Document> {
        client.document(document_path, auth, &tracing_context).await
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

    use crate::search::client::SearchClient;

    use super::*;

    pub struct SearchStub {
        document: Box<dyn Fn(DocumentPath) -> anyhow::Result<Document> + Send + Sync + 'static>,
        document_metadata:
            Box<dyn Fn(DocumentPath) -> anyhow::Result<Option<Value>> + Send + Sync + 'static>,
    }

    impl SearchStub {
        pub fn new() -> Self {
            Self {
                document: Box::new(|_| Err(anyhow::anyhow!("Not implemented"))),
                document_metadata: Box::new(|_| Err(anyhow::anyhow!("Not implemented"))),
            }
        }

        pub fn with_document(
            mut self,
            document: impl Fn(DocumentPath) -> anyhow::Result<Document> + Send + Sync + 'static,
        ) -> Self {
            self.document = Box::new(document);
            self
        }

        pub fn with_document_metadata(
            mut self,
            document_metadata: impl Fn(DocumentPath) -> anyhow::Result<Option<Value>>
            + Send
            + Sync
            + 'static,
        ) -> Self {
            self.document_metadata = Box::new(document_metadata);
            self
        }
    }

    impl SearchApi for SearchStub {
        async fn document_metadata(
            &self,
            document_path: DocumentPath,
            _auth: Authentication,
            _tracing_context: TracingContext,
        ) -> anyhow::Result<Option<Value>> {
            (self.document_metadata)(document_path)
        }

        async fn document(
            &self,
            document_path: DocumentPath,
            _auth: Authentication,
            _tracing_context: TracingContext,
        ) -> anyhow::Result<Document> {
            (self.document)(document_path)
        }
    }

    impl SearchRequest {
        pub fn new(index: IndexPath, query: impl Into<String>) -> Self {
            Self {
                index_path: index,
                query: query.into(),
                max_results: 1,
                min_score: None,
                filters: Vec::new(),
            }
        }
    }

    #[tokio::test]
    async fn search_not_configured() {
        // Given a search client that is not configured
        let api_token = Authentication::none();
        let search = Search::new(None);

        // When making a query
        let index = IndexPath::new("Kernel", "test", "asym-64");
        let request = SearchRequest::new(index, "What is the Pharia Kernel?");
        let results = search
            .api()
            .search(request, api_token, TracingContext::dummy())
            .await;
        search.wait_for_shutdown().await;

        // Then we get an error
        assert!(results.is_err());
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
            _authentication: Authentication,
            _tracing_context: &TracingContext,
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
                Authentication::none(),
                TracingContext::dummy(),
            ),
            api.search(
                SearchRequest::new(index_path, "query"),
                Authentication::none(),
                TracingContext::dummy(),
            ),
        );

        // We need to drop the sender in order for `actor.run` to terminate
        drop(api);
        search.wait_for_shutdown().await;

        // Then: Both run concurrently and only return once both are completed.
        assert!(resp.is_ok());
    }
}
