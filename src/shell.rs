use std::{future::Future, iter::once, net::SocketAddr};

use anyhow::Context;
use axum::{
    extract::{MatchedPath, Path, State},
    http::{header::AUTHORIZATION, Request, StatusCode},
    response::Redirect,
    routing::{delete, get, post},
    Json, Router,
};
use axum_extra::{
    headers::{authorization::Bearer, Authorization},
    TypedHeader,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tower::ServiceBuilder;
use tower_http::{
    compression::CompressionLayer,
    decompression::DecompressionLayer,
    sensitive_headers::SetSensitiveRequestHeadersLayer,
    services::{ServeDir, ServeFile},
    trace::{DefaultOnResponse, TraceLayer},
};
use tracing::{info, info_span, Level};
use utoipa::{
    openapi::{
        self,
        security::{HttpAuthScheme, HttpBuilder, SecurityScheme},
    },
    Modify, OpenApi, ToSchema,
};
use utoipa_scalar::{Scalar, Servable};

use crate::skills::{ExecuteSkillError, SkillExecutorApi, SkillPath, SkillProviderApi};

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

/// Start a shell listening to incoming requests at the given address.
///
/// The outer future awaits the bind operation of the listener.
/// The inner future awaits the shutdown.
pub async fn run(
    addr: impl Into<SocketAddr>,
    skill_executor_api: SkillExecutorApi,
    skill_provider_api: SkillProviderApi,
    shutdown_signal: impl Future<Output = ()> + Send + 'static,
) -> impl Future<Output = anyhow::Result<()>> {
    let addr = addr.into();
    let listener = TcpListener::bind(addr).await.context(format!(
        "Could not bind a tcp listener to '{addr}' please check environment vars for \
        PHARIA_KERNEL_ADDRESS."
    ));
    info!("Listening on: {addr}");

    async {
        axum::serve(listener?, http(skill_executor_api, skill_provider_api))
            .with_graceful_shutdown(shutdown_signal)
            .await?;
        Ok(())
    }
}

pub fn http(skill_executor_api: SkillExecutorApi, skill_provider_api: SkillProviderApi) -> Router {
    let serve_dir =
        ServeDir::new("./doc/book/html").not_found_service(ServeFile::new("docs/index.html"));

    Router::new()
        .route("/cached_skills", get(cached_skills))
        .route("/cached_skills/:name", delete(drop_cached_skill))
        .route("/skills", get(skills))
        .with_state(skill_provider_api)
        .route("/skill.wit", get(skill_wit()))
        .route("/execute_skill", post(execute_skill))
        .with_state(skill_executor_api)
        .nest_service("/docs", serve_dir.clone())
        .merge(Scalar::with_url("/api-docs", ApiDoc::openapi()))
        .route("/openapi.json", get(serve_docs))
        .route("/healthcheck", get(|| async { "ok" }))
        .route("/", get(|| async { Redirect::permanent("/docs/") }))
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
                        .on_response(DefaultOnResponse::new().level(Level::INFO)),
                )
                // Compress responses
                .layer(CompressionLayer::new())
                .layer(DecompressionLayer::new()),
        )
}

