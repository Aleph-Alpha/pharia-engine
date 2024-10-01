use tokio::{
    select,
    sync::{mpsc, oneshot},
};

use super::client::{
    IndexPath, Modality, SearchClient, SearchRequest as ClientSearchRequest,
    SearchResult as ClientSearchResult,
};

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
pub struct SearchActor<C: SearchClient> {
    /// Internal client to interact with the Document Index API
    client: C,
    recv: mpsc::Receiver<SearchMessage>,
}

impl<C: SearchClient> SearchActor<C> {
    pub fn new(client: C, recv: mpsc::Receiver<SearchMessage>) -> Self {
        Self { client, recv }
    }

    pub async fn run(&mut self) {
        loop {
            // While there are messages and searches, poll both.
            // If there is a message, add it to the queue.
            // If there are searches, make progress on them.
            select! {
                msg = self.recv.recv() => match msg {
                    Some(msg) => self.act(msg).await,
                    // Senders are gone, break out of the loop for shutdown.
                    None => break
                },
            };
        }
    }

    async fn act(&self, msg: SearchMessage) {
        let SearchMessage {
            request,
            send,
            api_token,
        } = msg;
        let result = self.search(request, &api_token).await;
        drop(send.send(result));
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

#[derive(Debug)]
pub struct SearchMessage {
    pub request: SearchRequest,
    pub send: oneshot::Sender<anyhow::Result<Vec<SearchResult>>>,
    pub api_token: String,
}
