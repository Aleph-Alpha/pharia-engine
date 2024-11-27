use std::{future::Future, iter::once, net::SocketAddr, time::Instant};

use anyhow::Context;
use axum::{
    extract::{FromRef, MatchedPath, Path, Request, State},
    http::{header::AUTHORIZATION, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse},
    routing::{delete, get, post},
    Json, Router,
};
use axum_extra::{
    headers::{authorization::Bearer, Authorization},
    TypedHeader,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use strum::IntoStaticStr;
use tokio::{net::TcpListener, task::JoinHandle};
use tower::ServiceBuilder;
use tower_http::{
    compression::CompressionLayer,
    cors::CorsLayer,
    decompression::DecompressionLayer,
    sensitive_headers::SetSensitiveRequestHeadersLayer,
    services::{ServeDir, ServeFile},
    trace::{DefaultOnRequest, DefaultOnResponse, TraceLayer},
};
use tracing::{error, info, info_span, Level};
use utoipa::{
    openapi::{
        self,
        security::{HttpAuthScheme, HttpBuilder, SecurityScheme},
    },
    Modify, OpenApi, ToSchema,
};
use utoipa_scalar::{Scalar, Servable};

use crate::{
    authorization::{authorization_middleware, AuthorizationApi},
    csi::Csi,
    csi_shell::http_csi_handle,
    skill_store::SkillStoreApi,
    skills::{ExecuteSkillError, SkillExecutorApi, SkillPath},
};

pub struct Shell {
    handle: JoinHandle<()>,
}

impl Shell {
    /// Start a shell listening to incoming requests at the given address. Successful construction
    /// implies that the listener is bound to the endpoint.
    pub async fn new(
        addr: impl Into<SocketAddr>,
        authorization_api: AuthorizationApi,
        skill_executor_api: SkillExecutorApi,
        skill_store_api: SkillStoreApi,
        csi_drivers: impl Csi + Clone + Send + Sync + 'static,
        shutdown_signal: impl Future<Output = ()> + Send + 'static,
    ) -> Result<Self, anyhow::Error> {
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
            skill_executor_api,
            csi_drivers,
        );
        let handle = tokio::spawn(async {
            let res = axum::serve(listener, http(app_state))
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
pub struct AppState<C>
where
    C: Clone,
{
    authorization_api: AuthorizationApi,
    skill_store_api: SkillStoreApi,
    skill_executor_api: SkillExecutorApi,
    pub csi_drivers: C,
}

impl<C> AppState<C>
where
    C: Csi + Clone + Sync + Send + 'static,
{
    pub fn new(
        authorization_api: AuthorizationApi,
        skill_store_api: SkillStoreApi,
        skill_executor_api: SkillExecutorApi,
        csi_drivers: C,
    ) -> Self {
        Self {
            authorization_api,
            skill_store_api,
            skill_executor_api,
            csi_drivers,
        }
    }
}

impl<C> FromRef<AppState<C>> for AuthorizationApi
where
    C: Clone,
{
    fn from_ref(app_state: &AppState<C>) -> AuthorizationApi {
        app_state.authorization_api.clone()
    }
}

impl<C> FromRef<AppState<C>> for SkillStoreApi
where
    C: Clone,
{
    fn from_ref(app_state: &AppState<C>) -> SkillStoreApi {
        app_state.skill_store_api.clone()
    }
}

impl<C> FromRef<AppState<C>> for SkillExecutorApi
where
    C: Clone,
{
    fn from_ref(app_state: &AppState<C>) -> SkillExecutorApi {
        app_state.skill_executor_api.clone()
    }
}

pub fn http<C>(app_state: AppState<C>) -> Router
where
    C: Csi + Clone + Sync + Send + 'static,
{
    let serve_dir =
        ServeDir::new("./doc/book/html").not_found_service(ServeFile::new("docs/index.html"));

    Router::new()
        // Authenticated routes
        .route("/csi", post(http_csi_handle::<C>))
        .route("/skills", get(skills))
        .route("/execute_skill", post(execute_skill))
        .route("/cached_skills", get(cached_skills))
        .route("/cached_skills/:name", delete(drop_cached_skill))
        .route_layer(middleware::from_fn_with_state(
            app_state.clone(),
            authorization_middleware,
        ))
        .with_state(app_state)
        // Unauthenticated routes
        .route("/skill.wit", get(skill_wit()))
        .nest_service("/docs", serve_dir.clone())
        .merge(Scalar::with_url("/api-docs", ApiDoc::openapi()))
        .route("/openapi.json", get(serve_docs))
        .route("/healthcheck", get(|| async { "ok" }))
        .route("/", get(index))
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

#[derive(IntoStaticStr, strum::Display)]
#[strum(serialize_all = "snake_case")]
pub enum ShellMetrics {
    HttpRequestsTotal,
    HttpRequestsDurationSeconds,
}

impl From<ShellMetrics> for metrics::KeyName {
    fn from(value: ShellMetrics) -> Self {
        Self::from_const_str(value.into())
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
    paths(serve_docs, skills, cached_skills, execute_skill, drop_cached_skill, skill_wit),
    modifiers(&SecurityAddon),
    components(schemas(ExecuteSkillArgs)),
    tags(
        (name = "skills"),
        (name = "docs"),
    )
)]
struct ApiDoc;

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
    /// The qualified name of the skill to invoke. The qualified name consists of a namespace and
    /// a skill name (e.g. "acme/summarize").
    /// If the namespace is omitted, the default 'pharia-kernel-team' namespace is used.
    ///
    skill: String,
    /// The expected input for the skill in JSON format. Examples:
    /// * "input": "Hello"
    /// * "input": {"text": "some text to be summarized", "length": "short"}
    input: Value,
}

/// execute_skill
///
/// Execute a skill in the kernel from one of the available repositories.
#[utoipa::path(
    post,
    operation_id = "execute_skill",
    path = "/execute_skill",
    request_body = ExecuteSkillArgs,
    security(("api_token" = [])),
    tag = "skills",
    responses(
        (status = 200, description = "The Skill was executed.", body=String, example = json!("Skill output")),
        (status = 400, description = "The Skill invocation failed.", body=String, example = json!("Skill not found."))
    ),
)]
async fn execute_skill(
    State(skill_executor_api): State<SkillExecutorApi>,
    bearer: TypedHeader<Authorization<Bearer>>,
    Json(args): Json<ExecuteSkillArgs>,
) -> (StatusCode, Json<Value>) {
    if args.skill.trim().is_empty() {
        return (
            VALIDATION_ERROR_STATUS_CODE,
            Json(json!("Empty skill names are not allowed.")),
        );
    }

    let skill_path = SkillPath::from_str(&args.skill);
    let result = skill_executor_api
        .execute_skill(skill_path, args.input, bearer.token().to_owned())
        .await;
    match result {
        Ok(response) => (StatusCode::OK, Json(json!(response))),
        Err(ExecuteSkillError::SkillDoesNotExist) => (
            StatusCode::BAD_REQUEST,
            Json(json!(ExecuteSkillError::SkillDoesNotExist.to_string())),
        ),
        Err(ExecuteSkillError::Other(err)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!(err.to_string())),
        ),
    }
}

/// skills
///
/// List of all configured skills.
#[utoipa::path(
    get,
    operation_id = "skills",
    path = "/skills",
    tag = "skills",
    responses(
        (status = 200, body=Vec<String>, example = json!(["acme/first_skill", "acme/second_skill"])),
    ),
)]
async fn skills(State(skill_store_api): State<SkillStoreApi>) -> (StatusCode, Json<Vec<String>>) {
    let response = skill_store_api.list().await;
    let response = response.iter().map(ToString::to_string).collect();
    (StatusCode::OK, Json(response))
}

