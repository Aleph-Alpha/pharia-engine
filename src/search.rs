mod actor;
mod client;

pub use actor::{Search, SearchApi, SearchRequest, SearchResult, TextCursor};
pub use client::{
    Document, DocumentPath, Filter, FilterCondition, IndexPath, MetadataFieldValue, MetadataFilter,
    MetadataFilterCondition, Modality,
};

#[cfg(test)]
pub mod tests {
    pub use super::actor::tests::SearchStub;
}
