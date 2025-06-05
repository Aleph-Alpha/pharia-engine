use anyhow::Context;
use async_stream::try_stream;
use axum::{
    Json, Router,
    body::Body,
    extract::{FromRef, MatchedPath, Path, Query, Request, State},
    http::{StatusCode, header::AUTHORIZATION},
    middleware::{self, Next},
    response::{ErrorResponse, Html, IntoResponse, Response, Sse, sse::Event},
    routing::{delete, get, post},
};
use axum_extra::{
    TypedHeader,
    headers::{self, Authorization, authorization::Bearer},
};
use axum_tracing_opentelemetry::middleware::{OtelAxumLayer, OtelInResponseLayer};
use futures::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{convert::Infallible, future::Future, iter::once, net::SocketAddr, time::Instant};
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
    Modify, OpenApi, ToSchema,
    openapi::{
        self,
        security::{HttpAuthScheme, HttpBuilder, SecurityScheme},
    },
};
use utoipa_scalar::Scalar;

use crate::{
    authorization::AuthorizationApi,
    context,
    csi::RawCsi,
    csi_shell,
    feature_set::FeatureSet,
    logging::TracingContext,
    namespace_watcher::Namespace,
    skill_loader::SkillDescriptionFilterType,
    skill_runtime::{SkillExecutionError, SkillExecutionEvent, SkillRuntimeApi},
    skill_store::SkillStoreApi,
    skills::{AnySkillManifest, JsonSchema, Signature, SkillPath},
    tool::{McpServerStoreApi, McpServerUrl},
};

pub struct Shell {
    handle: JoinHandle<()>,
}

