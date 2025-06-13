mod state;

use anyhow::Context;
use axum::{
    Json, Router,
    body::Body,
    extract::{MatchedPath, Request, State},
    http::{StatusCode, header::AUTHORIZATION},
    middleware::{self, Next},
    response::{ErrorResponse, Html, IntoResponse, Response},
    routing::get,
};
use axum_extra::{
    TypedHeader,
    headers::{self, authorization::Bearer},
};
use axum_tracing_opentelemetry::middleware::{OtelAxumLayer, OtelInResponseLayer};
use std::{future::Future, iter::once, net::SocketAddr, time::Instant};
use tokio::{net::TcpListener, task::JoinHandle};
use tower::ServiceBuilder;
use tower_http::{
    compression::CompressionLayer,
    cors::CorsLayer,
    decompression::DecompressionLayer,
    sensitive_headers::SetSensitiveRequestHeadersLayer,
    trace::{DefaultOnRequest, DefaultOnResponse, TraceLayer},
};
use tracing::{Level, error, info};
use utoipa::{
    Modify, OpenApi,
    openapi::{
        self,
        security::{HttpAuthScheme, HttpBuilder, SecurityScheme},
    },
};
use utoipa_scalar::Scalar;

use crate::{
    authorization::{AuthorizationApi, AuthorizationProvider, AuthorizationState},
    context,
    csi::RawCsi,
    csi_shell::{self, CsiProvider},
    feature_set::FeatureSet,
    logging::TracingContext,
    mcp::{McpApi, McpServerStoreProvider, http_mcp_servers_v1, openapi_mcp_servers_v1},
    namespace_watcher::Namespace,
    skill_runtime::{
        SkillRuntimeApi, SkillRuntimeProvider, http_skill_runtime_v1, openapi_skill_runtime_v1,
    },
    skill_store::{
        SkillStoreApi, SkillStoreProvider, http_skill_store_v0, http_skill_store_v1,
        openapi_skill_store_v1,
    },
    tool::{ToolApi, ToolProvider, http_tools_v1, openapi_tools_v1},
};

pub use self::state::ShellState;

pub struct Shell {
    handle: JoinHandle<()>,
}

impl Shell {
    /// Start a shell listening to incoming requests at the given address. Successful construction
    /// implies that the listener is bound to the endpoint.
    pub async fn new<T>(
        feature_set: FeatureSet,
        addr: impl Into<SocketAddr>,
        app_state: T,
        shutdown_signal: impl Future<Output = ()> + Send + 'static,
    ) -> anyhow::Result<Self>
    where
        T: AppState + Clone + Send + Sync + 'static,
        T::Csi: RawCsi + Clone + Send + Sync + 'static,
        T::Authorization: AuthorizationApi + Clone + Send + Sync + 'static,
        T::SkillRuntime: SkillRuntimeApi + Clone + Send + Sync + 'static,
        T::SkillStore: SkillStoreApi + Clone + Send + Sync + 'static,
        T::McpServerStore: McpApi + Clone + Send + Sync + 'static,
        T::Tool: ToolApi + Clone + Send + Sync + 'static,
    {
        let addr = addr.into();
        // It is important to construct the listener outside of the `spawn` invocation. We need to
        // guarantee the listener is already bound to the port, once `Self` is constructed.
        let listener = TcpListener::bind(addr)
            .await
            .context(format!("Could not bind a tcp listener to '{addr}'"))?;
        info!("Listening on: {addr}");

        let handle = tokio::spawn(async move {
            let res = axum::serve(listener, http(feature_set, app_state))
                .with_graceful_shutdown(shutdown_signal)
                .await;
            if let Err(e) = res {
                error!("Error terminating shell: {e}");
            }
        });
        Ok(Self { handle })
    }

    pub async fn wait_for_shutdown(self) {
        self.handle.await.unwrap();
    }
}

pub trait AppState:
    McpServerStoreProvider
    + SkillStoreProvider
    + SkillRuntimeProvider
    + CsiProvider
    + ToolProvider
    + AuthorizationProvider
{
}

