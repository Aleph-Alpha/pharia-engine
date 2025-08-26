use axum::{
    Json, Router,
    extract::{FromRef, Path, State},
    http::StatusCode,
    routing::get,
};
use utoipa::OpenApi;

use crate::{
    FeatureSet,
    namespace_watcher::Namespace,
    tool::{ToolDescription, ToolRuntimeApi},
};

pub fn http_tools_v1<T>(_feature_set: FeatureSet) -> Router<T>
where
    T: Send + Sync + Clone + ToolProvider + 'static,
    T::Tool: ToolRuntimeApi + Send + Clone,
{
    Router::new().route("/tools/{namespace}", get(list_tools))
}

pub fn openapi_tools_v1(feature_set: FeatureSet) -> utoipa::openapi::OpenApi {
    if feature_set == FeatureSet::Beta {
        ToolOpenApiDocBeta::openapi()
    } else {
        ToolOpenApiDoc::openapi()
    }
}

#[derive(OpenApi)]
#[openapi(paths(list_tools))]
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
        (status = 200, body=Vec<ToolDescription>, example = json!([
            {
                "name": "add",
                "description": "Add two numbers",
                "inputSchema": {
                    "properties": {
                        "a": {
                            "title": "A",
                            "type": "integer"
                        },
                        "b": {
                            "title": "B",
                            "type": "integer"
                        }
                    },
                    "required": [
                        "a",
                        "b"
                    ],
                    "title": "addArguments",
                    "type": "object"
                }
            },
            {
                "name": "saboteur",
                "description": "I have ran out of cheese.",
                "inputSchema": {
                    "properties": {},
                    "title": "saboteurArguments",
                    "type": "object"
                }
            },
        ])),
    (status = 404, body=String, example = json!("Namespace 'unknown-namespace' not found")),
    ),
)]
async fn list_tools<T>(
    Path(namespace): Path<Namespace>,
    State(ToolState(tool)): State<ToolState<T>>,
) -> Result<Json<Vec<ToolDescription>>, (StatusCode, Json<String>)>
where
    T: ToolRuntimeApi,
{
    if let Ok(mut tools) = tool.list_tools(namespace.clone()).await {
        tools.sort();
        Ok(Json(tools))
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(format!("Namespace '{namespace}' not found")),
        ))
    }
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt as _;
    use reqwest::Method;
    use serde_json::json;
    use tower::ServiceExt as _;

    use super::{ToolProvider, http_tools_v1};
    use crate::{
        feature_set::PRODUCTION_FEATURE_SET,
        namespace_watcher::Namespace,
        tool::{ToolDescription, ToolRuntimeApi, toolbox::NamespaceNotFound},
    };

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
        impl ToolRuntimeApi for ToolMock {
            async fn list_tools(
                &self,
                namespace: Namespace,
            ) -> Result<Vec<ToolDescription>, NamespaceNotFound> {
                assert_eq!(namespace, Namespace::new("my-test-namespace").unwrap());
                Ok(vec![
                    ToolDescription::new("divide", "Divide two numbers", json!({})),
                    ToolDescription::new("add", "Add two numbers", json!({})),
                ])
            }
        }
        let app_state = ProviderStub::new(ToolMock);
        let http = http_tools_v1(PRODUCTION_FEATURE_SET).with_state(app_state);

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
        assert_eq!(
            answer,
            "[{\"name\":\"add\",\"description\":\"Add two numbers\",\"input_schema\":{}},{\"name\":\"divide\",\"description\":\"Divide two numbers\",\"input_schema\":{}}]"
        );
    }

    #[tokio::test]
    async fn request_for_unknown_namespace_returns_404() {
        // Given a tool that does not know about any namespace
        #[derive(Clone)]
        struct ToolMock;
        impl ToolRuntimeApi for ToolMock {
            async fn list_tools(
                &self,
                _namespace: Namespace,
            ) -> Result<Vec<ToolDescription>, NamespaceNotFound> {
                Err(NamespaceNotFound)
            }
        }
        let app_state = ProviderStub::new(ToolMock);
        let http = http_tools_v1(PRODUCTION_FEATURE_SET).with_state(app_state);

        // When
        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/tools/unknown-namespace")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then we get a 404
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