impl Shell {
    /// Start a shell listening to incoming requests at the given address. Successful construction
    /// implies that the listener is bound to the endpoint.
    pub async fn new(
        feature_set: FeatureSet,
        addr: impl Into<SocketAddr>,
        authorization_api: impl AuthorizationApi + Clone + Send + Sync + 'static,
        skill_runtime_api: impl SkillRuntimeApi + Clone + Send + Sync + 'static,
        skill_store_api: impl SkillStoreApi + Clone + Send + Sync + 'static,
        mcp_servers: impl McpServerStoreApi + Clone + Send + Sync + 'static,
        csi_drivers: impl RawCsi + Clone + Send + Sync + 'static,
        shutdown_signal: impl Future<Output = ()> + Send + 'static,
    ) -> anyhow::Result<Self> {
        let addr = addr.into();
        // It is important to construct the listener outside of the `spawn` invocation. We need to
        // guarantee the listener is already bound to the port, once `Self` is constructed.
        let listener = TcpListener::bind(addr)
            .await
            .context(format!("Could not bind a tcp listener to '{addr}'"))?;
        info!("Listening on: {addr}");

        let app_state = AppState::new(
            authorization_api,
            skill_store_api,
            skill_runtime_api,
            mcp_servers,
            csi_drivers,
        );
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

/// State shared between routes
#[derive(Clone)]
pub struct AppState<A, C, R, S, M>
where
    A: Clone,
    C: Clone,
    R: Clone,
    S: Clone,
    M: Clone,
{
    authorization_api: A,
    skill_store_api: S,
    skill_runtime_api: R,
    csi_drivers: C,
    mcp_servers: M,
}

impl<A, C, R, S, M> AppState<A, C, R, S, M>
where
    A: AuthorizationApi + Clone,
    C: RawCsi + Clone + Sync + Send + 'static,
    R: SkillRuntimeApi + Clone,
    S: SkillStoreApi + Clone,
    M: McpServerStoreApi + Clone,
{
    pub fn new(
        authorization_api: A,
        skill_store_api: S,
        skill_runtime_api: R,
        mcp_servers: M,
        csi_drivers: C,
    ) -> Self {
        Self {
            authorization_api,
            skill_store_api,
            skill_runtime_api,
            csi_drivers,
            mcp_servers,
        }
    }
}

/// Wrapper used to extract [`AuthorizationApi`] api from the [`AppState`] using a [`FromRef`] implementation.
struct AuthorizationState<A>(pub A);

impl<A, C, R, S, M> FromRef<AppState<A, C, R, S, M>> for AuthorizationState<A>
where
    A: Clone,
    C: Clone,
    R: Clone,
    S: Clone,
    M: Clone,
{
    fn from_ref(app_state: &AppState<A, C, R, S, M>) -> AuthorizationState<A> {
        AuthorizationState(app_state.authorization_api.clone())
    }
}

/// Wrapper used to extract [`Csi`] api from the [`AppState`] using a [`FromRef`] implementation.
pub struct CsiState<C>(pub C);

impl<A, C, R, S, M> FromRef<AppState<A, C, R, S, M>> for CsiState<C>
where
    A: Clone,
    C: Clone,
    R: Clone,
    S: Clone,
    M: Clone,
{
    fn from_ref(app_state: &AppState<A, C, R, S, M>) -> CsiState<C> {
        CsiState(app_state.csi_drivers.clone())
    }
}

/// Wrapper around Skill runtime Api for the shell. We use this strict alias to enable extracting a
/// reference from the [`AppState`] using a [`FromRef`] implementation.
struct SkillRuntimeState<R>(pub R);

impl<A, C, R, S, M> FromRef<AppState<A, C, R, S, M>> for SkillRuntimeState<R>
where
    A: Clone,
    C: Clone,
    R: Clone,
    S: Clone,
    M: Clone,
{
    fn from_ref(app_state: &AppState<A, C, R, S, M>) -> SkillRuntimeState<R> {
        SkillRuntimeState(app_state.skill_runtime_api.clone())
    }
}

/// Wrapper around Skill runtime Api for the shell. We use this strict alias to enable extracting a
/// reference from the [`AppState`] using a [`FromRef`] implementation.
struct SkillStoreState<S>(pub S);

impl<A, C, R, S, M> FromRef<AppState<A, C, R, S, M>> for SkillStoreState<S>
where
    A: Clone,
    C: Clone,
    R: Clone,
    S: Clone,
    M: Clone,
{
    fn from_ref(app_state: &AppState<A, C, R, S, M>) -> SkillStoreState<S> {
        SkillStoreState(app_state.skill_store_api.clone())
    }
}

/// Wrapper around Skill runtime Api for the shell. We use this strict alias to enable extracting a
/// reference from the [`AppState`] using a [`FromRef`] implementation.
struct McpServerStoreState<M>(pub M);

impl<A, C, R, S, M> FromRef<AppState<A, C, R, S, M>> for McpServerStoreState<M>
where
    A: Clone,
    C: Clone,
    R: Clone,
    S: Clone,
    M: Clone,
{
    fn from_ref(app_state: &AppState<A, C, R, S, M>) -> McpServerStoreState<M> {
        McpServerStoreState(app_state.mcp_servers.clone())
    }
}

fn v1<A, C, R, S, M>(feature_set: FeatureSet) -> Router<AppState<A, C, R, S, M>>
where
    A: AuthorizationApi + Clone + Send + Sync + 'static,
    C: RawCsi + Clone + Sync + Send + 'static,
    R: SkillRuntimeApi + Clone + Send + Sync + 'static,
    S: SkillStoreApi + Clone + Send + Sync + 'static,
    M: McpServerStoreApi + Clone + Send + Sync + 'static,
{
    let skills_route = if feature_set == FeatureSet::Beta {
        get(skills_beta)
    } else {
        get(skills)
    };

    let mut router = Router::new()
        .route("/skills", skills_route)
        .route("/skills/{namespace}/{name}/metadata", get(skill_metadata))
        .route("/skills/{namespace}/{name}/run", post(run_skill))
        .route(
            "/skills/{namespace}/{name}/message-stream",
            post(message_stream_skill),
        );

    // Additional routes for beta features, please keep them in sync with the API documentation.
    if feature_set == FeatureSet::Beta {
        router = router.route("/mcp_servers/{namespace}", get(list_mcp_servers));
    }

    router
}

fn http<A, C, R, S, M>(feature_set: FeatureSet, app_state: AppState<A, C, R, S, M>) -> Router
where
    A: AuthorizationApi + Clone + Send + Sync + 'static,
    C: RawCsi + Clone + Sync + Send + 'static,
    R: SkillRuntimeApi + Clone + Send + Sync + 'static,
    S: SkillStoreApi + Clone + Send + Sync + 'static,
    M: McpServerStoreApi + Clone + Send + Sync + 'static,
{
    // Show documentation for unstable features only in beta systems.
    let api_doc = if feature_set == FeatureSet::Beta {
        ApiDocBeta::openapi()
    } else {
        ApiDoc::openapi()
    };

    Router::new()
        // Authenticated routes
        .nest("/v1", v1(feature_set))
        .merge(csi_shell::http())
        // Hidden routes for cache for internal use
        .route("/cached_skills", get(cached_skills))
        .route(
            "/cached_skills/{namespace}/{name}",
            delete(drop_cached_skill),
        )
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
            get(async || Html(Scalar::new(api_doc).to_html())),
        )
        .route("/openapi.json", get(serve_docs))
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

struct HttpError {
    message: String,
    status_code: StatusCode,
}

impl HttpError {
    fn new(message: String, status_code: StatusCode) -> Self {
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

impl From<SkillExecutionError> for HttpError {
    fn from(value: SkillExecutionError) -> Self {
        let status_code = match &value {
            // We return 5xx not only for runtime errors, but also for errors we consider Bugs in
            // the deployed skills.
            SkillExecutionError::MisconfiguredNamespace { .. }
            | SkillExecutionError::CsiUseFromMetadata
            | SkillExecutionError::InvalidOutput(_)
            | SkillExecutionError::MessageAppendWithoutMessageBegin
            | SkillExecutionError::MessageBeginWhileMessageActive
            | SkillExecutionError::MessageEndWithoutMessageBegin
            | SkillExecutionError::SkillLoadError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            // Service unavailable indicates better that the situation is temporary and retrying it
            // might be worth it.
            SkillExecutionError::RuntimeError(_) => StatusCode::SERVICE_UNAVAILABLE,
            // 400 for every error we see an error on the client side of HTTP
            SkillExecutionError::UserCode(_)
            | SkillExecutionError::SkillNotConfigured
            | SkillExecutionError::IsFunction
            | SkillExecutionError::IsMessageStream
            | SkillExecutionError::InvalidInput(_) => StatusCode::BAD_REQUEST,
        };
        HttpError::new(value.to_string(), status_code)
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
    paths(serve_docs, skills, run_skill, message_stream_skill),
    modifiers(&SecurityAddon),
    components(schemas(ExecuteSkillArgs, Namespace)),
    tags(
        (name = "skills"),
        (name = "docs"),
    )
)]
struct ApiDoc;

#[derive(OpenApi)]
#[openapi(
    info(description = "Pharia Kernel (Beta): The best place to run serverless AI applications."),
    paths(serve_docs, skills_beta, run_skill, message_stream_skill, skill_wit, skill_metadata, list_mcp_servers),
    modifiers(&SecurityAddon),
    components(schemas(ExecuteSkillArgs, Namespace)),
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
async fn serve_docs() -> Json<openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

#[derive(Deserialize, Serialize, ToSchema)]
struct ExecuteSkillArgs {
    /// The qualified name of the Skill to invoke. The qualified name consists of a namespace and
    /// a Skill name (e.g. "acme/summarize").
    ///
    skill: String,
    /// The expected input for the Skill in JSON format. Examples:
    /// * "input": "Hello"
    /// * "input": {"text": "some text to be summarized", "length": "short"}
    input: Value,
}

/// Metadata
///
/// Get the metadata (input schema, output schema, description) for a Skill.
#[utoipa::path(
    get,
    operation_id = "skill_metadata",
    path = "/v1/skills/{namespace}/{name}/metadata",
    security(("api_token" = [])),
    tag = "skills",
    responses(
        (status = 200, description = "Description, input schema, and output schema of the skill if specified",
            body=SkillMetadataV1Representation, example = json!({
                "description": "The summary of the text.",
                "input_schema": {
                    "properties": {
                        "topic": {
                            "description": "The topic of the haiku",
                            "examples": ["Banana", "Oat milk"],
                            "title": "Topic",
                            "type": "string",
                        }
                    },
                    "required": ["topic"],
                    "title": "Input",
                    "type": "object",
                },
                "output_schema": {
                    "properties": {"message": {"title": "Message", "type": "string"}},
                    "required": ["message"],
                    "title": "Output",
                    "type": "object",
                }
            })),
        (status = 400, description = "Failed to get skill metadata.", body=String, example = "Invalid skill input schema.")
    ),
)]
async fn skill_metadata<R>(
    State(SkillRuntimeState(skill_runtime_api)): State<SkillRuntimeState<R>>,
    _bearer: TypedHeader<Authorization<Bearer>>,
    Path((namespace, name)): Path<(Namespace, String)>,
) -> Result<Json<SkillMetadataV1Representation>, HttpError>
where
    R: SkillRuntimeApi,
{
    let tracing_context = TracingContext::current();
    let skill_path = SkillPath::new(namespace, name);
    let response = skill_runtime_api
        .skill_metadata(skill_path, tracing_context)
        .await?;
    Ok(Json(response.into()))
}

#[derive(ToSchema, Serialize, Debug, Clone)]
struct SkillMetadataV1Representation {
    description: Option<String>,
    version: Option<&'static str>,
    skill_type: &'static str,
    input_schema: Option<JsonSchema>,
    output_schema: Option<JsonSchema>,
}

impl From<AnySkillManifest> for SkillMetadataV1Representation {
    fn from(metadata: AnySkillManifest) -> Self {
        let signature = metadata.signature();
        SkillMetadataV1Representation {
            description: metadata.description().map(ToOwned::to_owned),
            version: metadata.version(),
            skill_type: metadata.skill_type_name(),
            input_schema: signature.map(Signature::input_schema).cloned(),
            output_schema: signature.and_then(Signature::output_schema).cloned(),
        }
    }
}

/// Run
///
/// Run a Skill in the Kernel from one of the available repositories.
#[utoipa::path(
    post,
    operation_id = "run_skill",
    path = "/v1/skills/{namespace}/{name}/run",
    request_body(content_type = "application/json", description = "The expected input for the skill in JSON format.", example = json!({"text": "some text to be summarized", "length": "short"})),
    security(("api_token" = [])),
    tag = "skills",
    responses(
        (status = 200, description = "The Skill was executed.", body=Value, example = json!({"summary": "The summary of the text."})),
        (status = 400, description = "The Skill invocation failed.", body=String, example = "Skill not found.")
    ),
)]
async fn run_skill<R>(
    State(SkillRuntimeState(skill_runtime_api)): State<SkillRuntimeState<R>>,
    bearer: TypedHeader<Authorization<Bearer>>,
    Path((namespace, name)): Path<(Namespace, String)>,
    Json(input): Json<Value>,
) -> Result<Json<Value>, HttpError>
where
    R: SkillRuntimeApi,
{
    let tracing_context = TracingContext::current();
    let skill_path = SkillPath::new(namespace, name);
    let response = skill_runtime_api
        .run_function(
            skill_path,
            input,
            bearer.token().to_owned(),
            tracing_context,
        )
        .await?;
    Ok(Json(response))
}