fn v1<T>(feature_set: FeatureSet) -> Router<T>
where
    T: AppState + Clone + Send + Sync + 'static,
    T::SkillRuntime: SkillRuntimeApi + Clone + Send + Sync + 'static,
    T::SkillStore: SkillStoreApi + Clone + Send + Sync + 'static,
    T::McpServerStore: McpApi + Clone + Send + Sync + 'static,
    T::Tool: ToolApi + Clone + Send + Sync + 'static,
{
    Router::new()
        .merge(http_tools_v1(feature_set))
        .merge(http_mcp_servers_v1(feature_set))
        .merge(http_skill_store_v1(feature_set))
        .merge(http_skill_runtime_v1(feature_set))
}

fn open_api_docs(feature_set: FeatureSet) -> utoipa::openapi::OpenApi {
    // Show documentation for unstable features only in beta systems.
    let api_doc = if feature_set == FeatureSet::Beta {
        ApiDocBeta::openapi()
    } else {
        ApiDoc::openapi()
    };
    api_doc.nest(
        "v1",
        openapi_tools_v1(feature_set)
            .merge_from(openapi_mcp_servers_v1(feature_set))
            .merge_from(openapi_skill_store_v1(feature_set))
            .merge_from(openapi_skill_runtime_v1(feature_set)),
    )
}

fn http<T>(feature_set: FeatureSet, app_state: T) -> Router
where
    T: AppState + Clone + Send + Sync + 'static,
    T::Authorization: AuthorizationApi + Clone + Send + Sync + 'static,
    T::Csi: RawCsi + Clone + Sync + Send + 'static,
    T::SkillRuntime: SkillRuntimeApi + Clone + Send + Sync + 'static,
    T::SkillStore: SkillStoreApi + Clone + Send + Sync + 'static,
    T::McpServerStore: McpApi + Clone + Send + Sync + 'static,
    T::Tool: ToolApi + Clone + Send + Sync + 'static,
{
    let api_docs = open_api_docs(feature_set);
    Router::new()
        // Authenticated routes
        .nest("/v1", v1(feature_set))
        .merge(csi_shell::http())
        .merge(http_skill_store_v0(feature_set))
        .route_layer(middleware::from_fn_with_state(
            app_state.clone(),
            authorization_middleware,
        ))
        .with_state(app_state)
        // Unauthenticated routes
        .route("/", get(index))
        .route("/skill.wit", get(skill_wit()))
        .route(
            "/api-docs",
            get(async || Html(Scalar::new(api_docs).to_html())),
        )
        .route("/openapi.json", get(move || serve_docs(feature_set)))
        .route("/health", get(async || "ok"))
        .route_layer(middleware::from_fn(track_route_metrics))
        .layer(
            // ServiceBuilder nests layers unlike the router, the first layer is the outermost.
            ServiceBuilder::new()
                // Mark the `Authorization` request header as sensitive so it doesn't show in logs
                .layer(SetSensitiveRequestHeadersLayer::new(once(AUTHORIZATION)))
                .layer(OtelAxumLayer::default())
                // Inject the current context into the response, therefore needs to be nested below
                // the OtelAxumLayer for the span to still be active.
                .layer(OtelInResponseLayer)
                // Spans are created by the `OtelAxumLayer`. The `TraceLayer` adds on_request and
                // on_response events to the span, which allows us to see the duration of a
                // request in the logs.
                .layer(
                    TraceLayer::new_for_http()
                        .on_request(DefaultOnRequest::new().level(Level::INFO))
                        .on_response(DefaultOnResponse::new().level(Level::INFO)),
                )
                // Compress responses
                .layer(CompressionLayer::new())
                .layer(DecompressionLayer::new())
                .layer(CorsLayer::very_permissive()),
        )
}

