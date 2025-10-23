use std::future::Future;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{authorization::Authentication, http::HttpClient, logging::TracingContext};

#[cfg_attr(test, double_trait::dummies)]
pub trait SearchClient: Send + Sync + 'static {
    fn search(
        &self,
        index: IndexPath,
        request: SearchRequest,
        authentication: Authentication,
        tracing_context: &TracingContext,
    ) -> impl Future<Output = anyhow::Result<Vec<SearchResult>>> + Send;

    fn document_metadata(
        &self,
        document_path: DocumentPath,
        authentication: Authentication,
        tracing_context: &TracingContext,
    ) -> impl Future<Output = anyhow::Result<Option<Value>>> + Send;

    fn document(
        &self,
        document_path: DocumentPath,
        authentication: Authentication,
        tracing_context: &TracingContext,
    ) -> impl Future<Output = anyhow::Result<Document>> + Send;
}

/// A search client that always returns runtime errors.
///
/// While the `Document Index` powers all the vector search capabilities of the CSI, there is also
/// a broad subset of Skills that only make use of inference capabilities. In order to make the
/// Kernel useful without other proprietary systems like the Document Index, we provide a way to
/// operate the Kernel without connecting to the Document Index. If any Skill tries to access
/// functionality that requires the Document Index, the search implementation will return an error
/// and Skill execution will be suspended.
pub struct SearchNotConfigured;

impl SearchNotConfigured {
    const ERROR_MESSAGE: &str = "Document Index is not configured. The Kernel is running without \
    search capabilities. To enable search capabilities, please ask your operator to configure the \
    Document Index URL in the Kernel configuration.";
}

impl SearchClient for SearchNotConfigured {
    async fn search(
        &self,
        _index: IndexPath,
        _request: SearchRequest,
        _authentication: Authentication,
        _tracing_context: &TracingContext,
    ) -> anyhow::Result<Vec<SearchResult>> {
        Err(anyhow::anyhow!(Self::ERROR_MESSAGE))
    }

    async fn document_metadata(
        &self,
        _document_path: DocumentPath,
        _authentication: Authentication,
        _tracing_context: &TracingContext,
    ) -> anyhow::Result<Option<Value>> {
        Err(anyhow::anyhow!(Self::ERROR_MESSAGE))
    }

    async fn document(
        &self,
        _document_path: DocumentPath,
        _authentication: Authentication,
        _tracing_context: &TracingContext,
    ) -> anyhow::Result<Document> {
        Err(anyhow::anyhow!(Self::ERROR_MESSAGE))
    }
}

/// Search a Document Index collection
pub struct SearchRequest {
    /// What you want to search for
    query: Vec<Modality>,
    /// The maximum number of results to return. Defaults to 1
    max_results: u32,
    /// The minimum score each result should have to be returned.
    /// By default, all results are returned, up to the `max_results`.
    min_score: Option<f64>,
    /// Whether only text chunks should be returned
    text_only: bool,
    /// A filter to apply to the search
    filters: Vec<Filter>,
}

impl SearchRequest {
    pub fn new(
        query: Vec<Modality>,
        max_results: u32,
        min_score: Option<f64>,
        text_only: bool,
        filter: Vec<Filter>,
    ) -> Self {
        Self {
            query,
            max_results,
            min_score,
            text_only,
            filters: filter,
        }
    }
}