/// Stream
///
/// Stream from a Skill in the Kernel from one of the available repositories.
#[utoipa::path(
    post,
    operation_id = "message_stream_skill",
    path = "/v1/skills/{namespace}/{name}/message-stream",
    request_body(
        content_type = "application/json",
        description = "The expected input for the skill in JSON format.",
        example = json!({})
    ),
    security(("api_token" = [])),
    tag = "skills",
    responses(
        (status = 200,
            description = "A stream of substrings composing a message",
            body=Value,
            content(("text/event-stream", examples(
                ("Tell me a joke" = (
                    summary = "Namespace: `example`, Skill: `tell_me_a_joke`",
                    description = "The `tell_me_a_joke` Skill in the `example` namespace streams a joke.",
                    value = json!("\
                        event: message\n\
                        data: {\"type\":\"begin\"}\n\
                        \n\
                        event: message\n\
                        data: {\"type\":\"append\",\"text\":\"Here's one:\\n\\nWhat do you call a fake noodle?\\n\\nAn impasta!\\n\\nHope that made you laugh! Do you want to hear another one?\"}\n\
                        \n\
                        event: message\n\
                        data: {\"type\":\"end\",\"payload\":\"Stop\"}\n\
                    ")
                )),
                ("Hello" = (
                    summary = "Namespace: `example`, Skill: `hello`",
                    description = "The `hello` Skill in the `example` namespace streams each character of the text \"Hello\".",
                    value = json!("\
                        event: message\n\
                        data: {\"type\":\"begin\"}\n\
                        \n\
                        event: message\n\
                        data: {\"type\":\"append\",\"text\":\"H\"}\n\
                        \n\
                        event: message\n\
                        data: {\"type\":\"append\",\"text\":\"e\"}\n\
                        \n\
                        event: message\n\
                        data: {\"type\":\"append\",\"text\":\"l\"}\n\
                        \n\
                        event: message\n\
                        data: {\"type\":\"append\",\"text\":\"l\"}\n\
                        \n\
                        event: message\n\
                        data: {\"type\":\"append\",\"text\":\"o\"}\n\
                        \n\
                        event: message\n\
                        data: {\"type\":\"end\",\"payload\":null}\n\
                    ")
                )),
                ("Saboteur" = (
                    summary = "Namespace: `example`, Skill: `saboteur`",
                    description = "The `saboteur` Skill in the `example` namespace responds with an error.",
                    value = json!("\
                        event: error\n\
                        data: {\"message\":\"The skill you called responded with an error. Maybe you should check your input, if it seems to be correct you may want to contact the skill developer. Error reported by Skill:\\n\\nSkill is a saboteur\"}\n\
                    ")
                )),
                ("Non-existing Skill" = (
                    summary = "Namespace: `example`, Skill: `non-existing-skill`",
                    description = "The `non-existing-skill` Skill does not exist in the `example` namespace.",
                    value = json!("\
                        event: error\n\
                        data: {\"message\":\"Sorry, we could not find the skill you requested in its namespace. This can have three causes:\\n\n1. You sent the wrong skill name.\\n2. You sent the wrong namespace.\\n3. The skill is not configured in the namespace you requested. You may want to check the namespace configuration.\"}\n\
                    ")
                ))
            )))),
        ),
)]
async fn message_stream_skill<R>(
    State(SkillRuntimeState(skill_runtime_api)): State<SkillRuntimeState<R>>,
    bearer: TypedHeader<Authorization<Bearer>>,
    Path((namespace, name)): Path<(Namespace, String)>,
    Json(input): Json<Value>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>>
where
    R: SkillRuntimeApi,
{
    let path = SkillPath::new(namespace, name);
    let tracing_context = TracingContext::current();
    let mut stream_events = skill_runtime_api
        .run_message_stream(path, input, bearer.token().to_owned(), tracing_context)
        .await;

    // We need to use `try_stream!` instead of `stream!`, because `stream!` does not implement the
    // traits required to be converted into an http body for the response. Since we do wanna rely on
    // the implementation provided by axum for this, we use `try_stream!` with an infallible error
    // type. Please note, that we report the actual errors as "normal" events in the stream.
    let stream = try_stream! {
        while let Some(event) = stream_events.recv().await {
            // Convert stream events to Server Side Events
            yield event.into();
        }
    };

    Sse::new(stream)
}

impl From<SkillExecutionEvent> for Event {
    fn from(value: SkillExecutionEvent) -> Self {
        match value {
            SkillExecutionEvent::MessageBegin => Self::default()
                .event("message")
                .json_data(MessageEvent::Begin)
                .expect("`json_data` must only be called once."),
            SkillExecutionEvent::MessageEnd { payload } => Self::default()
                .event("message")
                .json_data(MessageEvent::End { payload })
                .expect("`json_data` must only be called once."),
            SkillExecutionEvent::MessageAppend { text } => Self::default()
                .event("message")
                .json_data(MessageEvent::Append { text })
                .expect("`json_data` must only be called once."),
            SkillExecutionEvent::Error(error) => Self::default()
                .event("error")
                .json_data(SseErrorEvent {
                    message: error.to_string(),
                })
                .expect("`json_data` must only be called once."),
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum MessageEvent {
    Begin,
    Append { text: String },
    End { payload: Value },
}

#[derive(Serialize)]
struct SseErrorEvent {
    message: String,
}

#[derive(Deserialize, ToSchema)]
struct SkillListParams {
    #[serde(rename = "type")]
    skill_type: Option<SkillDescriptionSchemaType>,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkillDescriptionSchemaType {
    Chat,
    Programmable,
}

impl From<SkillDescriptionSchemaType> for SkillDescriptionFilterType {
    fn from(value: SkillDescriptionSchemaType) -> Self {
        match value {
            SkillDescriptionSchemaType::Chat => SkillDescriptionFilterType::Chat,
            SkillDescriptionSchemaType::Programmable => SkillDescriptionFilterType::Programmable,
        }
    }
}

/// List
///
/// List of configured Skills (Beta).
#[utoipa::path(
    get,
    operation_id = "skills",
    path = "/v1/skills",
    tag = "skills",
    security(("api_token" = [])),
    params(("type" = Option<SkillDescriptionSchemaType>, Query, nullable, description = "The type of skills to list. Can be `chat` or `null`.")),
    responses(
        (status = 200, body=Vec<String>, example = json!(["acme/first_skill", "acme/second_skill"])),
    ),
)]
async fn skills_beta<S>(
    State(SkillStoreState(skill_store_api)): State<SkillStoreState<S>>,
    Query(SkillListParams { skill_type }): Query<SkillListParams>,
) -> Json<Vec<String>>
where
    S: SkillStoreApi,
{
    let response = skill_store_api.list(skill_type.map(Into::into)).await;
    let response = response.iter().map(ToString::to_string).collect();
    Json(response)
}

/// List
///
/// List of configured Skills.
#[utoipa::path(
    get,
    operation_id = "skills",
    path = "/v1/skills",
    tag = "skills",
    security(("api_token" = [])),
    responses(
        (status = 200, body=Vec<String>, example = json!(["acme/first_skill", "acme/second_skill"])),
    ),
)]
async fn skills<S>(
    State(SkillStoreState(skill_store_api)): State<SkillStoreState<S>>,
) -> Json<Vec<String>>
where
    S: SkillStoreApi,
{
    let response = skill_store_api.list(None).await;
    let response = response.iter().map(ToString::to_string).collect();
    Json(response)
}

/// cached_skills
///
/// List of all cached Skills. These are Skills that are already compiled
/// and are faster because they do not have to be transpiled to machine code.
/// When executing a Skill which is not loaded yet, it will be cached.
#[utoipa::path(
    get,
    operation_id = "cached_skills",
    path = "/cached_skills",
    tag = "skills",
    security(("api_token" = [])),
    responses(
        (status = 200, body=Vec<String>, example = json!(["acme/first_skill", "acme/second_skill"])),
    ),
)]
async fn cached_skills<S>(
    State(SkillStoreState(skill_store_api)): State<SkillStoreState<S>>,
) -> Json<Vec<String>>
where
    S: SkillStoreApi,
{
    let response = skill_store_api.list_cached().await;
    let response = response.iter().map(ToString::to_string).collect();
    Json(response)
}

