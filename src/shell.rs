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

use crate::skills::{SkillExecutorApi, SkillPath};

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
    shutdown_signal: impl Future<Output = ()> + Send + 'static,
) -> impl Future<Output = anyhow::Result<()>> {
    let addr = addr.into();
    let listener = TcpListener::bind(addr).await.context(format!(
        "Could not bind a tcp listener to '{addr}' please check environment vars for \
        PHARIA_KERNEL_ADDRESS."
    ));
    info!("Listening on: {addr}");

    async {
        axum::serve(listener?, http(skill_executor_api))
            .with_graceful_shutdown(shutdown_signal)
            .await?;
        Ok(())
    }
}

pub fn http(skill_executor_api: SkillExecutorApi) -> Router {
    let serve_dir =
        ServeDir::new("./doc/book/html").not_found_service(ServeFile::new("docs/index.html"));

    Router::new()
        .route("/skill.wit", get(skill_wit()))
        .route("/execute_skill", post(execute_skill))
        .route("/skills", get(skills))
        .route("/cached_skills", get(cached_skills))
        .route("/cached_skills/:name", delete(drop_cached_skill))
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
        Err(err) => (
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
    State(skill_executor_api): State<SkillExecutorApi>,
) -> (StatusCode, Json<Vec<String>>) {
    let response = skill_executor_api.skills().await;
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
    State(skill_executor_api): State<SkillExecutorApi>,
) -> (StatusCode, Json<Vec<String>>) {
    let response = skill_executor_api.loaded_skills().await;
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
    State(skill_executor_api): State<SkillExecutorApi>,
    Path(name): Path<String>,
) -> (StatusCode, Json<String>) {
    let skill_path = SkillPath::from_str(&name);
    let skill_was_cached = skill_executor_api.drop_from_cache(skill_path).await;
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
    use crate::{
        configuration_observer::OperatorConfig,
        inference::tests::InferenceStub,
        skills::{tests::LiarRuntime, SkillExecutor, SkillPath},
    };

    use super::*;

    use crate::inference::Inference;
    use axum::{
        body::Body,
        http::{self, header, Request},
    };
    use dotenvy::dotenv;
    use http_body_util::BodyExt;
    use serde_json::json;
    use std::env;
    use std::sync::OnceLock;
    use tokio::sync::mpsc;
    use tower::util::ServiceExt;

    /// API Token used by tests to authenticate requests
    fn api_token() -> &'static str {
        static API_TOKEN: OnceLock<String> = OnceLock::new();
        API_TOKEN.get_or_init(|| {
            drop(dotenv());
            env::var("AA_API_TOKEN").expect("AA_API_TOKEN variable not set")
        })
    }

    /// inference address used by test to call inference
    fn inference_addr() -> &'static str {
        static INFERENCE_ADDRESS: OnceLock<String> = OnceLock::new();
        INFERENCE_ADDRESS.get_or_init(|| {
            drop(dotenv());
            env::var("INFERENCE_ADDRESS")
                .unwrap_or_else(|_| "https://api.aleph-alpha.com".to_owned())
        })
    }

    #[cfg_attr(not(feature = "test_inference"), ignore)]
    #[tokio::test]
    async fn execute_skill() {
        let api_token = api_token();
        let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
        auth_value.set_sensitive(true);

        let completion = "dummy completion";
        let inference = InferenceStub::with_completion(completion);
        let config = OperatorConfig::local();
        let skill_executor_api = SkillExecutor::new(inference.api(), &config.namespaces).api();
        let skill_path = SkillPath::new("local", "greet_skill");
        skill_executor_api
            .upsert_skill(skill_path.clone(), None)
            .await;
        let http = http(skill_executor_api);

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
        assert_eq!(answer, completion);
    }

    #[tokio::test]
    async fn api_token_missing() {
        let inference = Inference::new(inference_addr().to_owned());
        let config = OperatorConfig::local();

        let http = http(SkillExecutor::new(inference.api(), &config.namespaces).api());
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
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            String::from_utf8(body.to_vec()).unwrap(),
            "Header of type `authorization` was missing".to_owned()
        );
    }

    #[tokio::test]
    async fn cached_skills_are_returned() {
        let skills = ["First skill".to_owned(), "Second skill".to_owned()];
        let runtime = LiarRuntime::new(&skills);
        let inference = InferenceStub::with_completion("hello");

        let http = http(SkillExecutor::with_runtime(runtime, inference.api()).api());

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

        assert!(answer.contains("First skill"));
        assert!(answer.contains("Second skill"));
    }

    #[tokio::test]
    async fn drop_cached_skill() {
        // Given a runtime with one installed skill
        let skill_name = "haiku_skill".to_owned();
        let runtime = LiarRuntime::new(&[skill_name.clone()]);
        let inference = InferenceStub::with_completion("hello");
        let http = http(SkillExecutor::with_runtime(runtime, inference.api()).api());

        // When the skill is deleted
        let resp = http
            .oneshot(
                Request::builder()
                    .method(http::Method::DELETE)
                    .uri(format!("/cached_skills/{skill_name}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let answer = String::from_utf8(body.to_vec()).unwrap();
        assert!(answer.contains("removed from cache"));
    }

    #[tokio::test]
    async fn drop_non_cached_skill() {
        // Given a runtime without cached skills
        let runtime = LiarRuntime::new(&[]);
        let inference = InferenceStub::with_completion("hello");
        let http = http(SkillExecutor::with_runtime(runtime, inference.api()).api());

        // When a skills is deleted
        let resp = http
            .oneshot(
                Request::builder()
                    .method(http::Method::DELETE)
                    .uri(format!("/cached_skills/{}", "non-cached-skill"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let answer = String::from_utf8(body.to_vec()).unwrap();
        assert!(answer.contains("not present"));
    }

    #[tokio::test]
    async fn blank_skill_name_is_bad_request() {
        // Given a shell with a skill executor API
        // drop the receiver, we expect the shell to never try to execute a skill
        let (send, _) = mpsc::channel(1);
        let skill_executor_api = SkillExecutorApi::new(send);
        let http = http(skill_executor_api);

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

        let http = http(dummy_skill_executer_api);
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

        let http = http(dummy_skill_executer_api);
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
        // given a skill executor with cached skills
        let (send, mut recv) = mpsc::channel(1);
        let skill_executer_api = SkillExecutorApi::new(send);
        let skill_path = SkillPath::dummy();
        let skill_qualified_name = skill_path.to_string();
        tokio::spawn(async move {
            if let Some(crate::skills::tests::SkillExecutorMessage::Skills { send }) =
                recv.recv().await
            {
                send.send(vec![skill_path]).unwrap();
            }
        });

        let http = http(skill_executer_api);
        let resp = http
            .oneshot(
                Request::builder()
                    .uri("/skills")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let skills_str = String::from_utf8(body.to_vec()).unwrap();
        let skills = serde_json::from_str::<Vec<String>>(&skills_str).unwrap();
        assert_eq!(skills, vec![skill_qualified_name]);
    }
}