#[derive(Serialize, Debug)]
#[serde(untagged)]
pub enum MetadataFieldValue {
    String(String),
    Integer(i64),
    Boolean(bool),
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum MetadataFilterCondition {
    GreaterThan(f64),
    GreaterThanOrEqualTo(f64),
    LessThan(f64),
    LessThanOrEqualTo(f64),
    After(String),
    AtOrAfter(String),
    Before(String),
    AtOrBefore(String),
    EqualTo(MetadataFieldValue),
    IsNull(serde_bool::True),
}

#[derive(Serialize, Debug)]
pub struct MetadataFilter {
    pub field: String,
    #[serde(flatten)]
    pub condition: MetadataFilterCondition,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum FilterCondition {
    Modality(ModalityType),
    Metadata(MetadataFilter),
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Filter {
    Without(Vec<FilterCondition>),
    WithOneOf(Vec<FilterCondition>),
    With(Vec<FilterCondition>),
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ModalityType {
    Text,
}

/// Which documents you want to search in, and which type of index should be used
#[derive(Clone, Debug)]
pub struct IndexPath {
    /// The namespace the collection belongs to
    pub namespace: String,
    /// The collection you want to search in
    pub collection: String,
    /// The search index you want to use for the collection
    pub index: String,
}

/// Modality of the search result in the API
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "modality")]
pub enum Modality {
    Text { text: String },
    Image { bytes: String },
}

/// A position within a document. The cursor is always inclusive of the current position, in both
/// start and end positions.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", tag = "modality")]
pub enum Cursor {
    Text {
        /// Index of the item in the document
        item: u32,
        /// The character position the cursor can be found at within the string.
        position: u32,
    },
    /// Contains the index of the item in the document, but we do not use it
    Image,
}

/// The name of a given document
#[derive(Debug, Deserialize, Clone)]
pub struct DocumentPath {
    /// The namespace the collection belongs to
    pub namespace: String,
    /// The collection the document belongs to
    pub collection: String,
    /// The name of the document
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct Document {
    pub path: DocumentPath,
    pub contents: Vec<Modality>,
    pub metadata: Option<Value>,
}

/// A result for a given search that comes back from the API
#[derive(Debug, Deserialize)]
pub struct SearchResult {
    pub document_path: DocumentPath,
    pub section: Vec<Modality>,
    pub score: f64,
    pub start: Cursor,
    pub end: Cursor,
}

/// Sends HTTP Request to Document Index API
pub struct Client {
    /// The base host to use for all API requests
    host: String,
    /// Shared client to reuse connections
    http: HttpClient,
}

impl Client {
    pub fn new(host: impl Into<String>) -> Self {
        Self::with_http_client(host, HttpClient::default())
    }