async fn authorization_middleware<A>(
    State(authorization_api): State<AuthorizationState<A>>,
    bearer: Option<TypedHeader<headers::Authorization<Bearer>>>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, ErrorResponse>
where
    A: AuthorizationApi,
{
    let context = TracingContext::current();
    if let Some(bearer) = bearer {
        let context = context!(context, "pharia_kernel::authorization", "check_permissions");
        match authorization_api
            .0
            .check_permission(bearer.token().to_owned(), context)
            .await
        {
            Ok(allowed) => {
                if !allowed {
                    return Err(ErrorResponse::from((
                        StatusCode::FORBIDDEN,
                        "Bearer token invalid".to_owned(),
                    )));
                }
            }
            Err(e) => {
                return Err(ErrorResponse::from((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    e.to_string(),
                )));
            }
        }
    } else {
        return Err(ErrorResponse::from((
            StatusCode::BAD_REQUEST,
            "Bearer token expected".to_owned(),
        )));
    }
    let response = next.run(request).await;
    Ok(response)
}

pub struct HttpError {
    message: String,
    status_code: StatusCode,
}

impl HttpError {
    pub fn new(message: String, status_code: StatusCode) -> Self {
        Self {
            message,
            status_code,
        }
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> axum::response::Response {
        (self.status_code, self.message).into_response()
    }
}

pub enum ShellMetrics {
    HttpRequestsTotal,
    HttpRequestsDurationSeconds,
}

impl From<ShellMetrics> for metrics::KeyName {
    fn from(value: ShellMetrics) -> Self {
        Self::from_const_str(match value {
            ShellMetrics::HttpRequestsTotal => "kernel_http_requests_total",
            ShellMetrics::HttpRequestsDurationSeconds => "kernel_http_requests_duration_seconds",
        })
    }
}

/// Tracks which routes get called and latency for each request
async fn track_route_metrics(req: Request, next: Next) -> impl IntoResponse {
    let start = Instant::now();
    let path = if let Some(matched_path) = req.extensions().get::<MatchedPath>() {
        matched_path.as_str().to_owned()
    } else {
        req.uri().path().to_owned()
    };
    let method = req.method().to_string();

    // Run request
    let response = next.run(req).await;

    let latency = start.elapsed().as_secs_f64();
    let status = response.status().as_u16().to_string();

    let labels = [("method", method), ("path", path), ("status", status)];

    metrics::counter!(ShellMetrics::HttpRequestsTotal, &labels).increment(1);
    metrics::histogram!(ShellMetrics::HttpRequestsDurationSeconds, &labels).record(latency);
    response
}

#[derive(OpenApi)]
#[openapi(
    info(description = "The best place to run serverless AI applications."),
    paths(serve_docs),
    modifiers(&SecurityAddon),
    components(schemas(Namespace)),
    tags(
        (name = "skills"),
        (name = "docs"),
    )
)]
struct ApiDoc;

#[derive(OpenApi)]
#[openapi(
    info(description = "Pharia Kernel (Beta): The best place to run serverless AI applications."),
    paths(serve_docs, skill_wit),
    modifiers(&SecurityAddon),
    components(schemas(Namespace)),
    tags(
        (name = "skills"),
        (name = "docs"),
    )
)]
struct ApiDocBeta;

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "api_token",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("JWT")
                    .build(),
            ),
        );
    }
}

async fn index() -> Html<&'static str> {
    const INDEX: &str = include_str!("./shell/index.html");
    Html(INDEX)
}

/// openapi.json
///
/// Return JSON version of an OpenAPI schema
#[utoipa::path(
    get,
    path = "/openapi.json",
    tag = "docs",
    responses(
        (status = 200, description = "JSON file", body = ())
    ),
)]
#[allow(clippy::unused_async)]
async fn serve_docs(feature_set: FeatureSet) -> Json<openapi::OpenApi> {
    Json(open_api_docs(feature_set))
}

/// WIT (WebAssembly Interface Types) of Skills
///
/// Skills are WebAssembly components built against a WIT world. This route returns this WIT world.
#[utoipa::path(
    get,
    operation_id = "get_skill_wit",
    path = "/skill.wit",
    tag = "docs",
    responses(
        (status = 200, description = "Ok", body=String),
    ),
)]
fn skill_wit() -> &'static str {
    include_str!("../wit/skill@0.3/skill.wit")
}

#[cfg(test)]
pub mod tests {
    use std::sync::{Arc, Mutex};

    use crate::{
        authorization::tests::StubAuthorization,
        feature_set::PRODUCTION_FEATURE_SET,
        logging::tests::given_tracing_subscriber,
        skill_driver::{SkillExecutionError, SkillExecutionEvent},
        skill_runtime::SkillRuntimeDouble,
        skills::{AnySkillManifest, SkillPath},
        tests::api_token,
    };

    use super::*;

    use axum::{
        body::Body,
        http::{Method, Request, header},
    };

    use double_trait::Dummy;
    use http_body_util::BodyExt;
    use mime::APPLICATION_JSON;
    use reqwest::header::CONTENT_TYPE;
    use serde_json::{Value, json};
    use tokio::sync::mpsc;
    use tower::util::ServiceExt;