/// cached_skills
///
/// List of all cached skills. These are skills that are already compiled
/// and are faster because they do not have to be transpiled to machine code.
/// When executing a skill which is not loaded yet, it will be cached.
#[utoipa::path(
    get,
    operation_id = "cached_skills",
    path = "/cached_skills",
    tag = "skills",
    responses(
        (status = 200, body=Vec<String>, example = json!(["acme/first_skill", "acme/second_skill"])),
    ),
)]
async fn cached_skills(
    State(skill_store_api): State<SkillStoreApi>,
) -> (StatusCode, Json<Vec<String>>) {
    let response = skill_store_api.list_cached().await;
    let response = response.iter().map(ToString::to_string).collect();
    (StatusCode::OK, Json(response))
}

/// drop_cached_skill
///
/// Remove a loaded skill from the runtime. With a first invocation, skills are loaded to
/// the runtime. This leads to faster execution on the second invocation. If a skill is
/// updated in the repository, it needs to be removed from the cache so that the new version
/// becomes available in the kernel.
#[utoipa::path(
    delete,
    operation_id = "drop_cached_skill",
    path = "/cached_skills/{namespace}%2F{name}",
    tag = "skills",
    responses(
        (status = 200, body=String, example = json!("Skill removed from cache.")),
        (status = 200, body=String, example = json!("Skill was not present in cache.")),
    ),
)]
async fn drop_cached_skill(
    State(skill_store_api): State<SkillStoreApi>,
    Path(name): Path<String>,
) -> (StatusCode, Json<String>) {
    let skill_path = SkillPath::from_str(&name);
    let skill_was_cached = skill_store_api.invalidate_cache(skill_path).await;
    let msg = if skill_was_cached {
        "Skill removed from cache".to_string()
    } else {
        "Skill was not present in cache".to_string()
    };
    (StatusCode::OK, Json(msg))
}

