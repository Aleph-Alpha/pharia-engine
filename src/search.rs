mod actor;
mod client;

pub use actor::{
    DocumentIndexMessage, DocumentMetadataRequest, Search, SearchApi, SearchRequest, SearchResult,
};
pub use client::{DocumentPath, IndexPath};

#[cfg(test)]
pub mod tests {
    pub use super::actor::tests::SearchStub;
}