    impl<A, R> AppState for AppStateDouble<A, R>
    where
        A: Clone,
        R: Clone,
    {
    }

    /// State shared between routes
    #[derive(Clone)]
    struct AppStateDouble<A, R>
    where
        A: Clone,
        R: Clone,
    {
        authorization_api: A,
        skill_runtime_api: R,
    }

    impl<A, R> AuthorizationProvider for AppStateDouble<A, R>
    where
        A: Clone,
        R: Clone,
    {
        type Authorization = A;

        fn authorization(&self) -> &A {
            &self.authorization_api
        }
    }

    impl<A, R> McpServerStoreProvider for AppStateDouble<A, R>
    where
        A: Clone,
        R: Clone,
    {
        type McpServerStore = Dummy;

        fn mcp_server_store(&self) -> &Dummy {
            &Dummy
        }
    }

    impl<A, R> CsiProvider for AppStateDouble<A, R>
    where
        A: Clone,
        R: Clone,
    {
        type Csi = Dummy;

        fn csi(&self) -> &Dummy {
            &Dummy
        }
    }

    impl<A, R> SkillStoreProvider for AppStateDouble<A, R>
    where
        A: Clone,
        R: Clone,
    {
        type SkillStore = Dummy;

        fn skill_store(&self) -> &Dummy {
            &Dummy
        }
    }

    impl<A, R> SkillRuntimeProvider for AppStateDouble<A, R>
    where
        A: Clone,
        R: Clone,
    {
        type SkillRuntime = R;

        fn skill_runtime(&self) -> &R {
            &self.skill_runtime_api
        }
    }

    impl<A, R> ToolProvider for AppStateDouble<A, R>
    where
        A: Clone,
        R: Clone,
    {
        type Tool = Dummy;

        fn tool(&self) -> &Dummy {
            &Dummy
        }
    }

    impl AppStateDouble<StubAuthorization, Dummy> {
        pub fn dummy() -> Self {
            Self {
                authorization_api: StubAuthorization::new(true),
                skill_runtime_api: Dummy,
            }
        }
    }

    impl<A, R> AppStateDouble<A, R>
    where
        A: AuthorizationApi + Clone + Sync + Send + 'static,
        R: SkillRuntimeApi + Clone + Send + Sync + 'static,
    {
        pub fn new(authorization_api: A, skill_runtime_api: R) -> Self {
            Self {
                authorization_api,
                skill_runtime_api,
            }
        }

        pub fn with_authorization_api<A2>(self, authorization_api: A2) -> AppStateDouble<A2, R>
        where
            A2: AuthorizationApi + Clone + Sync + Send + 'static,
        {
            AppStateDouble::new(authorization_api, self.skill_runtime_api)
        }

        pub fn with_skill_runtime_api<R2>(self, skill_runtime_api: R2) -> AppStateDouble<A, R2>
        where
            R2: SkillRuntimeApi + Clone + Send + Sync + 'static,
        {
            AppStateDouble::new(self.authorization_api, skill_runtime_api)
        }
    }

    pub fn dummy_auth_value() -> header::HeaderValue {
        let api_token = "dummy auth token";
        let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
        auth_value.set_sensitive(true);
        auth_value
    }

    #[tokio::test]
    async fn api_token_missing_permission() {
        // Given
        let stub_authorization = StubAuthorization::new(false);
        let app_state = AppStateDouble::dummy().with_authorization_api(stub_authorization);

        // When we want to access an endpoint that requires authentication
        let api_token = api_token();
        let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
        auth_value.set_sensitive(true);

        let http = http(PRODUCTION_FEATURE_SET, app_state);
        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, auth_value)
                    .uri("/v1/skills/local/greet_skill/run")
                    .body(Body::from(serde_json::to_string(&json!("Homer")).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then
        assert_eq!(resp.status(), axum::http::StatusCode::FORBIDDEN);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            String::from_utf8(body.to_vec()).unwrap(),
            "Bearer token invalid".to_owned()
        );
    }

    #[tokio::test]
    async fn api_token_missing_in_run_skill() {
        // Given
        let app_state = AppStateDouble::dummy();

        // When
        let http = http(PRODUCTION_FEATURE_SET, app_state);
        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, APPLICATION_JSON.as_ref())
                    .uri("/v1/skills/local/greet_skill/run")
                    .body(Body::from(serde_json::to_string(&json!("Homer")).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            String::from_utf8(body.to_vec()).unwrap(),
            "Bearer token expected".to_owned()
        );
    }