/// drop_cached_skill
///
/// Remove a loaded Skill from the runtime. With a first invocation, Skills are loaded to
/// the runtime. This leads to faster execution on the second invocation. If a Skill is
/// updated in the repository, it needs to be removed from the cache so that the new version
/// becomes available in the Kernel.
#[utoipa::path(
    delete,
    operation_id = "drop_cached_skill",
    path = "/cached_skills/{namespace}/{name}",
    tag = "skills",
    security(("api_token" = [])),
    responses(
        (status = 200, body=String, example = json!("Skill removed from cache.")),
        (status = 200, body=String, example = json!("Skill was not present in cache.")),
    ),
)]
async fn drop_cached_skill<S>(
    State(SkillStoreState(skill_store_api)): State<SkillStoreState<S>>,
    Path((namespace, name)): Path<(Namespace, String)>,
) -> Json<String>
where
    S: SkillStoreApi,
{
    let skill_path = SkillPath::new(namespace, name);
    let skill_was_cached = skill_store_api.invalidate_cache(skill_path).await;
    let msg = if skill_was_cached {
        "Skill removed from cache".to_string()
    } else {
        "Skill was not present in cache".to_string()
    };
    Json(msg)
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

/// List of all mcp servers configured for this namespace
#[utoipa::path(
    get,
    operation_id = "list_mcp_servers",
    path = "v1/mcp_servers/{namespace}",
    tag = "namespace",
    security(("api_token" = [])),
    responses(
        (status = 200, body=Vec<String>, example = json!(["localhost:8080/my_tool"])),
    ),
)]
async fn list_mcp_servers<M>(
    Path(namespace): Path<Namespace>,
    State(McpServerStoreState(mcp_servers)): State<McpServerStoreState<M>>,
) -> Json<Vec<McpServerUrl>>
where
    M: McpServerStoreApi,
{
    let servers = mcp_servers.list(namespace).await;
    Json(servers)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crate::{
        authorization::tests::StubAuthorization,
        chunking::{Chunk, ChunkRequest},
        csi::{
            CsiError,
            tests::{CompletionStub, RawCsiDouble, RawCsiDummy},
        },
        feature_set::PRODUCTION_FEATURE_SET,
        inference,
        logging::tests::given_tracing_subscriber,
        skill_runtime::SkillRuntimeDouble,
        skill_store::{
            SkillStoreApiDouble,
            tests::{SkillStoreDummy, SkillStoreMsg},
        },
        skills::{AnySkillManifest, JsonSchema, SkillMetadataV0_3, SkillPath},
        tests::api_token,
        tool::tests::McpServerStoreDouble,
    };

    use super::*;

    use axum::{
        body::Body,
        http::{Method, Request, header},
    };

    use http_body_util::BodyExt;
    use mime::{APPLICATION_JSON, TEXT_EVENT_STREAM};
    use reqwest::header::CONTENT_TYPE;
    use serde_json::json;
    use tokio::sync::mpsc;
    use tower::util::ServiceExt;

    impl
        AppState<
            StubAuthorization,
            RawCsiDummy,
            SkillRuntimeDummy,
            SkillStoreDummy,
            McpServerStoreDummy,
        >
    {
        pub fn dummy() -> Self {
            Self::new(
                StubAuthorization::new(true),
                SkillStoreDummy,
                SkillRuntimeDummy,
                McpServerStoreDummy,
                RawCsiDummy,
            )
        }
    }

    impl<A, C, R, S, M> AppState<A, C, R, S, M>
    where
        A: AuthorizationApi + Clone + Sync + Send + 'static,
        C: RawCsi + Clone + Sync + Send + 'static,
        R: SkillRuntimeApi + Clone + Send + Sync + 'static,
        S: SkillStoreApi + Clone + Send + Sync + 'static,
        M: McpServerStoreApi + Clone + Send + Sync + 'static,
    {
        pub fn with_authorization_api<A2>(self, authorization_api: A2) -> AppState<A2, C, R, S, M>
        where
            A2: AuthorizationApi + Clone + Sync + Send + 'static,
        {
            AppState::new(
                authorization_api,
                self.skill_store_api,
                self.skill_runtime_api,
                self.mcp_servers,
                self.csi_drivers,
            )
        }

        pub fn with_skill_store_api<S2>(self, skill_store_api: S2) -> AppState<A, C, R, S2, M>
        where
            S2: SkillStoreApi + Clone + Send + Sync + 'static,
        {
            AppState::new(
                self.authorization_api,
                skill_store_api,
                self.skill_runtime_api,
                self.mcp_servers,
                self.csi_drivers,
            )
        }

        pub fn with_skill_runtime_api<R2>(self, skill_runtime_api: R2) -> AppState<A, C, R2, S, M>
        where
            R2: SkillRuntimeApi + Clone + Send + Sync + 'static,
        {
            AppState::new(
                self.authorization_api,
                self.skill_store_api,
                skill_runtime_api,
                self.mcp_servers,
                self.csi_drivers,
            )
        }

        pub fn with_csi_drivers<C2>(self, csi_drivers: C2) -> AppState<A, C2, R, S, M>
        where
            C2: RawCsi + Clone + Sync + Send + 'static,
        {
            AppState::new(
                self.authorization_api,
                self.skill_store_api,
                self.skill_runtime_api,
                self.mcp_servers,
                csi_drivers,
            )
        }
    }

    fn dummy_auth_value() -> header::HeaderValue {
        let api_token = "dummy auth token";
        let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
        auth_value.set_sensitive(true);
        auth_value
    }

    #[tokio::test]
    #[should_panic]
    async fn list_mcp_servers_by_namespace() {
        let app_state = AppState::dummy();
        let http = http(FeatureSet::Beta, app_state);

        // When
        let api_token = api_token();
        let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
        auth_value.set_sensitive(true);

        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/v1/mcp_servers/my-test-namespace")
                    .header(AUTHORIZATION, auth_value)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then
        // assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let answer = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(answer, "[\"localhost:8080/my_tool\"]");
    }

    #[tokio::test]
    async fn skill_metadata() {
        // Given
        let metadata = AnySkillManifest::V0_3(SkillMetadataV0_3 {
            description: Some("dummy description".to_owned()),
            signature: Signature::Function {
                input_schema: JsonSchema::dummy(),
                output_schema: JsonSchema::dummy(),
            },
        });
        let runtime = SkillRuntimeStub::with_metadata(metadata);
        let app_state = AppState::dummy().with_skill_runtime_api(runtime);

        // When
        let http = http(PRODUCTION_FEATURE_SET, app_state);
        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .header(AUTHORIZATION, dummy_auth_value())
                    .uri("/v1/skills/local/greet_skill/metadata")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let metadata = serde_json::from_slice::<Value>(&body).unwrap();
        let expected = json!({
            "description": "dummy description",
            "skill_type": "function",
            "input_schema": {"properties": {"topic": {"title": "Topic", "type": "string"}}, "required": ["topic"], "title": "Input", "type": "object"},
            "output_schema": {"properties": {"topic": {"title": "Topic", "type": "string"}}, "required": ["topic"], "title": "Input", "type": "object"},
            "version": "0.3",
        });
        assert_eq!(metadata, expected);
    }

    #[tokio::test]
    async fn http_csi_handle_returns_completion() {
        // Given a versioned csi request
        let prompt = "Say hello to Homer";
        let body = json!({
            "version": "0.2",
            "function": "complete",
            "model": "pharia-1-llm-7b-control",
            "prompt": prompt,
            "params": {
                "max_tokens": 1,
                "temperature": null,
                "top_k": null,
                "top_p": null,
                "stop": [],
            },
        });

        // When
        let csi = CompletionStub::new(|r| inference::Completion::from_text(r.prompt));
        let app_state = AppState::dummy().with_csi_drivers(csi);
        let http = http(PRODUCTION_FEATURE_SET, app_state);

        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, dummy_auth_value())
                    .uri("/csi")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_value["text"].as_str().unwrap(), prompt);
    }

    #[tokio::test]
    async fn http_csi_handle_returns_completion_stream() {
        // Given a versioned csi request and a stub csi that returns three events for completions
        #[derive(Clone)]
        struct RawCsiStub;

        impl RawCsiDouble for RawCsiStub {
            async fn completion_stream(
                &self,
                _auth: String,
                _tracing_context: TracingContext,
                _request: inference::CompletionRequest,
            ) -> mpsc::Receiver<Result<inference::CompletionEvent, inference::InferenceError>>
            {
                let (sender, receiver) = mpsc::channel(1);
                tokio::spawn(async move {
                    let append = inference::CompletionEvent::Append {
                        text: "Say hello to Homer".to_owned(),
                        logprobs: vec![],
                    };
                    let end = inference::CompletionEvent::End {
                        finish_reason: inference::FinishReason::Stop,
                    };
                    let usage = inference::CompletionEvent::Usage {
                        usage: inference::TokenUsage {
                            prompt: 0,
                            completion: 0,
                        },
                    };
                    sender.send(Ok(append)).await.unwrap();
                    sender.send(Ok(end)).await.unwrap();
                    sender.send(Ok(usage)).await.unwrap();
                });
                receiver
            }
        }

        let body = json!({
            "model": "pharia-1-llm-7b-control",
            "prompt": "Hi",
            "params": {
                "max_tokens": 1,
                "stop": [],
                "return_special_tokens": true,
                "logprobs": "no",
            },
        });

        // When
        let app_state = AppState::dummy().with_csi_drivers(RawCsiStub);
        let http = http(PRODUCTION_FEATURE_SET, app_state);

        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, dummy_auth_value())
                    .uri("/csi/v1/completion_stream")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then we get the expected events
        assert_eq!(resp.status(), StatusCode::OK);
        let content_type = resp.headers().get(CONTENT_TYPE).unwrap();
        assert_eq!(content_type, TEXT_EVENT_STREAM.as_ref());

        let body_text = resp.into_body().collect().await.unwrap().to_bytes();
        let expected_body = "event: append
