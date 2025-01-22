mod actor;
mod client;

pub use actor::{DocumentIndexMessage, Search, SearchApi, SearchRequest, SearchResult};
pub use client::{Document, DocumentPath, IndexPath};

#[cfg(test)]
pub mod tests {
    pub use super::actor::tests::SearchStub;
}