#[derive(Deserialize, Serialize, ToSchema)]
struct ExecuteSkillArgs {
    /// The qualified name of the skill to invoke. The qualified name consists of a namespace and
    /// a skillname (e.g. 'acme/greet').
    /// If the namespace is omitted, the default 'pharia-kernel-team' namespace is used.
    ///
    skill: String,
    /// The expected input for the skill.
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
async fn skills(
    State(skill_provider_api): State<SkillProviderApi>,
) -> (StatusCode, Json<Vec<String>>) {
    let response = skill_provider_api.list().await;
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
    State(skill_provider_api): State<SkillProviderApi>,
) -> (StatusCode, Json<Vec<String>>) {
    let response = skill_provider_api.list_cached().await;
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
    path = "/cached_skills",
    tag = "skills",
    responses(
        (status = 200, body=String, example = json!("Skill removed from cache.")),
        (status = 200, body=String, example = json!("Skill was not present in cache.")),
    ),
)]
async fn drop_cached_skill(
    State(skill_provider_api): State<SkillProviderApi>,
    Path(name): Path<String>,
) -> (StatusCode, Json<String>) {
    let skill_path = SkillPath::from_str(&name);
    let skill_was_cached = skill_provider_api.invalidate_cache(skill_path).await;
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
        skills::{
            tests::{dummy_skill_provider_api, SkillExecutorMessage, SkillProviderMsg},
            ExecuteSkillError, SkillPath,
        },
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

    #[tokio::test]
    async fn execute_skill() {
        // Given
        let skill_path = SkillPath::new("local", "greet_skill");
        let skill_path_clone = skill_path.clone();
        let skill_executer_mock = StubSkillExecuter::new(move |msg| {
            if let SkillExecutorMessage::Execute {
                skill_path, send, ..
            } = msg
            {
                assert_eq!(skill_path, skill_path_clone);
                send.send(Ok(json!("dummy completion"))).unwrap();
            }
        });
        let skill_executor_api = skill_executer_mock.api();

        // When
        let api_token = api_token();
        let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
        auth_value.set_sensitive(true);
        skill_executor_api
            .upsert_skill(skill_path.clone(), None)
            .await;
        let http = http(skill_executor_api, dummy_skill_provider_api());

        let args = ExecuteSkillArgs {
            skill: skill_path.to_string(),
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
    async fn api_token_missing() {
        // Given
        let skill_executor = StubSkillExecuter::new(|_| {});

        // When
        let http = http(skill_executor.api(), dummy_skill_provider_api());
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
            "Header of type `authorization` was missing".to_owned()
        );
    }

    #[tokio::test]
    async fn list_cached_skills_for_user() {
        let dummy_skill_executer = StubSkillExecuter::new(|_| panic!());
        let (send, mut recv) = mpsc::channel(1);
        let skill_provider_api = SkillProviderApi::new(send);
        tokio::spawn(async move {
            if let SkillProviderMsg::ListCached { send } = recv.recv().await.unwrap() {
                send.send(vec![
                    SkillPath::new("ns", "first"),
                    SkillPath::new("ns", "second"),
                ])
            } else {
                panic!("unexpected message in test")
            }
        });

        let http = http(dummy_skill_executer.api(), skill_provider_api);

        let resp = http
            .oneshot(
                Request::builder()
                    .method(http::Method::GET)
                    .uri("/cached_skills")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let answer = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(answer, "[\"ns/first\",\"ns/second\"]");
    }

    #[tokio::test]
    async fn drop_cached_skill() {
        // Given a provider wchich answers invalidate cache with `true`
        let dummy_skill_executer = StubSkillExecuter::new(|_| panic!());
        // We use this to spy on the path send to the skill executer. Better to use a channel,
        // rather than a mutex, but we do not have async closures yet.
        let skill_path = Arc::new(Mutex::new(None));
        let skill_path_clone = skill_path.clone();
        let (send, mut recv) = mpsc::channel(1);
        let skill_provider_api = SkillProviderApi::new(send);
        tokio::spawn(async move {
            if let SkillProviderMsg::InvalidateCache { skill_path, send } =
                recv.recv().await.unwrap()
            {
                skill_path_clone.lock().unwrap().replace(skill_path);
                // `true` means it we actually deleted a skill
                send.send(true).unwrap();
            }
        });
        let http = http(dummy_skill_executer.api(), skill_provider_api);

        // When the skill is deleted
        let resp = http
            .oneshot(
                Request::builder()
                    .method(http::Method::DELETE)
                    .uri("/cached_skills/haiku_skill".to_owned())
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
        let dummy_skill_executer = StubSkillExecuter::new(|_| panic!());
        // We use this to spy on the path send to the skill executer. Better to use a channel,
        // rather than a mutex, but we do not have async closures yet.
        let skill_path = Arc::new(Mutex::new(None));
        let skill_path_clone = skill_path.clone();
        let (send, mut recv) = mpsc::channel(1);
        let skill_provider_api = SkillProviderApi::new(send);
        tokio::spawn(async move {
            if let SkillProviderMsg::InvalidateCache { skill_path, send } =
                recv.recv().await.unwrap()
            {
                skill_path_clone.lock().unwrap().replace(skill_path);
                // `false` means the skill has not been there before
                send.send(false).unwrap();
            }
        });
        let http = http(dummy_skill_executer.api(), skill_provider_api);

        // When the skill is deleted
        let resp = http
            .oneshot(
                Request::builder()
                    .method(http::Method::DELETE)
                    .uri("/cached_skills/haiku_skill".to_owned())
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
        let http = http(skill_executor_api, dummy_skill_provider_api());

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
        let (send, _recv) = mpsc::channel(1);
        let dummy_skill_executer_api = SkillExecutorApi::new(send);

        let http = http(dummy_skill_executer_api, dummy_skill_provider_api());
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
        let (send, _recv) = mpsc::channel(1);
        let dummy_skill_executer_api = SkillExecutorApi::new(send);

        let http = http(dummy_skill_executer_api, dummy_skill_provider_api());
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
        let dummy_skill_executer = StubSkillExecuter::new(|_| panic!());
        let (send, mut recv) = mpsc::channel(1);
        let skill_provider_api = SkillProviderApi::new(send);
        tokio::spawn(async move {
            if let SkillProviderMsg::List { send } = recv.recv().await.unwrap() {
                send.send(vec![
                    SkillPath::new("ns_one", "one"),
                    SkillPath::new("ns_two", "two"),
                ])
                .unwrap();
            } else {
                panic!("Unexpected message in test")
            }
        });
        let http = http(dummy_skill_executer.api(), skill_provider_api);

        // When
        let resp = http
            .oneshot(
                Request::builder()
                    .uri("/skills")
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
    async fn invalid_namespace_config_is_500_error() {
        // Given a skill executor which has an invalid namespace
        let skill_executor = StubSkillExecuter::new(|msg| {
            if let SkillExecutorMessage::Execute { send, .. } = msg {
                send.send(Err(ExecuteSkillError::Other(anyhow!(
                    "Namespace is invalid"
                ))))
                .unwrap();
            }
        });
        let http = http(skill_executor.api(), dummy_skill_provider_api());

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
            if let SkillExecutorMessage::Execute { send, .. } = msg {
                send.send(Err(ExecuteSkillError::SkillDoesNotExist))
                    .unwrap();
            }
        });
        let skill_executer_api = skill_executer_dummy.api();
        let auth_value = header::HeaderValue::from_str("Bearer DummyToken").unwrap();

        // When executing a skill
        let http = http(skill_executer_api, dummy_skill_provider_api());
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
        send: mpsc::Sender<SkillExecutorMessage>,
        handle: JoinHandle<()>,
    }

    impl StubSkillExecuter {
        pub fn new(
            mut handle: impl FnMut(SkillExecutorMessage) + Send + 'static,
        ) -> StubSkillExecuter {
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
