use anyhow::Context;
use async_stream::try_stream;
use axum::{
    Json, Router,
    extract::{FromRef, MatchedPath, Path, Request, State},
    http::{StatusCode, header::AUTHORIZATION},
    middleware::{self, Next},
    response::{Html, IntoResponse, Sse, sse::Event},
    routing::{delete, get, post},
};
use axum_extra::{
    TypedHeader,
    headers::{Authorization, authorization::Bearer},
};
use futures::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{convert::Infallible, future::Future, iter::once, net::SocketAddr, time::Instant};
use tokio::{net::TcpListener, sync::mpsc, task::JoinHandle};
use tower::ServiceBuilder;
use tower_http::{
    compression::CompressionLayer,
    cors::CorsLayer,
    decompression::DecompressionLayer,
    sensitive_headers::SetSensitiveRequestHeadersLayer,
    trace::{DefaultOnRequest, DefaultOnResponse, TraceLayer},
};
use tracing::{Level, error, info, info_span};
use utoipa::{
    Modify, OpenApi, ToSchema,
    openapi::{
        self,
        security::{HttpAuthScheme, HttpBuilder, SecurityScheme},
    },
};
use utoipa_scalar::Scalar;

use crate::{
    authorization::{AuthorizationApi, authorization_middleware},
    csi::Csi,
    csi_shell::http_csi_handle,
    feature_set::FeatureSet,
    namespace_watcher::Namespace,
    skill_runtime::{ChatEvent, SkillExecutionError, SkillRuntimeApi},
    skill_store::{SkillStoreApi, SkillStoreMsg},
    skills::{AnySkillMetadata, JsonSchema, Signature, SkillPath},
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
        authorization_api: AuthorizationApi,
        skill_runtime_api: impl SkillRuntimeApi + Clone + Send + Sync + 'static,
        skill_store_api: mpsc::Sender<SkillStoreMsg>,
        csi_drivers: impl Csi + Clone + Send + Sync + 'static,
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
            csi_drivers,
        );
        let handle = tokio::spawn(async {
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
struct AppState<C, R, S>
where
    C: Clone,
    R: Clone,
    S: Clone,
{
    authorization_api: AuthorizationApi,
    skill_store_api: S,
    skill_runtime_api: R,
    csi_drivers: C,
}

impl<C, R, S> AppState<C, R, S>
where
    C: Csi + Clone + Sync + Send + 'static,
    R: SkillRuntimeApi + Clone,
    S: SkillStoreApi + Clone,
{
    pub fn new(
        authorization_api: AuthorizationApi,
        skill_store_api: S,
        skill_runtime_api: R,
        csi_drivers: C,
    ) -> Self {
        Self {
            authorization_api,
            skill_store_api,
            skill_runtime_api,
            csi_drivers,
        }
    }
}

impl<C, R, S> FromRef<AppState<C, R, S>> for AuthorizationApi
where
    C: Clone,
    R: Clone,
    S: Clone,
{
    fn from_ref(app_state: &AppState<C, R, S>) -> AuthorizationApi {
        app_state.authorization_api.clone()
    }
}

/// Wrapper used to extract [`Csi`] api from the [`AppState`] using a [`FromRef`] implementation.
pub struct CsiState<C>(pub C);

impl<C, R, S> FromRef<AppState<C, R, S>> for CsiState<C>
where
    C: Clone,
    R: Clone,
    S: Clone,
{
    fn from_ref(app_state: &AppState<C, R, S>) -> CsiState<C> {
        CsiState(app_state.csi_drivers.clone())
    }
}

/// Wrapper around Skill runtime Api for the shell. We use this strict alias to enable extracting a
/// reference from the [`AppState`] using a [`FromRef`] implementation.
struct SkillRuntimeState<R>(pub R);

impl<C, R, S> FromRef<AppState<C, R, S>> for SkillRuntimeState<R>
where
    C: Clone,
    R: Clone,
    S: Clone,
{
    fn from_ref(app_state: &AppState<C, R, S>) -> SkillRuntimeState<R> {
        SkillRuntimeState(app_state.skill_runtime_api.clone())
    }
}

/// Wrapper around Skill runtime Api for the shell. We use this strict alias to enable extracting a
/// reference from the [`AppState`] using a [`FromRef`] implementation.
struct SkillStoreState<S>(pub S);

impl<C, R, S> FromRef<AppState<C, R, S>> for SkillStoreState<S>
where
    C: Clone,
    R: Clone,
    S: Clone,
{
    fn from_ref(app_state: &AppState<C, R, S>) -> SkillStoreState<S> {
        SkillStoreState(app_state.skill_store_api.clone())
    }
}

fn http<C, R, S>(feature_set: FeatureSet, app_state: AppState<C, R, S>) -> Router
where
    C: Csi + Clone + Sync + Send + 'static,
    R: SkillRuntimeApi + Clone + Send + Sync + 'static,
    S: SkillStoreApi + Clone + Send + Sync + 'static,
{
    // Show documentation for unstable features only in beta systems.
    let api_doc = if feature_set == FeatureSet::Beta {
        ApiDocBeta::openapi()
    } else {
        ApiDoc::openapi()
    };
    Router::new()
        // Authenticated routes
        .route("/v1/skills", get(skills))
        .route(
            "/v1/skills/{namespace}/{name}/metadata",
            get(skill_metadata),
        )
        .route("/v1/skills/{namespace}/{name}/run", post(run_skill))
        .route("/v1/skills/{namespace}/{name}/chat", post(chat_skill))
        .route("/csi", post(http_csi_handle::<C>))
        // Keep for backwards compatibility
        .route("/skills", get(skills))
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
        // maintaining `healthcheck` route for backward compatibility
        .route("/healthcheck", get(async || "ok"))
        .route("/health", get(async || "ok"))
        .route_layer(middleware::from_fn(track_route_metrics))
        .layer(
            ServiceBuilder::new()
                // Mark the `Authorization` request header as sensitive so it doesn't show in logs
                .layer(SetSensitiveRequestHeadersLayer::new(once(AUTHORIZATION)))
                // High level logging of requests and responses
                .layer(
                    TraceLayer::new_for_http()
                        .make_span_with(|request: &Request<_>| {
                            // Log the matched route's path (with placeholders not filled in).
                            // Use request.uri() or OriginalUri if you want the real path.
                            let matched_path = request
                                .extensions()
                                .get::<MatchedPath>()
                                .map(MatchedPath::as_str);

                            info_span!(
                                "http_request",
                                method = ?request.method(),
                                matched_path,
                            )
                        })
                        .on_request(DefaultOnRequest::new().level(Level::INFO))
                        .on_response(DefaultOnResponse::new().level(Level::INFO)),
                )
                // Compress responses
                .layer(CompressionLayer::new())
                .layer(DecompressionLayer::new())
                .layer(CorsLayer::very_permissive()),
        )
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
            SkillExecutionError::MisconfiguredNamespace { .. }
            | SkillExecutionError::CsiUseFromMetadata
            | SkillExecutionError::RuntimeError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            SkillExecutionError::SkillLogicError(_) | SkillExecutionError::SkillNotConfigured => {
                StatusCode::BAD_REQUEST
            }
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
    paths(serve_docs, skills, run_skill, skill_wit),
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
    paths(serve_docs, skills, run_skill, chat_skill, skill_wit, skill_metadata),
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
    let skill_path = SkillPath::new(namespace, name);
    let response = skill_runtime_api.skill_metadata(skill_path).await?;
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

impl From<AnySkillMetadata> for SkillMetadataV1Representation {
    fn from(metadata: AnySkillMetadata) -> Self {
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
    let skill_path = SkillPath::new(namespace, name);
    let response = skill_runtime_api
        .run_function(skill_path, input, bearer.token().to_owned())
        .await?;
    Ok(Json(response))
}

/// Chat
///
/// Chat with a Skill in the Kernel from one of the available repositories.
#[utoipa::path(
    post,
    operation_id = "chat_skill",
    path = "/v1/skills/{namespace}/{name}/chat",
    request_body(
        content_type = "application/json",
        description = "The expected input for the skill in JSON format.",
        example = json!({})
    ),
    security(("api_token" = [])),
    tag = "skills",
    responses(
        (status = 200,
            description = "A stream of substrings composing a message in response to a chat history",
            body=Value,
            content(("text/event-stream", example = ""))),
        ),
)]
async fn chat_skill<R>(
    State(SkillRuntimeState(skill_runtime_api)): State<SkillRuntimeState<R>>,
    bearer: TypedHeader<Authorization<Bearer>>,
    Path((namespace, name)): Path<(Namespace, String)>,
    Json(input): Json<Value>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>>
where
    R: SkillRuntimeApi,
{
    let path = SkillPath::new(namespace, name);
    let mut chat_events = skill_runtime_api
        .run_chat(path, input, bearer.token().to_owned())
        .await;

    // We need to use `try_stream!` instead of `stream!`, because `stream!` does not implement the
    // traits required to be converted into an http body for the response. Since we do wanna rely on
    // the implementation provided by axum for this, we use `try_stream!` with an infallible error
    // type. Please note, that we report the actual errors as "normal" events in the stream.
    let stream = try_stream! {
        while let Some(event) = chat_events.recv().await {
            // Convert Chat events to Server Side Events
            yield event.into();
        }
    };

    Sse::new(stream)
}

impl From<ChatEvent> for Event {
    fn from(value: ChatEvent) -> Self {
        match value {
            ChatEvent::Append(text) => Self::default()
                .event("message_delta")
                .json_data(MessageDelta { text })
                .expect("`json_data` must only be called once."),
            ChatEvent::Error(message) => Self::default()
                .event("error")
                .json_data(SseErrorEvent { message })
                .expect("`json_data` must only be called once."),
        }
    }
}

#[derive(Serialize)]
struct MessageDelta {
    text: String,
}

#[derive(Serialize)]
struct SseErrorEvent {
    message: String,
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
    let response = skill_store_api.list().await;
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

#[cfg(test)]
mod tests {
    use std::{
        panic,
        sync::{Arc, Mutex},
    };

    use crate::{
        authorization::{self, tests::StubAuthorization},
        csi::tests::{CsiDummy, StubCsi},
        feature_set::PRODUCTION_FEATURE_SET,
        inference,
        skill_runtime::SkillExecutionError,
        skill_store::tests::{SkillStoreDummy, SkillStoreMsg, SkillStoreStub},
        skills::{AnySkillMetadata, JsonSchema, SkillMetadataV0_3, SkillPath},
        tests::api_token,
    };

    use super::*;

    use async_trait::async_trait;
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

    impl AppState<CsiDummy, SkillRuntimeDummy, SkillStoreDummy> {
        pub fn dummy() -> Self {
            let dummy_authorization = StubAuthorization::new(|msg| {
                match msg {
                    authorization::AuthorizationMsg::Auth { api_token: _, send } => {
                        drop(send.send(Ok(true)));
                    }
                };
            });
            Self::new(
                dummy_authorization.api(),
                SkillStoreDummy,
                SkillRuntimeDummy,
                CsiDummy,
            )
        }
    }

    impl<C, R, S> AppState<C, R, S>
    where
        C: Csi + Clone + Sync + Send + 'static,
        R: SkillRuntimeApi + Clone + Send + Sync + 'static,
        S: SkillStoreApi + Clone + Send + Sync + 'static,
    {
        pub fn with_authorization_api(mut self, authorization_api: AuthorizationApi) -> Self {
            self.authorization_api = authorization_api;
            self
        }

        pub fn with_skill_store_api<S2>(self, skill_store_api: S2) -> AppState<C, R, S2>
        where
            S2: SkillStoreApi + Clone + Send + Sync + 'static,
        {
            AppState::new(
                self.authorization_api,
                skill_store_api,
                self.skill_runtime_api,
                self.csi_drivers,
            )
        }

        pub fn with_skill_runtime_api<R2>(self, skill_runtime_api: R2) -> AppState<C, R2, S>
        where
            R2: SkillRuntimeApi + Clone + Send + Sync + 'static,
        {
            AppState::new(
                self.authorization_api,
                self.skill_store_api,
                skill_runtime_api,
                self.csi_drivers,
            )
        }

        pub fn with_csi_drivers<C2>(self, csi_drivers: C2) -> AppState<C2, R, S>
        where
            C2: Csi + Clone + Sync + Send + 'static,
        {
            AppState::new(
                self.authorization_api,
                self.skill_store_api,
                self.skill_runtime_api,
                csi_drivers,
            )
        }
    }

    #[tokio::test]
    async fn skill_metadata() {
        // Given
        let metadata = AnySkillMetadata::V0_3(SkillMetadataV0_3 {
            description: Some("dummy description".to_owned()),
            signature: Signature::Function {
                input_schema: JsonSchema::dummy(),
                output_schema: JsonSchema::dummy(),
            },
        });
        let runtime = SkillRuntimeStub::with_metadata(metadata);
        let app_state = AppState::dummy().with_skill_runtime_api(runtime);
        let api_token = "dummy auth token";

        // When
        let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
        auth_value.set_sensitive(true);

        let http = http(PRODUCTION_FEATURE_SET, app_state);
        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .header(header::AUTHORIZATION, auth_value)
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
        let api_token = "dummy auth token";
        let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
        auth_value.set_sensitive(true);
        let csi = StubCsi::with_completion(|r| inference::Completion::from_text(r.prompt));
        let app_state = AppState::dummy().with_csi_drivers(csi);
        let http = http(PRODUCTION_FEATURE_SET, app_state);

        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                    .header(header::AUTHORIZATION, auth_value)
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
    async fn run_skill_with_bad_namespace() {
        // Given an invalid namespace
        let bad_namespace = "bad_namespace";
        let app_state = AppState::dummy();
        let http = http(PRODUCTION_FEATURE_SET, app_state);

        // When
        let api_token = "dummy auth token";
        let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
        auth_value.set_sensitive(true);

        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, auth_value)
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
        let api_token = "dummy auth token";
        let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
        auth_value.set_sensitive(true);

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
        let api_token = "dummy auth token";
        let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
        auth_value.set_sensitive(true);

        let _resp = http
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
        assert_eq!(runtime_spy.api_token(), "dummy auth token");
        assert_eq!(runtime_spy.skill_path(), SkillPath::local("greet_skill"));
        assert_eq!(runtime_spy.input(), json!("Homer"));
    }

    #[tokio::test]
    async fn chat_endpoint_should_send_individual_message_deltas() {
        // Given
        let chat_events = "Hello"
            .chars()
            .map(|c| ChatEvent::Append(c.to_string()))
            .collect();
        let skill_executer_mock = SkillRuntimeStub::with_chat_events(chat_events);
        let app_state = AppState::dummy().with_skill_runtime_api(skill_executer_mock);
        let http = http(FeatureSet::Beta, app_state);

        // When asking for a chat message
        let api_token = "dummy auth token";
        let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
        auth_value.set_sensitive(true);

        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, auth_value)
                    .uri("/v1/skills/local/hello/chat")
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
            event: message_delta\n\
            data: {\"text\":\"H\"}\n\n\
            event: message_delta\n\
            data: {\"text\":\"e\"}\n\n\
            event: message_delta\n\
            data: {\"text\":\"l\"}\n\n\
            event: message_delta\n\
            data: {\"text\":\"l\"}\n\n\
            event: message_delta\n\
            data: {\"text\":\"o\"}\n\n";
        assert_eq!(body_text, expected_body);
    }

    #[tokio::test]
    async fn chat_endpoint_for_saboteur_skill() {
        // Given
        let chat_events = vec![ChatEvent::Error("Skill is a saboteur".to_string())];
        let skill_runtime = SkillRuntimeStub::with_chat_events(chat_events);
        let app_state = AppState::dummy().with_skill_runtime_api(skill_runtime);
        let http = http(FeatureSet::Beta, app_state);

        // When asking for a chat message from a skill that does not exist
        let api_token = "dummy auth token";
        let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
        auth_value.set_sensitive(true);

        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, auth_value)
                    .uri("/v1/skills/local/saboteur/chat")
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
            data: {\"message\":\"Skill is a saboteur\"}\
            \n\n";
        assert_eq!(body_text, expected_body);
    }

    #[tokio::test]
    async fn api_token_missing_permission() {
        // Given
        let stub_authorization = StubAuthorization::new(|msg| {
            match msg {
                authorization::AuthorizationMsg::Auth { api_token: _, send } => {
                    drop(send.send(Ok(false)));
                }
            };
        });
        let app_state = AppState::dummy().with_authorization_api(stub_authorization.api());

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
        // Given
        let mut skill_store = SkillStoreStub::new();
        skill_store.with_list_cached_response(vec![
            SkillPath::new(Namespace::new("ns").unwrap(), "first"),
            SkillPath::new(Namespace::new("ns").unwrap(), "second"),
        ]);
        let app_state = AppState::dummy().with_skill_store_api(skill_store);
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
                    .header(header::AUTHORIZATION, auth_value)
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
    async fn healthcheck() {
        let app_state = AppState::dummy();
        let http = http(PRODUCTION_FEATURE_SET, app_state);
        let resp = http
            .oneshot(
                Request::builder()
                    .uri("/healthcheck")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"ok");
    }

    #[tokio::test]
    async fn health() {
        let saboteur_authorization = StubAuthorization::new(|msg| {
            match msg {
                authorization::AuthorizationMsg::Auth { api_token: _, send } => {
                    drop(send.send(Ok(false)));
                }
            };
        });
        let app_state = AppState::dummy().with_authorization_api(saboteur_authorization.api());
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
        // Given we can provide two skills "ns-one/one" and "ns-two/two"
        let mut skill_store = SkillStoreStub::new();
        skill_store.with_list_response(vec![
            SkillPath::new(Namespace::new("ns-one").unwrap(), "one"),
            SkillPath::new(Namespace::new("ns-two").unwrap(), "two"),
        ]);
        let app_state = AppState::dummy().with_skill_store_api(skill_store);
        let http = http(PRODUCTION_FEATURE_SET, app_state);

        // When
        let api_token = api_token();
        let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
        auth_value.set_sensitive(true);

        let resp = http
            .oneshot(
                Request::builder()
                    .uri("/skills")
                    .header(header::AUTHORIZATION, auth_value)
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
        let saboteur_authorization = StubAuthorization::new(|msg| {
            match msg {
                authorization::AuthorizationMsg::Auth { api_token: _, send } => {
                    drop(send.send(Ok(false)));
                }
            };
        });
        let app_state = AppState::dummy().with_authorization_api(saboteur_authorization.api());
        let http = http(PRODUCTION_FEATURE_SET, app_state);

        // When
        let api_token = api_token();
        let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
        auth_value.set_sensitive(true);

        let resp = http
            .oneshot(
                Request::builder()
                    .uri("/skills")
                    .header(header::AUTHORIZATION, auth_value)
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
            "Sorry, We could not find the skill you requested in its namespace. This can have \
            three causes:\n\n1. You send the wrong skill name.\n2. You send the wrong namespace.\
            \n3. The skill is not configured in the namespace you requested. You may want to \
            check the namespace configuration.",
            body_str
        );
    }

    #[derive(Debug, Clone)]
    struct SkillRuntimeDummy;

    #[async_trait]
    impl SkillRuntimeApi for SkillRuntimeDummy {
        async fn run_function(
            &self,
            _skill_path: SkillPath,
            _input: Value,
            _api_token: String,
        ) -> Result<Value, SkillExecutionError> {
            panic!("Skill runtime dummy called")
        }

        async fn run_chat(
            &self,
            _skill_path: SkillPath,
            _input: Value,
            _api_token: String,
        ) -> mpsc::Receiver<ChatEvent> {
            panic!("Skill runtime dummy called")
        }

        async fn skill_metadata(
            &self,
            _skill_path: SkillPath,
        ) -> Result<AnySkillMetadata, SkillExecutionError> {
            panic!("Skill runtime dummy called")
        }
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

    #[async_trait]
    impl SkillRuntimeApi for SkillRuntimeSaboteur {
        async fn run_function(
            &self,
            _skill_path: SkillPath,
            _input: Value,
            _api_token: String,
        ) -> Result<Value, SkillExecutionError> {
            Err((*self.make_error)())
        }

        async fn run_chat(
            &self,
            _skill_path: SkillPath,
            _input: Value,
            _api_token: String,
        ) -> mpsc::Receiver<ChatEvent> {
            panic!(
                "Use the `SkillRuntimeStub`, to simulate errors during chat instead of the \
                `SkillRuntimeSaboteur`."
            )
        }

        async fn skill_metadata(
            &self,
            _skill_path: SkillPath,
        ) -> Result<AnySkillMetadata, SkillExecutionError> {
            Err((*self.make_error)())
        }
    }

    /// Stub Skill Runtime which emits predifined chat events
    #[derive(Debug, Clone)]
    struct SkillRuntimeStub {
        function_result: Value,
        metadata: AnySkillMetadata,
        chat_events: Vec<ChatEvent>,
    }

    impl SkillRuntimeStub {
        pub fn with_function_ok(value: Value) -> Self {
            Self {
                function_result: value,
                chat_events: Vec::new(),
                metadata: AnySkillMetadata::V0,
            }
        }

        pub fn with_chat_events(chat_events: Vec<ChatEvent>) -> Self {
            Self {
                function_result: Value::default(),
                chat_events,
                metadata: AnySkillMetadata::V0,
            }
        }

        pub fn with_metadata(metadata: AnySkillMetadata) -> Self {
            Self {
                function_result: Value::default(),
                chat_events: Vec::new(),
                metadata,
            }
        }
    }

    #[async_trait]
    impl SkillRuntimeApi for SkillRuntimeStub {
        async fn run_function(
            &self,
            _skill_path: SkillPath,
            _input: Value,
            _api_token: String,
        ) -> Result<Value, SkillExecutionError> {
            Ok(self.function_result.clone())
        }

        async fn run_chat(
            &self,
            _skill_path: SkillPath,
            _input: Value,
            _api_token: String,
        ) -> mpsc::Receiver<ChatEvent> {
            let (send, recv) = mpsc::channel(self.chat_events.len());
            for ce in &self.chat_events {
                send.send(ce.clone()).await.unwrap();
            }
            recv
        }

        async fn skill_metadata(
            &self,
            _skill_path: SkillPath,
        ) -> Result<AnySkillMetadata, SkillExecutionError> {
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
    }

    impl SkillRuntimeSpy {
        pub fn new() -> Self {
            Self {
                inner: Arc::new(Mutex::new(SkillRuntimeSpyInner {
                    api_token: String::new(),
                    input: Value::default(),
                    skill_path: SkillPath::local("SKILL HAS NOT BEEN SEND"),
                })),
            }
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

    #[async_trait]
    impl SkillRuntimeApi for SkillRuntimeSpy {
        async fn run_function(
            &self,
            skill_path: SkillPath,
            input: Value,
            api_token: String,
        ) -> Result<Value, SkillExecutionError> {
            let mut inner = self.inner.lock().unwrap();
            inner.api_token = api_token;
            inner.input = input;
            inner.skill_path = skill_path;
            Ok(Value::default())
        }

        async fn run_chat(
            &self,
            skill_path: SkillPath,
            input: Value,
            api_token: String,
        ) -> mpsc::Receiver<ChatEvent> {
            let mut inner = self.inner.lock().unwrap();
            inner.api_token = api_token;
            inner.input = input;
            inner.skill_path = skill_path;
            let (_send, recv) = mpsc::channel(1);
            recv
        }

        async fn skill_metadata(
            &self,
            _skill_path: SkillPath,
        ) -> Result<AnySkillMetadata, SkillExecutionError> {
            unimplemented!("Not needed in any test for now")
        }
    }
}