    #[tokio::test]
    async fn health() {
        let app_state = AppStateDouble::dummy();
        let http = http(PRODUCTION_FEATURE_SET, app_state);
        let resp = http
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"ok");
    }

    #[tokio::test]
    async fn skill_wit_route_should_return_current_wit_world() {
        let app_state = AppStateDouble::dummy();
        let http = http(PRODUCTION_FEATURE_SET, app_state);
        let resp = http
            .oneshot(
                Request::builder()
                    .uri("/skill.wit")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let actual = String::from_utf8(body.to_vec()).unwrap();

        assert_eq!(actual, include_str!("../wit/skill@0.3/skill.wit"));
    }

    #[tokio::test]
    async fn cannot_list_skills_without_permissions() {
        // Given we have a saboteur authorization
        let saboteur_authorization = StubAuthorization::new(false);
        let app_state = AppStateDouble::dummy().with_authorization_api(saboteur_authorization);
        let http = http(PRODUCTION_FEATURE_SET, app_state);

        // When
        let api_token = api_token();
        let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
        auth_value.set_sensitive(true);

        let resp = http
            .oneshot(
                Request::builder()
                    .uri("/v1/skills")
                    .header(AUTHORIZATION, auth_value)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            String::from_utf8(body.to_vec()).unwrap(),
            "Bearer token invalid".to_owned()
        );
    }

    #[tokio::test]
    async fn invalid_namespace_config_is_500_error() {
        // Given a skill runtime which has an invalid namespace
        let skill_runtime =
            SkillRuntimeSaboteur::new(|| SkillExecutionError::MisconfiguredNamespace {
                namespace: Namespace::new("playground").unwrap(),
                original_syntax_error: "error msg".to_owned(),
            });
        let app_state = AppStateDouble::dummy().with_skill_runtime_api(skill_runtime);
        let http = http(PRODUCTION_FEATURE_SET, app_state);

        // When executing a skill in the namespace
        let auth_value = header::HeaderValue::from_str("Bearer DummyToken").unwrap();
        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, auth_value)
                    .uri("/v1/skills/any-namespace/any_skill/run")
                    .body(Body::from(serde_json::to_string(&json!("Homer")).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then the response is 500 about invalid namespace
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let response = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(
            response,
            "The skill could not be executed to completion, the namespace 'playground' is \
            misconfigured. If you are the developer who configured the skill, you should probably \
            fix this error. If you are not, there is nothing you can do, until the developer who \
            maintains the list of skills to be served, fixes this. Original Syntax error:\n\n\
            error msg"
        );
    }

    /// A test helper answering each request with a predefined error
    #[derive(Clone)]
    struct SkillRuntimeSaboteur {
        make_error: Arc<dyn Fn() -> SkillExecutionError + Send + Sync>,
    }

    impl SkillRuntimeSaboteur {
        pub fn new(error: impl Fn() -> SkillExecutionError + Send + Sync + 'static) -> Self {
            Self {
                make_error: Arc::new(error),
            }
        }
    }

    impl SkillRuntimeDouble for SkillRuntimeSaboteur {
        async fn run_function(
            &self,
            _skill_path: SkillPath,
            _input: Value,
            _api_token: String,
            _tracing_context: TracingContext,
        ) -> Result<Value, SkillExecutionError> {
            Err((*self.make_error)())
        }

        async fn skill_metadata(
            &self,
            _skill_path: SkillPath,
            _tracing_context: TracingContext,
        ) -> Result<AnySkillManifest, SkillExecutionError> {
            Err((*self.make_error)())
        }
    }

    /// A test helper to snoop on parameters send to the skill runtime
    #[derive(Clone)]
    struct SkillRuntimeSpy {
        inner: Arc<Mutex<SkillRuntimeSpyInner>>,
    }

    struct SkillRuntimeSpyInner {
        api_token: String,
        input: Value,
        skill_path: SkillPath,
        tracing_context: Vec<TracingContext>,
    }

    impl SkillRuntimeSpy {
        pub fn new() -> Self {
            Self {
                inner: Arc::new(Mutex::new(SkillRuntimeSpyInner {
                    api_token: String::new(),
                    input: Value::default(),
                    skill_path: SkillPath::local("SKILL HAS NOT BEEN SEND"),
                    tracing_context: Vec::new(),
                })),
            }
        }

        pub fn tracing_contexts(&self) -> Vec<TracingContext> {
            self.inner.lock().unwrap().tracing_context.clone()
        }
    }

    impl SkillRuntimeDouble for SkillRuntimeSpy {
        async fn run_function(
            &self,
            skill_path: SkillPath,
            input: Value,
            api_token: String,
            tracing_context: TracingContext,
        ) -> Result<Value, SkillExecutionError> {
            let mut inner = self.inner.lock().unwrap();
            inner.api_token = api_token;
            inner.input = input;
            inner.skill_path = skill_path;
            inner.tracing_context.push(tracing_context);
            Ok(Value::default())
        }

        async fn run_message_stream(
            &self,
            skill_path: SkillPath,
            input: Value,
            api_token: String,
            tracing_context: TracingContext,
        ) -> mpsc::Receiver<SkillExecutionEvent> {
            let mut inner = self.inner.lock().unwrap();
            inner.api_token = api_token;
            inner.input = input;
            inner.skill_path = skill_path;
            inner.tracing_context.push(tracing_context);
            let (_send, recv) = mpsc::channel(1);
            recv
        }
    }

    #[tokio::test]
    async fn trace_parent_is_read_from_incoming_request() {
        // We need to setup a tracing subscriber to actually get spans. If there is no subscriber
        // spans will not be created as no one is interested in them.
        given_tracing_subscriber();

        let app_state = AppStateDouble::dummy();
        let http = http(PRODUCTION_FEATURE_SET, app_state);

        // When doing a request with a traceparent header
        let trace_id = "0af7651916cd43dd8448eb211c80319c";
        let span_id = "b7ad6b7169203331";
        let traceparent = format!("00-{trace_id}-{span_id}-01");
        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/")
                    .header("traceparent", traceparent)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);

        // Then we get the traceparent header in the response
        let traceparent = resp.headers().get("traceparent").unwrap();
        assert!(traceparent.to_str().unwrap().contains(trace_id));
    }

    #[tokio::test]
    async fn skill_runtime_gets_tracecontext_from_incoming_request() {
        given_tracing_subscriber();

        // Given a shell
        let skill_runtime = SkillRuntimeSpy::new();
        let app_state = AppStateDouble::dummy().with_skill_runtime_api(skill_runtime.clone());
        let http = http(PRODUCTION_FEATURE_SET, app_state);

        // When a request with a trace id comes in
        let trace_id = "0af7651916cd43dd8448eb211c80319c";
        let span_id = "b7ad6b7169203331";
        let traceparent = format!("00-{trace_id}-{span_id}-01");
        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, dummy_auth_value())
                    .header("traceparent", traceparent)
                    .uri("/v1/skills/acme/summarize/run")
                    .body(Body::from(serde_json::to_string(&json!("Homer")).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then the skill runtime receives the trace id
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            skill_runtime.tracing_contexts()[0].trace_id().to_string(),
            trace_id
        );
    }

    #[tokio::test]
    async fn tracestate_is_extracted_from_incoming_request() {
        given_tracing_subscriber();

        // Given a shell
        let skill_runtime = SkillRuntimeSpy::new();
        let app_state = AppStateDouble::dummy().with_skill_runtime_api(skill_runtime.clone());
        let http = http(PRODUCTION_FEATURE_SET, app_state);

        // When a request with a tracestate header comes in
        let trace_id = "0af7651916cd43dd8448eb211c80319c";
        let span_id = "b7ad6b7169203331";
        let traceparent = format!("00-{trace_id}-{span_id}-01");
        let tracestate = "foo=bar";
        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, dummy_auth_value())
                    .header("traceparent", traceparent)
                    .header("tracestate", tracestate)
                    .uri("/v1/skills/acme/summarize/run")
                    .body(Body::from(serde_json::to_string(&json!("Homer")).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then the tracestate can be reconstructed from the tracing context
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            skill_runtime.tracing_contexts()[0]
                .w3c_headers()
                .get("tracestate")
                .unwrap()
                .to_str()
                .unwrap(),
            tracestate
        );
    }
}
