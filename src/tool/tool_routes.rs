use axum::{
    Json, Router,
    extract::{FromRef, Path, State},
    routing::get,
};
use utoipa::OpenApi;

use crate::{FeatureSet, namespace_watcher::Namespace, tool::ToolApi};

pub fn http_tools_v1<T>(feature_set: FeatureSet) -> Router<T>
where
    T: Send + Sync + Clone + ToolProvider + 'static,
    T::Tool: ToolApi + Send + Clone,
{
    // Additional routes for beta features, please keep them in sync with the API documentation.
    if feature_set == FeatureSet::Beta {
        Router::new().route("/tools/{namespace}", get(list_tools))
    } else {
        Router::new()
    }
}

pub fn openapi_tools_v1(feature_set: FeatureSet) -> utoipa::openapi::OpenApi {
    if feature_set == FeatureSet::Beta {
        ToolOpenApiDocBeta::openapi()
    } else {
        ToolOpenApiDoc::openapi()
    }
}

#[derive(OpenApi)]
#[openapi(paths())]
struct ToolOpenApiDoc;

#[derive(OpenApi)]
#[openapi(paths(list_tools))]
struct ToolOpenApiDocBeta;

pub trait ToolProvider {
    type Tool;
    fn tool(&self) -> &Self::Tool;
}

/// Wrapper around Tool Api for the shell. We use this strict alias to enable extracting a reference
/// from the [`AppState`] using a [`FromRef`] implementation.
pub struct ToolState<M>(pub M);

impl<T> FromRef<T> for ToolState<T::Tool>
where
    T: ToolProvider,
    T::Tool: Clone,
{
    fn from_ref(app_state: &T) -> ToolState<T::Tool> {
        ToolState(app_state.tool().clone())
    }
}

/// List of all tools configured for this namespace
#[utoipa::path(
    get,
    operation_id = "list_tools",
    path = "/tools/{namespace}",
    tag = "tools",
    security(("api_token" = [])),
    responses(
        (status = 200, body=Vec<String>, example = json!(["add", "divide"])),
    ),
)]
async fn list_tools<T>(
    Path(namespace): Path<Namespace>,
    State(ToolState(tool)): State<ToolState<T>>,
) -> Json<Vec<String>>
where
    T: ToolApi,
{
    let mut tools = tool.list_tools(namespace).await;
    tools.sort();
    Json(tools)
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt as _;
    use reqwest::Method;
    use tower::ServiceExt as _;

    use super::{ToolProvider, http_tools_v1};
    use crate::{FeatureSet, namespace_watcher::Namespace, tool::actor::ToolDouble};

    #[derive(Clone)]
    struct ProviderStub<T> {
        tool: T,
    }

    impl<T> ProviderStub<T> {
        fn new(tool: T) -> Self {
            Self { tool }
        }
    }

    impl<T> ToolProvider for ProviderStub<T> {
        type Tool = T;

        fn tool(&self) -> &T {
            &self.tool
        }
    }

    #[tokio::test]
    async fn tools_per_namespace_are_listed_and_sorted() {
        // Given a tool mock that returns an unordered list of tools
        #[derive(Clone)]
        struct ToolMock;
        impl ToolDouble for ToolMock {
            async fn list_tools(&self, namespace: Namespace) -> Vec<String> {
                assert_eq!(namespace, Namespace::new("my-test-namespace").unwrap());
                vec!["divide".to_owned(), "add".to_owned()]
            }
        }
        let app_state = ProviderStub::new(ToolMock);
        let http = http_tools_v1(FeatureSet::Beta).with_state(app_state);

        // When
        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/tools/my-test-namespace")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then we get a sorted list of tools
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let answer = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(answer, "[\"add\",\"divide\"]");
    }
}