/// WIT (Webassembly Interface Types) of Skills
///
/// Skills are web assembly components build against a wit world. This route returns this wit world.
/// This allows you to build your own skill in many languages. We hope to provide higher level
/// tooling for selected languages in the future though.
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
    include_str!("../wit/skill@0.2/skill.wit")
}

/// We use `BAD_REQUEST` (400) for validation error as it is more commonly used.
/// `UNPROCESSABLE_ENTITY` (422) is an alternative, but it may surprise users as it is less commonly
/// known
const VALIDATION_ERROR_STATUS_CODE: StatusCode = StatusCode::BAD_REQUEST;

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crate::{
        authorization::{self, tests::StubAuthorization},
        csi::tests::{DummyCsi, StubCsi},
        csi_shell::{V0_2CsiRequest, VersionedCsiRequest},
        inference::{self, Completion, CompletionParams, CompletionRequest},
        skill_store::tests::{dummy_skill_store_api, SkillStoreMessage},
        skills::{tests::SkillExecutorMsg, ExecuteSkillError, SkillPath},
        tests::api_token,
    };

    use super::*;

    use anyhow::anyhow;
    use axum::{
        body::Body,
        http::{self, header, Request},
    };
    use http_body_util::BodyExt;
    use serde_json::json;
    use tokio::{sync::mpsc, task::JoinHandle};
    use tower::util::ServiceExt;

    impl AppState<DummyCsi> {
        pub fn dummy() -> Self {
            let dummy_authorization = StubAuthorization::new(|msg| {
                match msg {
                    authorization::AuthorizationMsg::Auth { api_token: _, send } => {
                        drop(send.send(Ok(true)));
                    }
                };
            });
            let skill_executor = StubSkillExecuter::new(|_| {});
            Self::new(
                dummy_authorization.api(),
                dummy_skill_store_api(),
                skill_executor.api(),
                DummyCsi,
            )
        }
    }

    impl<C> AppState<C>
    where
        C: Csi + Clone + Sync + Send + 'static,
    {
        pub fn with_authorization_api(mut self, authorization_api: AuthorizationApi) -> Self {
            self.authorization_api = authorization_api;
            self
        }

        pub fn with_skill_store_api(mut self, skill_store_api: SkillStoreApi) -> Self {
            self.skill_store_api = skill_store_api;
            self
        }

        pub fn with_skill_executor_api(mut self, skill_executor_api: SkillExecutorApi) -> Self {
            self.skill_executor_api = skill_executor_api;
            self
        }

        pub fn with_csi_drivers<D>(self, csi_drivers: D) -> AppState<D>
        where
            D: Csi + Clone + Sync + Send + 'static,
        {
            AppState::new(
                self.authorization_api,
                self.skill_store_api,
                self.skill_executor_api,
                csi_drivers,
            )
        }
    }

    #[tokio::test]
    async fn http_csi_handle_returns_completion() {
        // Given a versioned csi request
        let prompt = "Say hello to Homer";
        let completion_request = CompletionRequest {
            model: "llama-3.1-8b-instruct".to_owned(),
            prompt: prompt.to_owned(),
            params: CompletionParams {
                max_tokens: Some(128),
                temperature: None,
                top_k: None,
                top_p: None,
                stop: vec![],
            },
        };
        let request = VersionedCsiRequest::V0_2(V0_2CsiRequest::Complete(completion_request));

        // When
        let api_token = "dummy auth token";
        let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
        auth_value.set_sensitive(true);
        let csi = StubCsi::with_completion(|r| inference::Completion::from_text(r.prompt));
        let app_state = AppState::dummy().with_csi_drivers(csi);
        let http = http(app_state);

        let resp = http
            .oneshot(
                Request::builder()
                    .method(http::Method::POST)
                    .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                    .header(header::AUTHORIZATION, auth_value)
                    .uri("/csi")
                    .body(Body::from(serde_json::to_string(&request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let completion = serde_json::from_slice::<Completion>(&body).unwrap();
        assert_eq!(completion.text, prompt);
    }

    #[tokio::test]
    async fn execute_skill() {
        // Given
        let skill_executer_mock = StubSkillExecuter::new(move |msg| {
            let SkillExecutorMsg {
                skill_path, send, ..
            } = msg;
            assert_eq!(skill_path, SkillPath::new("local", "greet_skill"));
            send.send(Ok(json!("dummy completion"))).unwrap();
        });
        let skill_executor_api = skill_executer_mock.api();
        let app_state = AppState::dummy().with_skill_executor_api(skill_executor_api.clone());
        let http = http(app_state);

        // When
        let api_token = "dummy auth token";
        let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
        auth_value.set_sensitive(true);

        let args = ExecuteSkillArgs {
            skill: "local/greet_skill".to_owned(),
            input: json!("Homer"),
        };
        let resp = http
            .oneshot(
                Request::builder()
                    .method(http::Method::POST)
                    .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                    .header(header::AUTHORIZATION, auth_value)
                    .uri("/execute_skill")
                    .body(Body::from(serde_json::to_string(&args).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let answer = serde_json::from_slice::<String>(&body).unwrap();
        assert_eq!(answer, "dummy completion");
    }

    #[tokio::test]
    async fn api_token_missing_permission() {
        // Given
        let saboteur_skill_executer = StubSkillExecuter::new(|_| panic!());
        let stub_authorization = StubAuthorization::new(|msg| {
            match msg {
                authorization::AuthorizationMsg::Auth { api_token: _, send } => {
                    drop(send.send(Ok(false)));
                }
            };
        });
        let app_state = AppState::dummy()
            .with_skill_executor_api(saboteur_skill_executer.api())
            .with_authorization_api(stub_authorization.api());

        // When we want to access an endpoint that requires authentication
        let api_token = api_token();
        let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
        auth_value.set_sensitive(true);

        let http = http(app_state);
        let args = ExecuteSkillArgs {
            skill: "greet".to_owned(),
            input: json!("Homer"),
        };
        let resp = http
            .oneshot(
                Request::builder()
                    .method(http::Method::POST)
                    .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                    .header(header::AUTHORIZATION, auth_value)
                    .uri("/execute_skill")
                    .body(Body::from(serde_json::to_string(&args).unwrap()))
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
    async fn api_token_missing_in_execute_skill() {
        // Given
        let saboteur_skill_executer = StubSkillExecuter::new(|_| panic!());
        let app_state = AppState::dummy().with_skill_executor_api(saboteur_skill_executer.api());

        // When
        let http = http(app_state);
        let args = ExecuteSkillArgs {
            skill: "greet".to_owned(),
            input: json!("Homer"),
        };
        let resp = http
            .oneshot(
                Request::builder()
                    .method(http::Method::POST)
                    .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                    .uri("/execute_skill")
                    .body(Body::from(serde_json::to_string(&args).unwrap()))
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
        let saboteur_skill_executer = StubSkillExecuter::new(|_| panic!());
        let (send, mut recv) = mpsc::channel(1);
        let skill_store_api = SkillStoreApi::new(send);
        tokio::spawn(async move {
            if let SkillStoreMessage::ListCached { send } = recv.recv().await.unwrap() {
                send.send(vec![
                    SkillPath::new("ns", "first"),
                    SkillPath::new("ns", "second"),
                ])
            } else {
                panic!("unexpected message in test")
            }
        });
        let app_state = AppState::dummy()
            .with_skill_store_api(skill_store_api.clone())
            .with_skill_executor_api(saboteur_skill_executer.api());
        let http = http(app_state);

        // When
        let api_token = api_token();
        let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
        auth_value.set_sensitive(true);

        let resp = http
            .oneshot(
                Request::builder()
                    .method(http::Method::GET)
                    .uri("/cached_skills")
                    .header(header::AUTHORIZATION, auth_value)
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
        let saboteur_skill_executer = StubSkillExecuter::new(|_| panic!());
        // We use this to spy on the path send to the skill executer. Better to use a channel,
        // rather than a mutex, but we do not have async closures yet.
        let skill_path = Arc::new(Mutex::new(None));
        let skill_path_clone = skill_path.clone();
        let (send, mut recv) = mpsc::channel(1);
        let skill_store_api = SkillStoreApi::new(send);
        tokio::spawn(async move {
            if let SkillStoreMessage::InvalidateCache { skill_path, send } =
                recv.recv().await.unwrap()
            {
                skill_path_clone.lock().unwrap().replace(skill_path);
                // `true` means it we actually deleted a skill
                send.send(true).unwrap();
            }
        });
        let app_state = AppState::dummy()
            .with_skill_store_api(skill_store_api.clone())
            .with_skill_executor_api(saboteur_skill_executer.api());
        let http = http(app_state);

        // When the skill is deleted
        let api_token = api_token();
        let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
        auth_value.set_sensitive(true);

        let resp = http
            .oneshot(
                Request::builder()
                    .method(http::Method::DELETE)
                    .uri("/cached_skills/haiku_skill".to_owned())
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
            SkillPath::new("pharia-kernel-team", "haiku_skill")
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
        // Given a runtime with one installed skill
        let saboteur_skill_executer = StubSkillExecuter::new(|_| panic!());
        // We use this to spy on the path send to the skill executer. Better to use a channel,
        // rather than a mutex, but we do not have async closures yet.
        let skill_path = Arc::new(Mutex::new(None));
        let skill_path_clone = skill_path.clone();
        let (send, mut recv) = mpsc::channel(1);
        let skill_store_api = SkillStoreApi::new(send);
        tokio::spawn(async move {
            if let SkillStoreMessage::InvalidateCache { skill_path, send } =
                recv.recv().await.unwrap()
            {
                skill_path_clone.lock().unwrap().replace(skill_path);
                // `false` means the skill has not been there before
                send.send(false).unwrap();
            }
        });
        let app_state = AppState::dummy()
            .with_skill_store_api(skill_store_api.clone())
            .with_skill_executor_api(saboteur_skill_executer.api());
        let http = http(app_state);

        // When the skill is deleted
        let api_token = api_token();
        let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
        auth_value.set_sensitive(true);

        let resp = http
            .oneshot(
                Request::builder()
                    .method(http::Method::DELETE)
                    .uri("/cached_skills/haiku_skill".to_owned())
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
            SkillPath::new("pharia-kernel-team", "haiku_skill")
        );
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let answer = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(answer, "\"Skill was not present in cache\"");
    }

    #[tokio::test]
    async fn blank_skill_name_is_bad_request() {
        // Given a shell with a skill executor API
        // drop the receiver, we expect the shell to never try to execute a skill
        let (send, _) = mpsc::channel(1);
        let skill_executor_api = SkillExecutorApi::new(send);
        let app_state = AppState::dummy().with_skill_executor_api(skill_executor_api.clone());
        let http = http(app_state);

        // When executing a skill with a blank name
        let api_token = api_token();
        let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
        auth_value.set_sensitive(true);
        let args = ExecuteSkillArgs {
            skill: "\n\n".to_owned(),
            input: json!("Homer"),
        };
        let resp = http
            .oneshot(
                Request::builder()
                    .method(http::Method::POST)
                    .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                    .header(header::AUTHORIZATION, auth_value)
                    .uri("/execute_skill")
                    .body(Body::from(serde_json::to_string(&args).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn healthcheck() {
        let saboteur_authorization = StubAuthorization::new(|msg| {
            match msg {
                authorization::AuthorizationMsg::Auth { api_token: _, send } => {
                    drop(send.send(Ok(false)));
                }
            };
        });
        let app_state = AppState::dummy().with_authorization_api(saboteur_authorization.api());
        let http = http(app_state);
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
    async fn skill_wit_route_should_return_current_wit_world() {
        let app_state = AppState::dummy();
        let http = http(app_state);
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

        assert_eq!(actual, include_str!("../wit/skill@0.2/skill.wit"));
    }

    #[tokio::test]
    async fn list_skills() {
        // Given we can provide two skills "ns_one/one" and "ns_two/two"
        let saboteur_skill_executer = StubSkillExecuter::new(|_| panic!());
        let (send, mut recv) = mpsc::channel(1);
        let skill_store_api = SkillStoreApi::new(send);
        tokio::spawn(async move {
            if let SkillStoreMessage::List { send } = recv.recv().await.unwrap() {
                send.send(vec![
                    SkillPath::new("ns_one", "one"),
                    SkillPath::new("ns_two", "two"),
                ])
                .unwrap();
            } else {
                panic!("Unexpected message in test")
            }
        });
        let app_state = AppState::dummy()
            .with_skill_store_api(skill_store_api)
            .with_skill_executor_api(saboteur_skill_executer.api());
        let http = http(app_state);

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
        let expected = "[\"ns_one/one\",\"ns_two/two\"]";
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
        let http = http(app_state);

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
        // Given a skill executor which has an invalid namespace
        let skill_executor = StubSkillExecuter::new(|msg| {
            let SkillExecutorMsg { send, .. } = msg;
            send.send(Err(ExecuteSkillError::Other(anyhow!(
                "Namespace is invalid"
            ))))
            .unwrap();
        });
        let app_state = AppState::dummy().with_skill_executor_api(skill_executor.api());
        let http = http(app_state);

        // When executing a skill in the namespace
        let auth_value = header::HeaderValue::from_str("Bearer DummyToken").unwrap();
        let args = ExecuteSkillArgs {
            skill: "any_namespace/any_skill".to_owned(),
            input: json!("Homer"),
        };
        let resp = http
            .oneshot(
                Request::builder()
                    .method(http::Method::POST)
                    .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                    .header(header::AUTHORIZATION, auth_value)
                    .uri("/execute_skill")
                    .body(Body::from(serde_json::to_string(&args).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then the response is 500 about invalid namespace
        assert_eq!(resp.status(), axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let response = serde_json::from_slice::<String>(&body).unwrap();
        assert_eq!(response, "Namespace is invalid");
    }

    #[tokio::test]
    async fn not_existing_skill_is_400_error() {
        // Given a skill executer which always replies Skill does not exist
        let skill_executer_dummy = StubSkillExecuter::new(|msg| {
            let SkillExecutorMsg { send, .. } = msg;
            send.send(Err(ExecuteSkillError::SkillDoesNotExist))
                .unwrap();
        });
        let auth_value = header::HeaderValue::from_str("Bearer DummyToken").unwrap();
        let app_state = AppState::dummy().with_skill_executor_api(skill_executer_dummy.api());

        // When executing a skill
        let http = http(app_state);
        let args = ExecuteSkillArgs {
            skill: "my_namespace/my_skill".to_owned(),
            input: json!("Homer"),
        };
        let resp = http
            .oneshot(
                Request::builder()
                    .method(http::Method::POST)
                    .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                    .header(header::AUTHORIZATION, auth_value)
                    .uri("/execute_skill")
                    .body(Body::from(serde_json::to_string(&args).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        // Cleanup
        skill_executer_dummy.shutdown().await;

        // Then answer is 400 skill does not exist
        assert_eq!(StatusCode::BAD_REQUEST, resp.status());
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(
            "\"The requested skill does not exist. Make sure it is configured in the configuration \
            associated with the namespace.\"",
            body_str
        );
    }

    /// A skill executer double, loaded up with predefined answers.
    struct StubSkillExecuter {
        send: mpsc::Sender<SkillExecutorMsg>,
        handle: JoinHandle<()>,
    }

    impl StubSkillExecuter {
        pub fn new(mut handle: impl FnMut(SkillExecutorMsg) + Send + 'static) -> StubSkillExecuter {
            let (send, mut recv) = mpsc::channel(1);
            let handle = tokio::spawn(async move {
                while let Some(msg) = recv.recv().await {
                    handle(msg);
                }
            });
            Self { send, handle }
        }

        pub fn api(&self) -> SkillExecutorApi {
            SkillExecutorApi::new(self.send.clone())
        }

        pub async fn shutdown(self) {
            drop(self.send);
            self.handle.await.unwrap();
        }
    }
}