data: {\"text\":\"Say hello to Homer\",\"logprobs\":[]}

event: end
data: {\"finish_reason\":\"stop\"}

event: usage
data: {\"usage\":{\"prompt\":0,\"completion\":0}}

";
        assert_eq!(body_text, expected_body);
    }

    #[tokio::test]
    async fn http_csi_v1_handle_returns_completion_with_echo() {
        // Given a versioned csi request
        let prompt = "Say hello to Homer";
        let body = json!([{
            "model": "pharia-1-llm-7b-control",
            "prompt": prompt,
            "params": {
                "echo": true,
                "max_tokens": 1,
                "stop": [],
                "return_special_tokens": true,
                "logprobs": "no",
            },
        }]);

        // When
        let csi = CompletionStub::new(|r| {
            // We expect echo to be true
            assert!(r.params.echo);
            inference::Completion::from_text(r.prompt)
        });
        let app_state = AppState::dummy().with_csi_drivers(csi);
        let http = http(PRODUCTION_FEATURE_SET, app_state);

        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, dummy_auth_value())
                    .uri("/csi/v1/complete")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn http_csi_v1_handle_returns_completion() {
        // Given a versioned csi request
        let prompt = "Say hello to Homer";
        let body = json!([{
            "model": "pharia-1-llm-7b-control",
            "prompt": prompt,
            "params": {
                "max_tokens": 1,
                "stop": [],
                "return_special_tokens": true,
                "logprobs": "no",
            },
        }]);

        // When
        let csi = CompletionStub::new(|r| {
            assert!(!r.params.echo);
            inference::Completion::from_text(r.prompt)
        });
        let app_state = AppState::dummy().with_csi_drivers(csi);
        let http = http(PRODUCTION_FEATURE_SET, app_state);

        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, dummy_auth_value())
                    .uri("/csi/v1/complete")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then we get separate events for each letter in "Hello"
        assert_eq!(resp.status(), StatusCode::OK);
        let content_type = resp.headers().get(CONTENT_TYPE).unwrap();
        assert_eq!(content_type, APPLICATION_JSON.as_ref());
    }

    #[tokio::test]
    async fn http_csi_handle_returns_stream() {
        // Given a versioned csi request and a stub csi that returns four events for completions
        #[derive(Clone)]
        struct RawCsiStub;

        impl RawCsiDouble for RawCsiStub {
            async fn chat_stream(
                &self,
                _auth: String,
                _tracing_context: TracingContext,
                _request: inference::ChatRequest,
            ) -> mpsc::Receiver<Result<inference::ChatEvent, inference::InferenceError>>
            {
                let (sender, receiver) = mpsc::channel(1);
                tokio::spawn(async move {
                    let message_begin = inference::ChatEvent::MessageBegin {
                        role: "assistant".to_owned(),
                    };
                    let message_append = inference::ChatEvent::MessageAppend {
                        content: "Say hello to Homer".to_owned(),
                        logprobs: vec![],
                    };
                    let message_end = inference::ChatEvent::MessageEnd {
                        finish_reason: inference::FinishReason::Stop,
                    };
                    let usage = inference::ChatEvent::Usage {
                        usage: inference::TokenUsage {
                            prompt: 0,
                            completion: 0,
                        },
                    };
                    sender.send(Ok(message_begin)).await.unwrap();
                    sender.send(Ok(message_append)).await.unwrap();
                    sender.send(Ok(message_end)).await.unwrap();
                    sender.send(Ok(usage)).await.unwrap();
                });
                receiver
            }
        }
        let body = json!({
            "model": "pharia-1-llm-7b-control",
            "messages": [
                {"role": "user", "content": "Hi"}
            ],
            "params": {
                "max_tokens": 1,
                "stop": [],
                "logprobs": "no",
            },
        });

        // When
        let app_state = AppState::dummy().with_csi_drivers(RawCsiStub);
        let http = http(PRODUCTION_FEATURE_SET, app_state);

        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, dummy_auth_value())
                    .uri("/csi/v1/chat_stream")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then we get the expected events
        assert_eq!(resp.status(), StatusCode::OK);
        let content_type = resp.headers().get(CONTENT_TYPE).unwrap();
        assert_eq!(content_type, TEXT_EVENT_STREAM.as_ref());

        let body_text = resp.into_body().collect().await.unwrap().to_bytes();
        let expected_body = "event: message_begin
data: {\"role\":\"assistant\"}

event: message_append
data: {\"content\":\"Say hello to Homer\",\"logprobs\":[]}

event: message_end
data: {\"finish_reason\":\"stop\"}

event: usage
data: {\"usage\":{\"prompt\":0,\"completion\":0}}

";
        assert_eq!(body_text, expected_body);
    }

    #[tokio::test]
    async fn http_csi_handle_returns_explain() {
        #[derive(Clone)]
        struct RawCsiStub;

        impl RawCsiDouble for RawCsiStub {
            async fn explain(
                &self,
                _auth: String,
                _tracing_context: TracingContext,
                _requests: Vec<inference::ExplanationRequest>,
            ) -> Result<Vec<inference::Explanation>, CsiError> {
                Ok(vec![inference::Explanation::new(vec![
                    inference::TextScore {
                        score: 0.0,
                        start: 0,
                        length: 2,
                    },
                ])])
            }
        }

        let body = json!([{
            "prompt": "prompt",
            "target": "target",
            "model": "model",
            "granularity": "auto"
        }]);

        let app_state = AppState::dummy().with_csi_drivers(RawCsiStub);
        let http = http(PRODUCTION_FEATURE_SET, app_state);

        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, dummy_auth_value())
                    .uri("/csi/v1/explain")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let content_type = resp.headers().get(CONTENT_TYPE).unwrap();
        assert_eq!(content_type, APPLICATION_JSON.as_ref());
    }

    #[tokio::test]
    async fn http_csi_handle_chat() {
        #[derive(Clone)]
        struct RawCsiStub;

        impl RawCsiDouble for RawCsiStub {
            async fn chat(
                &self,
                _auth: String,
                _tracing_context: TracingContext,
                _requests: Vec<inference::ChatRequest>,
            ) -> anyhow::Result<Vec<inference::ChatResponse>> {
                Ok(vec![inference::ChatResponse {
                    message: inference::Message {
                        role: "assistant".to_owned(),
                        content: "dummy-content".to_owned(),
                    },
                    finish_reason: inference::FinishReason::Stop,
                    logprobs: vec![],
                    usage: inference::TokenUsage {
                        prompt: 0,
                        completion: 0,
                    },
                }])
            }
        }
        // Given a versioned csi request
        let message = "Say hello to Homer";
        let body = json!([{
            "model": "pharia-1-llm-7b-control",
            "messages": [
                {"role": "user", "content": message}
            ],
            "params": {
                "max_tokens": 1,
                "stop": [],
                "logprobs": "no",
            },
        }]);

        // When
        let app_state = AppState::dummy().with_csi_drivers(RawCsiStub);
        let http = http(PRODUCTION_FEATURE_SET, app_state);

        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, dummy_auth_value())
                    .uri("/csi/v1/chat")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then
        assert_eq!(resp.status(), StatusCode::OK);
        let content_type = resp.headers().get(CONTENT_TYPE).unwrap();
        assert_eq!(content_type, APPLICATION_JSON.as_ref());
    }

    #[tokio::test]
    async fn http_csi_handle_chunk() {
        #[derive(Clone)]
        struct RawCsiStub;

        impl RawCsiDouble for RawCsiStub {
            async fn chunk(
                &self,
                _auth: String,
                _tracing_context: TracingContext,
                _requests: Vec<ChunkRequest>,
            ) -> anyhow::Result<Vec<Vec<Chunk>>> {
                Ok(vec![vec![Chunk {
                    text: "my_chunk".to_owned(),
                    byte_offset: 0,
                    character_offset: None,
                }]])
            }
        }
        let body = json!([{
            "text": "text",
            "params": {
                "model": "model",
                "max_tokens": 3,
                "overlap": 0,
            },
        }]);

        let app_state = AppState::dummy().with_csi_drivers(RawCsiStub);
        let http = http(PRODUCTION_FEATURE_SET, app_state);

        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, dummy_auth_value())
                    .uri("/csi/v1/chunk")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then we get separate events for each letter in "Hello"
        assert_eq!(resp.status(), StatusCode::OK);
        let content_type = resp.headers().get(CONTENT_TYPE).unwrap();
        assert_eq!(content_type, APPLICATION_JSON.as_ref());
    }

    #[tokio::test]
    async fn http_csi_handle_chunk_with_offsets() {
        #[derive(Clone)]
        struct RawCsiStub;

        impl RawCsiDouble for RawCsiStub {
            async fn chunk(
                &self,
                _auth: String,
                _tracing_context: TracingContext,
                _requests: Vec<ChunkRequest>,
            ) -> anyhow::Result<Vec<Vec<Chunk>>> {
                Ok(vec![vec![Chunk {
                    text: "my_chunk".to_owned(),
                    byte_offset: 0,
                    character_offset: Some(0),
                }]])
            }
        }

        let body = json!([{
            "text": "text",
            "params": {
                "model": "model",
                "max_tokens": 3,
                "overlap": 0,
            },
            "character_offsets": true
        }]);

        let app_state = AppState::dummy().with_csi_drivers(RawCsiStub);
        let http = http(PRODUCTION_FEATURE_SET, app_state);

        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, dummy_auth_value())
                    .uri("/csi/v1/chunk_with_offsets")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then we get separate events for each letter in "Hello"
        assert_eq!(resp.status(), StatusCode::OK);
        let content_type = resp.headers().get(CONTENT_TYPE).unwrap();
        assert_eq!(content_type, APPLICATION_JSON.as_ref());
    }

    #[tokio::test]
    async fn run_skill_with_bad_namespace() {
        // Given an invalid namespace
        let bad_namespace = "bad_namespace";
        let app_state = AppState::dummy();
        let http = http(PRODUCTION_FEATURE_SET, app_state);

        // When
        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, dummy_auth_value())
                    .uri(format!("/v1/skills/{bad_namespace}/greet_skill/run"))
                    .body(Body::from(serde_json::to_string(&json!("Homer")).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let error = String::from_utf8(body.to_vec()).unwrap();
        assert!(error.to_lowercase().contains("invalid namespace"));
    }

    #[tokio::test]
    async fn answer_of_succesfull_run_skill_function() {
        // Given
        let runtime = SkillRuntimeStub::with_function_ok(json!("Result from Skill"));
        let app_state = AppState::dummy().with_skill_runtime_api(runtime);
        let http = http(PRODUCTION_FEATURE_SET, app_state);

        // When
        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, dummy_auth_value())
                    .uri("/v1/skills/local/greet_skill/run")
                    .body(Body::from(serde_json::to_string(&json!("Homer")).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let answer = serde_json::from_slice::<String>(&body).unwrap();
        assert_eq!(answer, "Result from Skill");
    }

    #[tokio::test]
    async fn should_forward_function_input_to_skill_runtime() {
        // Given
        let runtime_spy = SkillRuntimeSpy::new();
        let app_state = AppState::dummy().with_skill_runtime_api(runtime_spy.clone());
        let http = http(PRODUCTION_FEATURE_SET, app_state);

        // When
        let _resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, dummy_auth_value())
                    .uri("/v1/skills/local/greet_skill/run")
                    .body(Body::from(serde_json::to_string(&json!("Homer")).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then
        assert_eq!(runtime_spy.api_token(), "dummy auth token");
        assert_eq!(runtime_spy.skill_path(), SkillPath::local("greet_skill"));
        assert_eq!(runtime_spy.input(), json!("Homer"));
    }

    #[tokio::test]
    async fn stream_endpoint_should_send_individual_message_deltas() {
        // Given
        let mut stream_events = Vec::new();
        stream_events.push(SkillExecutionEvent::MessageBegin);
        stream_events.extend("Hello".chars().map(|c| SkillExecutionEvent::MessageAppend {
            text: c.to_string(),
        }));
        stream_events.push(SkillExecutionEvent::MessageEnd {
            payload: json!(null),
        });
        let skill_executer_mock = SkillRuntimeStub::with_stream_events(stream_events);
        let app_state = AppState::dummy().with_skill_runtime_api(skill_executer_mock);
        let http = http(FeatureSet::Beta, app_state);

        // When asking for a message stream
        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, dummy_auth_value())
                    .uri("/v1/skills/local/hello/message-stream")
                    .body(Body::from("\"\""))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then we get separate events for each letter in "Hello"
        assert_eq!(resp.status(), StatusCode::OK);
        let content_type = resp.headers().get(CONTENT_TYPE).unwrap();
        assert_eq!(content_type, TEXT_EVENT_STREAM.as_ref());

        let body_text = resp.into_body().collect().await.unwrap().to_bytes();
        let expected_body = "\
            event: message\n\
            data: {\"type\":\"begin\"}\n\n\
            event: message\n\
            data: {\"type\":\"append\",\"text\":\"H\"}\n\n\
            event: message\n\
            data: {\"type\":\"append\",\"text\":\"e\"}\n\n\
            event: message\n\
            data: {\"type\":\"append\",\"text\":\"l\"}\n\n\
            event: message\n\
            data: {\"type\":\"append\",\"text\":\"l\"}\n\n\
            event: message\n\
            data: {\"type\":\"append\",\"text\":\"o\"}\n\n\
            event: message\n\
            data: {\"type\":\"end\",\"payload\":null}\n\n";
        assert_eq!(body_text, expected_body);
    }

    #[tokio::test]
    async fn stream_endpoint_for_saboteur_skill() {
        // Given
        let stream_events = vec![SkillExecutionEvent::Error(
            SkillExecutionError::RuntimeError("Skill is a saboteur".to_string()),
        )];
        let skill_runtime = SkillRuntimeStub::with_stream_events(stream_events);
        let app_state = AppState::dummy().with_skill_runtime_api(skill_runtime);
        let http = http(FeatureSet::Beta, app_state);

        // When asking for a message stream from a skill that does not exist
        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, dummy_auth_value())
                    .uri("/v1/skills/local/saboteur/message-stream")
                    .body(Body::from("\"\""))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then we get a OK response that contains an error
        assert_eq!(resp.status(), StatusCode::OK);
        let content_type = resp.headers().get(CONTENT_TYPE).unwrap();
        assert_eq!(content_type, TEXT_EVENT_STREAM.as_ref());

        let body_text = resp.into_body().collect().await.unwrap().to_bytes();
        let expected_body = "\
            event: error\n\
            data: {\"message\":\"The skill could not be executed to completion, something in our \
                runtime is currently unavailable or misconfigured. You should try again later, if \
                the situation persists you may want to contact the operators. Original error:\\n\\n\
                Skill is a saboteur\"}\n\n";
        assert_eq!(body_text, expected_body);
    }

    #[tokio::test]
    async fn api_token_missing_permission() {
        // Given
        let stub_authorization = StubAuthorization::new(false);
        let app_state = AppState::dummy().with_authorization_api(stub_authorization);

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
        let app_state = AppState::dummy();

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
    async fn list_cached_skills_for_user() {
        // Given a skill store with two cached skills
        #[derive(Clone)]
        struct SkillStoreStub;

        impl SkillStoreApiDouble for SkillStoreStub {
            async fn list_cached(&self) -> Vec<SkillPath> {
                vec![
                    SkillPath::new(Namespace::new("ns").unwrap(), "first"),
                    SkillPath::new(Namespace::new("ns").unwrap(), "second"),
                ]
            }
        }
        let app_state = AppState::dummy().with_skill_store_api(SkillStoreStub);
        let http = http(PRODUCTION_FEATURE_SET, app_state);

        // When
        let api_token = api_token();
        let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
        auth_value.set_sensitive(true);

        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/cached_skills")
                    .header(AUTHORIZATION, auth_value)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let answer = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(answer, "[\"ns/first\",\"ns/second\"]");
    }

    #[tokio::test]
    async fn drop_cached_skill() {
        // Given a provider which answers invalidate cache with `true`

        // We use this to spy on the path send to the skill executer. Better to use a channel,
        // rather than a mutex, but we do not have async closures yet.
        let skill_path = Arc::new(Mutex::new(None));
        let skill_path_clone = skill_path.clone();
        let (send, mut recv) = mpsc::channel(1);
        let namespace = Namespace::new("pharia-kernel-team").unwrap();
        tokio::spawn(async move {
            if let SkillStoreMsg::InvalidateCache { skill_path, send } = recv.recv().await.unwrap()
            {
                skill_path_clone.lock().unwrap().replace(skill_path);
                // `true` means it we actually deleted a skill
                send.send(true).unwrap();
            }
        });
        let app_state = AppState::dummy().with_skill_store_api(send);
        let http = http(PRODUCTION_FEATURE_SET, app_state);

        // When the skill is deleted
        let api_token = api_token();
        let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
        auth_value.set_sensitive(true);

        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri(format!("/cached_skills/{namespace}/haiku_skill"))
                    .header(AUTHORIZATION, auth_value)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        assert_eq!(
            skill_path.lock().unwrap().take().unwrap(),
            SkillPath::new(namespace, "haiku_skill")
        );
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let answer = String::from_utf8(body.to_vec()).unwrap();
        assert!(answer.contains("removed from cache"));
    }

    #[tokio::test]
    async fn drop_non_cached_skill() {
        // Given a runtime without cached skill

        // We use this to spy on the path send to the skill executer. Better to use a channel,
        // rather than a mutex, but we do not have async closures yet.
        let skill_path = Arc::new(Mutex::new(None));
        let skill_path_clone = skill_path.clone();
        let (send, mut recv) = mpsc::channel(1);
        let namespace = Namespace::new("pharia-kernel-team").unwrap();
        // We use this to spy on the path send to the skill executer. Better to use a channel,
        // rather than a mutex, but we do not have async closures yet.
        // Given a runtime with one installed skill
        tokio::spawn(async move {
            if let SkillStoreMsg::InvalidateCache { skill_path, send } = recv.recv().await.unwrap()
            {
                skill_path_clone.lock().unwrap().replace(skill_path);
                // `false` means the skill has not been there before
                send.send(false).unwrap();
            }
        });
        let app_state = AppState::dummy().with_skill_store_api(send);
        let http = http(PRODUCTION_FEATURE_SET, app_state);

        // When the skill is deleted
        let api_token = api_token();
        let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
        auth_value.set_sensitive(true);

        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri(format!("/cached_skills/{namespace}/haiku_skill"))
                    .header(AUTHORIZATION, auth_value)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        assert_eq!(
            skill_path.lock().unwrap().take().unwrap(),
            SkillPath::new(namespace, "haiku_skill")
        );
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let answer = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(answer, "\"Skill was not present in cache\"");
    }

    #[tokio::test]
    async fn health() {
        let app_state = AppState::dummy();
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
        let app_state = AppState::dummy();
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
    async fn list_skills() {
        // Given a skill store which can provide two skills "ns-one/one" and "ns-two/two"
        #[derive(Clone)]
        struct SkillStoreStub;

        impl SkillStoreApiDouble for SkillStoreStub {
            async fn list(
                &self,
                _skill_type: Option<SkillDescriptionFilterType>,
            ) -> Vec<SkillPath> {
                vec![
                    SkillPath::new(Namespace::new("ns-one").unwrap(), "one"),
                    SkillPath::new(Namespace::new("ns-two").unwrap(), "two"),
                ]
            }
        }
        let app_state = AppState::dummy().with_skill_store_api(SkillStoreStub);
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
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let actual = String::from_utf8(body.to_vec()).unwrap();
        let expected = "[\"ns-one/one\",\"ns-two/two\"]";
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn cannot_list_skills_without_permissions() {
        // Given we have a saboteur authorization
        let saboteur_authorization = StubAuthorization::new(false);
        let app_state = AppState::dummy().with_authorization_api(saboteur_authorization);
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
        let app_state = AppState::dummy().with_skill_runtime_api(skill_runtime);
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

    #[tokio::test]
    async fn not_existing_skill_is_400_error() {
        // Given a skill executer which always replies Skill does not exist
        let skill_runtime = SkillRuntimeSaboteur::new(|| SkillExecutionError::SkillNotConfigured);
        let auth_value = header::HeaderValue::from_str("Bearer DummyToken").unwrap();
        let app_state = AppState::dummy().with_skill_runtime_api(skill_runtime);

        // When executing a skill
        let http = http(PRODUCTION_FEATURE_SET, app_state);
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

        // Then answer is 400 skill does not exist
        assert_eq!(StatusCode::BAD_REQUEST, resp.status());
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(
            "Sorry, we could not find the skill you requested in its namespace. This can have \
            three causes:\n\n1. You sent the wrong skill name.\n2. You sent the wrong namespace.\
            \n3. The skill is not configured in the namespace you requested. You may want to \
            check the namespace configuration.",
            body_str
        );
    }

    #[derive(Debug, Clone)]
    struct SkillRuntimeDummy;

    impl SkillRuntimeDouble for SkillRuntimeDummy {}

    #[derive(Debug, Clone)]
    struct McpServerStoreDummy;
    impl McpServerStoreDouble for McpServerStoreDummy {}

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

    /// Stub Skill Runtime which emits predefined stream events
    #[derive(Debug, Clone)]
    struct SkillRuntimeStub {
        function_result: Value,
        metadata: AnySkillManifest,
        stream_events: Vec<SkillExecutionEvent>,
    }

    impl SkillRuntimeStub {
        pub fn with_function_ok(value: Value) -> Self {
            Self {
                function_result: value,
                stream_events: Vec::new(),
                metadata: AnySkillManifest::V0,
            }
        }

        pub fn with_stream_events(stream_events: Vec<SkillExecutionEvent>) -> Self {
            Self {
                function_result: Value::default(),
                stream_events,
                metadata: AnySkillManifest::V0,
            }
        }

        pub fn with_metadata(metadata: AnySkillManifest) -> Self {
            Self {
                function_result: Value::default(),
                stream_events: Vec::new(),
                metadata,
            }
        }
    }

    impl SkillRuntimeDouble for SkillRuntimeStub {
        async fn run_function(
            &self,
            _skill_path: SkillPath,
            _input: Value,
            _api_token: String,
            _tracing_context: TracingContext,
        ) -> Result<Value, SkillExecutionError> {
            Ok(self.function_result.clone())
        }

        async fn run_message_stream(
            &self,
            _skill_path: SkillPath,
            _input: Value,
            _api_token: String,
            _tracing_context: TracingContext,
        ) -> mpsc::Receiver<SkillExecutionEvent> {
            let (send, recv) = mpsc::channel(self.stream_events.len());
            for ce in &self.stream_events {
                send.send(ce.clone()).await.unwrap();
            }
            recv
        }

        async fn skill_metadata(
            &self,
            _skill_path: SkillPath,
            _tracing_context: TracingContext,
        ) -> Result<AnySkillManifest, SkillExecutionError> {
            Ok(self.metadata.clone())
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

        pub fn api_token(&self) -> String {
            self.inner.lock().unwrap().api_token.clone()
        }

        pub fn input(&self) -> Value {
            self.inner.lock().unwrap().input.clone()
        }

        pub fn skill_path(&self) -> SkillPath {
            self.inner.lock().unwrap().skill_path.clone()
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

        let app_state = AppState::dummy();
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
        let app_state = AppState::dummy().with_skill_runtime_api(skill_runtime.clone());
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
        let app_state = AppState::dummy().with_skill_runtime_api(skill_runtime.clone());
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
