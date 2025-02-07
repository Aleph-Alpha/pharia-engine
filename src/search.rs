mod actor;
mod client;

pub use actor::{DocumentIndexMessage, Search, SearchApi, SearchRequest, SearchResult, TextCursor};
pub use client::{Document, DocumentPath, IndexPath, Modality};

#[cfg(test)]
pub mod tests {
    pub use super::actor::tests::SearchStub;
}