    pub fn with_http_client(host: impl Into<String>, http: HttpClient) -> Self {
        Self {
            host: host.into(),
            http,
        }
    }
}

impl SearchClient for Client {
    async fn search(
        &self,
        index: IndexPath,
        request: SearchRequest,
        authentication: Authentication,
        tracing_context: &TracingContext,
    ) -> anyhow::Result<Vec<SearchResult>> {
        let api_token = authentication.into_maybe_string().ok_or_else(|| {
            anyhow::anyhow!(
                "Searching in the Document Index requires a PhariaAI token. \
                Please provide a valid token in the Authorization header."
            )
        })?;
        let IndexPath {
            namespace,
            collection,
            index,
        } = index;
        let SearchRequest {
            query,
            max_results,
            min_score,
            text_only,
            mut filters,
        } = request;

        let mut body = json!({
            "query": query,
            "max_results": max_results,
            "min_score": min_score,
        });

        if text_only {
            filters.push(Filter::With(vec![FilterCondition::Modality(
                ModalityType::Text,
            )]));
        }
        body["filters"] = json!(filters);

        // Namespaces, collections and indexes must match regex ^[a-zA-Z0-9\-\.]+$
        // therefore, we do not need to url encode them
        let url = format!(
            "{}/collections/{namespace}/{collection}/indexes/{index}/search",
            &self.host
        );

        Ok(self
            .http
            .post(url)
            .bearer_auth(api_token)
            .headers(tracing_context.w3c_headers())
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    async fn document_metadata(
        &self,
        document_path: DocumentPath,
        authentication: Authentication,
        tracing_context: &TracingContext,
    ) -> anyhow::Result<Option<Value>> {
        #[derive(Deserialize)]
        struct Document {
            metadata: Option<Value>,
        }

        let api_token = authentication.into_maybe_string().ok_or_else(|| {
            anyhow::anyhow!(
                "Fetching metadata from the Document Index requires a PhariaAI token. \
                Please provide a valid token in the Authorization header."
            )
        })?;

        let DocumentPath {
            namespace,
            collection,
            name,
        } = document_path;

        // Namespaces and collections must match regex ^[a-zA-Z0-9\-\.]+$
        // therefore, we do not need to url encode them
        // A document name can contain characters like `/`, which need to be url encoded
        let encoded_name = urlencoding::encode(&name);
        let url = format!(
            "{}/collections/{namespace}/{collection}/docs/{encoded_name}",
            &self.host
        );

        let document = self
            .http
            .get(url)
            .bearer_auth(api_token)
            .headers(tracing_context.w3c_headers())
            .send()
            .await?
            .error_for_status()?
            .json::<Document>()
            .await?;

        Ok(document.metadata)
    }

    async fn document(
        &self,
        document_path: DocumentPath,
        authentication: Authentication,
        tracing_context: &TracingContext,
    ) -> anyhow::Result<Document> {
        #[derive(Deserialize)]
        struct JsonDocument {
            contents: Vec<Modality>,
            metadata: Option<Value>,
        }

        let api_token = authentication.into_maybe_string().ok_or_else(|| {
            anyhow::anyhow!(
                "Fetching a document from the Document Index requires a PhariaAI token. \
                Please provide a valid token in the Authorization header."
            )
        })?;

        let DocumentPath {
            namespace,
            collection,
            name,
        } = &document_path;

        // Namespaces and collections must match regex ^[a-zA-Z0-9\-\.]+$
        // therefore, we do not need to url encode them
        // A document name can contain characters like `/`, which need to be url encoded
        let encoded_name = urlencoding::encode(name);
        let url = format!(
            "{}/collections/{namespace}/{collection}/docs/{encoded_name}",
            &self.host
        );

        let document = self
            .http
            .get(url)
            .bearer_auth(api_token)
            .headers(tracing_context.w3c_headers())
            .send()
            .await?
            .error_for_status()?
            .json::<JsonDocument>()
            .await?;

        Ok(Document {
            path: document_path,
            contents: document.contents,
            metadata: document.metadata,
        })
    }
}

#[cfg(test)]
pub mod tests {
    use std::path::{Path, PathBuf};

    use reqwest_vcr::VCRMode;

    use super::*;

    use crate::tests::{api_token, document_index_url};

    impl Document {
        pub fn dummy() -> Self {
            Self {
                path: DocumentPath::new("Kernel", "test", "kernel-docs"),
                contents: vec![Modality::Text {
                    text: "Hello Homer".to_owned(),
                }],
                metadata: Some(json!({ "url": "http://example.de" })),
            }
        }
    }

    impl IndexPath {
        pub fn new(
            namespace: impl Into<String>,
            collection: impl Into<String>,
            index: impl Into<String>,
        ) -> Self {
            Self {
                namespace: namespace.into(),
                collection: collection.into(),
                index: index.into(),
            }
        }
    }

    impl DocumentPath {
        pub fn new(
            namespace: impl Into<String>,
            collection: impl Into<String>,
            name: impl Into<String>,
        ) -> Self {
            Self {
                namespace: namespace.into(),
                collection: collection.into(),
                name: name.into(),
            }
        }
    }

    /// Create a search client with a cassette for the given cassette name and VCR mode
    ///
    /// To rerecord a cassette, delete the file and run the test again.
    fn client_with_cassette(cassette: &str) -> Client {
        let mut cassette_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let path = format!("tests/cassettes/{cassette}.vcr.json");
        cassette_path.push(path);
        let vcr_mode = if Path::new(&cassette_path).exists() {
            VCRMode::Replay
        } else {
            VCRMode::Record
        };

        let http = HttpClient::with_vcr(cassette_path, vcr_mode);
        let host = document_index_url();
        Client::with_http_client(host, http)
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "test_document_index"), ignore)]
    async fn document_exists() {
        // Given a search client pointed at the document index
        let cassette = "document_exists";
        let client = client_with_cassette(cassette);

        // When requesting a document
        let document_path = DocumentPath::new("Kernel", "test", "kernel-docs");
        let auth = Authentication::from_token(api_token());
        let document = client
            .document(document_path, auth, &TracingContext::dummy())
            .await
            .unwrap();

        // Then we get the expected document
        assert!(!document.contents.is_empty());
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "test_document_index"), ignore)]
    async fn document_not_found_is_err() {
        // Given a search client pointed at the document index
        let cassette = "document_not_found_is_err";
        let client = client_with_cassette(cassette);

        // When requesting a document that does not exist
        let document_path = DocumentPath::new("Kernel", "test", "kernel-docs-not-found");
        let auth = Authentication::from_token(api_token());
        let maybe_document = client
            .document(document_path, auth, &TracingContext::dummy())
            .await;

        // Then we get no document
        assert!(maybe_document.is_err());
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "test_document_index"), ignore)]
    async fn search_request() {
        // Given a search client pointed at the document index
        let cassette = "search_request";
        let client = client_with_cassette(cassette);

        // When making a query on an existing collection
        let index = IndexPath::new("Kernel", "test", "asym-64");
        let request = SearchRequest::new(
            vec![Modality::Text {
                text: "What is the Pharia Kernel?".to_owned(),
            }],
            1,
            None,
            true,
            Vec::new(),
        );
        let auth = Authentication::from_token(api_token());
        let results = client
            .search(index, request, auth, &TracingContext::dummy())
            .await
            .unwrap();

        // Then we get at least one result
        assert_eq!(results.len(), 1);
        assert!(
            results[0]
                .document_path
                .name
                .to_lowercase()
                .contains("kernel")
        );
        let Modality::Text { text } = &results[0].section[0] else {
            panic!("invalid entry");
        };
        assert!(text.contains("Kernel"));
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "test_document_index"), ignore)]
    async fn request_metadata() {
        // Given a search client pointed at the document index
        let cassette = "request_metadata";
        let client = client_with_cassette(cassette);

        // When requesting metadata of an existing document
        let document_path = DocumentPath::new("Kernel", "test", "kernel/docs");
        let auth = Authentication::from_token(api_token());
        let metadata = client
            .document_metadata(document_path, auth, &TracingContext::dummy())
            .await
            .unwrap()
            .unwrap();

        // Then we get the expected metadata
        assert_eq!(
            metadata["url"].as_str().unwrap(),
            "https://pharia-kernel.product.pharia.com/"
        );
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "test_document_index"), ignore)]
    async fn min_score() {
        // Given a search client pointed at the document index
        let cassette = "min_score";
        let client = client_with_cassette(cassette);

        let max_results = 5;
        let min_score = 0.99;

        // When making a query on an existing collection with a high min score
        let index = IndexPath::new("Kernel", "test", "asym-64");
        let request = SearchRequest::new(
            vec![Modality::Text {
                text: "tomato?".to_owned(),
            }],
            max_results,
            Some(min_score),
            true,
            Vec::new(),
        );
        let auth = Authentication::from_token(api_token());
        let results = client
            .search(index, request, auth, &TracingContext::dummy())
            .await
            .unwrap();

        // Then we don't get any results
        assert!(results.is_empty());
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "test_document_index"), ignore)]
    async fn filter_for_metadata() {
        // Given a search request with a metadata filter for a created field
        let cassette = "filter_for_metadata";
        let client = client_with_cassette(cassette);

        // When filtering for documents with metadata field created after 2100-01-01
        let index = IndexPath::new("Kernel", "test", "asym-64");
        let filter_condition = FilterCondition::Metadata(MetadataFilter {
            field: "created".to_owned(),
            condition: MetadataFilterCondition::After("2100-01-01T14:10:11Z".to_owned()),
        });
        let filter = Filter::With(vec![filter_condition]);
        let request = SearchRequest::new(
            vec![Modality::Text {
                text: "What is the Pharia Kernel?".to_owned(),
            }],
            1,
            None,
            true,
            vec![filter],
        );
        let auth = Authentication::from_token(api_token());
        let results = client
            .search(index, request, auth, &TracingContext::dummy())
            .await
            .unwrap();

        // Then we don't get any results
        assert!(results.is_empty());
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "test_document_index"), ignore)]
    async fn filter_for_without_metadata() {
        // Given a search request with a metadata filter for a created field
        let cassette = "filter_for_without_metadata";
        let client = client_with_cassette(cassette);

        let index = IndexPath::new("Kernel", "test", "asym-64");

        // When filtering for documents with metadata field created after 1970-07-01
        let filter_condition = FilterCondition::Metadata(MetadataFilter {
            field: "created".to_owned(),
            condition: MetadataFilterCondition::After("1970-07-01T14:10:11Z".to_owned()),
        });
        let filter = Filter::Without(vec![filter_condition]);
        let request = SearchRequest::new(
            vec![Modality::Text {
                text: "What is the Pharia Kernel?".to_owned(),
            }],
            1,
            None,
            true,
            vec![filter],
        );
        let auth = Authentication::from_token(api_token());
        let results = client
            .search(index, request, auth, &TracingContext::dummy())
            .await
            .unwrap();

        // Then we don't get any results
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn search_not_configured_returns_error() {
        // Given a search client that is not configured
        let client = SearchNotConfigured;

        // When making a query
        let index = IndexPath::new("Kernel", "test", "asym-64");
        let auth = Authentication::none();
        let request = SearchRequest::new(vec![], 1, None, true, vec![]);
        let results = client
            .search(index, request, auth, &TracingContext::dummy())
            .await;

        // Then we get an error
        assert!(results.is_err());
    }
}
