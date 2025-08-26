use axum::{
    Json, Router,
    extract::{FromRef, Path, State},
    http::StatusCode,
    response::Result,
    routing::get,
};
use utoipa::OpenApi;

use crate::{
    FeatureSet,
    mcp::{McpApi, McpServerUrl},
    namespace_watcher::Namespace,
};

pub fn http_mcp_servers_v1<T>(_feature_set: FeatureSet) -> Router<T>
where
    T: Send + Sync + Clone + McpServerStoreProvider + 'static,
    T::McpServerStore: McpApi + Send + Clone,
{
    Router::new().route("/mcp_servers/{namespace}", get(list_mcp_servers))
}

pub fn openapi_mcp_servers_v1(feature_set: FeatureSet) -> utoipa::openapi::OpenApi {
    if feature_set == FeatureSet::Beta {
        McpOpenApiDocBeta::openapi()
    } else {
        McpOpenApiDoc::openapi()
    }
}

#[derive(OpenApi)]
#[openapi(paths(list_mcp_servers))]
struct McpOpenApiDoc;

#[derive(OpenApi)]
#[openapi(paths(list_mcp_servers))]
struct McpOpenApiDocBeta;

pub trait McpServerStoreProvider {
    type McpServerStore;
    fn mcp_server_store(&self) -> &Self::McpServerStore;
}

/// Wrapper around Tool Api for the shell. We use this strict alias to enable extracting a reference
/// from the [`AppState`] using a [`FromRef`] implementation.
pub struct McpServerStoreState<M>(pub M);

impl<T> FromRef<T> for McpServerStoreState<T::McpServerStore>
where
    T: McpServerStoreProvider,
    T::McpServerStore: Clone,
{
    fn from_ref(app_state: &T) -> McpServerStoreState<T::McpServerStore> {
        McpServerStoreState(app_state.mcp_server_store().clone())
    }
}

/// List of all mcp servers configured for this namespace
#[utoipa::path(
    get,
    operation_id = "list_mcp_servers",
    path = "/mcp_servers/{namespace}",
    tag = "tools",
    security(("api_token" = [])),
    responses(
        (status = 200, body=Vec<String>, example = json!(["http://localhost:8000/mcp"])),
        (status = 404, body=String, example = "Namespace 'unknown-namespace' not found"),
    ),
)]
async fn list_mcp_servers<M>(
    Path(namespace): Path<Namespace>,
    State(McpServerStoreState(mcp_servers)): State<McpServerStoreState<M>>,
) -> Result<Json<Vec<McpServerUrl>>, (StatusCode, Json<String>)>
where
    M: McpApi,
{
    if let Ok(servers) = mcp_servers.mcp_list(namespace.clone()).await {
        Ok(Json(servers))
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(format!("Namespace '{namespace}' not found")),
        ))
    }
}

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt as _;
    use reqwest::Method;
    use tower::ServiceExt as _;

    use super::{McpServerStoreProvider, McpServerUrl, http_mcp_servers_v1};
    use crate::{
        feature_set::PRODUCTION_FEATURE_SET,
        mcp::{McpApi, store::NamespaceNotFound},
        namespace_watcher::Namespace,
    };

    #[derive(Clone)]
    struct ProviderStub<T> {
        mcp_server_store: T,
    }

    impl<T> ProviderStub<T> {
        fn new(mcp_server_store: T) -> Self {
            Self { mcp_server_store }
        }
    }

    impl<T> McpServerStoreProvider for ProviderStub<T> {
        type McpServerStore = T;

        fn mcp_server_store(&self) -> &T {
            &self.mcp_server_store
        }
    }

    #[tokio::test]
    async fn list_mcp_servers_by_namespace() {
        #[derive(Clone)]
        struct McpServerMock;
        impl McpApi for McpServerMock {
            async fn mcp_list(
                &self,
                namespace: Namespace,
            ) -> Result<Vec<McpServerUrl>, NamespaceNotFound> {
                assert_eq!(namespace, Namespace::new("my-test-namespace").unwrap());
                Ok(vec![McpServerUrl("http://localhost:8083/mcp".to_owned())])
            }
        }
        let app_state = ProviderStub::new(McpServerMock);
        let http = http_mcp_servers_v1(PRODUCTION_FEATURE_SET).with_state(app_state);

        // When
        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/mcp_servers/my-test-namespace")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let answer = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(answer, "[\"http://localhost:8083/mcp\"]");
    }

    #[tokio::test]
    async fn listing_tools_for_non_existing_namespace_returns_404() {
        // Given a an mcp server store that does not know about a namespace
        #[derive(Clone)]
        struct McpServerMock;
        impl McpApi for McpServerMock {
            async fn mcp_list(
                &self,
                _namespace: Namespace,
            ) -> Result<Vec<McpServerUrl>, NamespaceNotFound> {
                Err(NamespaceNotFound)
            }
        }
        let app_state = ProviderStub::new(McpServerMock);
        let http = http_mcp_servers_v1(PRODUCTION_FEATURE_SET).with_state(app_state);

        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/mcp_servers/my-test-namespace")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then we get a 404
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    }
}
