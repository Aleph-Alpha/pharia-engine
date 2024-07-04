use std::{future::Future, iter::once, sync::Arc};

use aide::{
    axum::{
        routing::{get, get_with, post_with},
        ApiRouter,
    },
    transform::TransformOperation,
};
use anyhow::{Context, Error};
use axum::{
    extract::{MatchedPath, State},
    http::{header::AUTHORIZATION, Request, StatusCode},
    response::Redirect,
    Extension,
};
use axum_extra::{
    headers::{authorization::Bearer, Authorization},
    TypedHeader,
};
use extractors::Json;
use openapi::{api_docs, open_api, openapi_routes};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::net::{TcpListener, ToSocketAddrs};
use tower::ServiceBuilder;
use tower_http::{
    compression::CompressionLayer,
    decompression::DecompressionLayer,
    sensitive_headers::SetSensitiveRequestHeadersLayer,
    services::{ServeDir, ServeFile},
    trace::{DefaultOnResponse, TraceLayer},
};
use tracing::{info_span, Level};

use crate::skills::SkillExecutorApi;

mod extractors;
mod openapi;

pub async fn run(
    addr: impl ToSocketAddrs,
    skill_executor_api: SkillExecutorApi,
    shutdown_signal: impl Future<Output = ()> + Send + 'static,
) -> Result<(), Error> {
    let mut open_api = open_api();

    let app = http(skill_executor_api)
        .finish_api_with(&mut open_api, api_docs)
        .layer(Extension(Arc::new(open_api)))
        .into_make_service();

    let listener = TcpListener::bind(addr).await.context(
        "Could not bind a tcp listener to host '{}' and port '{}' \
                please check environment vars for HOST and PORT.",
    )?;

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal)
        .await?;
    Ok(())
}

pub fn http(skill_executor_api: SkillExecutorApi) -> ApiRouter {
    let serve_dir =
        ServeDir::new("./doc/book/html").not_found_service(ServeFile::new("docs/index.html"));

    ApiRouter::new()
        .api_route(
            "/execute_skill",
            post_with(execute_skill, execute_skill_docs),
        )
        .api_route(
            "/cached_skills",
            get_with(cached_skills, cached_skills_docs),
        )
        .with_state(skill_executor_api)
        .nest_service("/docs", serve_dir.clone())
        .route("/healthcheck", get(|| async { "ok" }))
        .route("/", get(|| async { Redirect::permanent("/docs/") }))
        .merge(openapi_routes())
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

#[derive(Deserialize, Serialize, JsonSchema)]
struct ExecuteSkillArgs {
    /// The name of the skill to invoke from one of the configured repositories.
    skill: String,
    /// The expected input for the skill.
    input: String,
}

async fn execute_skill(
    State(mut skill_executor_api): State<SkillExecutorApi>,
    bearer: TypedHeader<Authorization<Bearer>>,
    Json(args): Json<ExecuteSkillArgs>,
) -> (StatusCode, Json<String>) {
    let result = skill_executor_api
        .execute_skill(args.skill, args.input, bearer.token().to_owned())
        .await;
    match result {
        Ok(response) => (StatusCode::OK, Json(response)),
        Err(err) => (StatusCode::BAD_REQUEST, Json(err.to_string())),
    }
}

fn execute_skill_docs(op: TransformOperation<'_>) -> TransformOperation<'_> {
    op.id("executeSkill")
        .description("Execute a skill in the kernel from one of the available repositories.")
        .response_with::<200, Json<String>, _>(|res| res.description("The Skill was executed."))
        .response_with::<400, Json<String>, _>(|res| {
            res.description("The Skill invocation failed.")
                .example("Skill not found.")
        })
}

async fn cached_skills(
    State(mut skill_executor_api): State<SkillExecutorApi>,
) -> (StatusCode, Json<Vec<String>>) {
    let response = skill_executor_api.skills().await;
    (StatusCode::OK, Json(response))
}

fn cached_skills_docs(op: TransformOperation<'_>) -> TransformOperation<'_> {
    op.id("cachedSkills")
        .description(
            "List of all cached skills. These are skills that are already compiled \
            and are faster because they do not have to be transpiled to machine code. \
            When executing a skill which is not loaded yet, it will be cached.",
        )
        .response_with::<200, Json<Vec<String>>, _>(|res| {
            res.example(["first skill".to_owned(), "second skill".to_owned()])
        })
}

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
