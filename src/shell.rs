use std::{future::Future, iter::once, net::SocketAddr};

use anyhow::{Context, Error};
use axum::{
    extract::{MatchedPath, State},
    http::{header::AUTHORIZATION, Request, StatusCode},
    response::Redirect,
    routing::{get, post},
    Json, Router,
};
use axum_extra::{
    headers::{authorization::Bearer, Authorization},
    TypedHeader,
};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tower::ServiceBuilder;
use tower_http::{
    compression::CompressionLayer,
    decompression::DecompressionLayer,
    sensitive_headers::SetSensitiveRequestHeadersLayer,
    services::{ServeDir, ServeFile},
    trace::{DefaultOnResponse, TraceLayer},
};
use tracing::{info_span, Level};
use utoipa::{
    openapi::{
        self,
        security::{HttpAuthScheme, HttpBuilder, SecurityScheme},
    },
    Modify, OpenApi, ToSchema,
};
use utoipa_scalar::{Scalar, Servable};

use crate::skills::SkillExecutorApi;

#[derive(OpenApi)]
#[openapi(
    info(description = "The best place to run serverless AI applications."),
    paths(serve_docs, cached_skills, execute_skill),
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

pub async fn run(
    addr: impl Into<SocketAddr>,
    skill_executor_api: SkillExecutorApi,
    shutdown_signal: impl Future<Output = ()> + Send + 'static,
) -> Result<(), Error> {
    let addr = addr.into();
    let listener = TcpListener::bind(addr).await.context(format!(
        "Could not bind a tcp listener to '{addr}' please check environment vars for \
        PHARIA_KERNEL_ADDRESS."
    ))?;

    axum::serve(listener, http(skill_executor_api))
        .with_graceful_shutdown(shutdown_signal)
        .await?;
    Ok(())
}

pub fn http(skill_executor_api: SkillExecutorApi) -> Router {
    let serve_dir =
        ServeDir::new("./doc/book/html").not_found_service(ServeFile::new("docs/index.html"));

    Router::new()
        .route("/execute_skill", post(execute_skill))
        .route("/cached_skills", get(cached_skills))
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
    /// The name of the skill to invoke from one of the configured repositories.
    skill: String,
    /// The expected input for the skill.
    input: String,
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
    State(mut skill_executor_api): State<SkillExecutorApi>,
    bearer: TypedHeader<Authorization<Bearer>>,
    Json(args): Json<ExecuteSkillArgs>,
) -> (StatusCode, Json<String>) {
    if args.skill.trim().is_empty() {
        return (
            VALIDATION_ERROR_STATUS_CODE,
            Json("empty skill names are not allowed".to_owned()),
        );
    }
    let result = skill_executor_api
        .execute_skill(args.skill, args.input, bearer.token().to_owned())
        .await;
    match result {
        Ok(response) => (StatusCode::OK, Json(response)),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())),
    }
}

/// cached_skills
///
/// List of all cached skills. These are skills that are already compiled
/// and are faster because they do not have to be transpiled to machine code.
/// When executing a skill which is not loaded yet, it will be cached.
#[utoipa::path(
    post,
    operation_id = "cached_skills",
    path = "/cached_skills",
    tag = "skills",
    responses(
        (status = 200, body=Vec<String>, example = json!(["first skill", "second skill"])),
    ),
)]
async fn cached_skills(
    State(mut skill_executor_api): State<SkillExecutorApi>,
) -> (StatusCode, Json<Vec<String>>) {
    let response = skill_executor_api.skills().await;
    (StatusCode::OK, Json(response))
}

/// We use BAD_REQUEST (400) for validation error as it is more commonly used.
/// UNPROCESSABLE_ENTITY (422) is an alternative, but it may surprise users as
/// it is less commonly known
const VALIDATION_ERROR_STATUS_CODE: StatusCode = StatusCode::BAD_REQUEST;

#[cfg(test)]
mod tests {
    use crate::{
        inference::tests::InferenceStub,
        skills::{tests::LiarRuntime, SkillExecutor, WasmRuntime},
    };

    use super::*;

    use crate::inference::Inference;
    use axum::{
        body::Body,
        http::{self, header, Request},
    };
    use dotenvy::dotenv;
    use http_body_util::BodyExt;
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

        let inference = Inference::new(inference_addr().to_owned());

        let runtime = WasmRuntime::new();
        let http = http(SkillExecutor::new(runtime, inference.api()).api());

        let args = ExecuteSkillArgs {
            skill: "greet_skill".to_owned(),
            input: "Homer".to_owned(),
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
        let answer = String::from_utf8(body.to_vec()).unwrap();
        assert!(answer.contains("Homer"));
    }

    #[tokio::test]
    async fn api_token_missing() {
        let inference = Inference::new(inference_addr().to_owned());

        let runtime = WasmRuntime::new();
        let http = http(SkillExecutor::new(runtime, inference.api()).api());
        let args = ExecuteSkillArgs {
            skill: "greet".to_owned(),
            input: "Homer".to_owned(),
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
        let skills = vec!["First skill".to_owned(), "Second skill".to_owned()];
        let runtime = LiarRuntime::new(skills);
        let inference = InferenceStub::with_completion("hello");

        let http = http(SkillExecutor::new(runtime, inference.api()).api());

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
            input: "Homer".to_owned(),
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
        let inference = Inference::new(inference_addr().to_owned());

        let runtime = WasmRuntime::new();
        let http = http(SkillExecutor::new(runtime, inference.api()).api());
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
}
