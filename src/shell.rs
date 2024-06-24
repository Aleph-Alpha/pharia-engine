use std::future::Future;

use crate::skills::SkillExecutorApi;
use anyhow::{Context, Error};
use axum::extract::{Json, State};
use axum::http::StatusCode;
use axum_extra::headers::authorization::Bearer;
use axum_extra::headers::Authorization;
use axum_extra::TypedHeader;

use axum::routing::post;
use axum::{routing::get, Router};
use serde::{Deserialize, Serialize};
use tokio::net::{TcpListener, ToSocketAddrs};

pub async fn run(
    addr: impl ToSocketAddrs,
    skill_executor_api: SkillExecutorApi,
    shutdown_signal: impl Future<Output = ()> + Send + 'static,
) -> Result<(), Error> {
    let listener = TcpListener::bind(addr).await.context(
        "Could not bind a tcp listener to host '{}' and port '{}' \
            please check environment vars for HOST and PORT.",
    )?;
    axum::serve(listener, http(skill_executor_api))
        .with_graceful_shutdown(shutdown_signal)
        .await?;
    Ok(())
}

pub fn http(skill_executor_api: SkillExecutorApi) -> Router {
    Router::new()
        .route("/execute_skill", post(execute_skill))
        .with_state(skill_executor_api)
        .route("/", get(|| async { "Hello, world!" }))
}

#[derive(Deserialize, Serialize)]
struct ExecuteSkillArgs {
    skill: String,
    input: String,
}

async fn execute_skill(
    State(mut skill_executor_api): State<SkillExecutorApi>,
    bearer: TypedHeader<Authorization<Bearer>>,
    Json(args): Json<ExecuteSkillArgs>,
) -> (StatusCode, String) {
    let result = skill_executor_api
        .execute_skill(args.skill, args.input, bearer.token().to_owned())
        .await;
    match result {
        Ok(response) => (StatusCode::OK, response),
        Err(err) => (StatusCode::BAD_REQUEST, err.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use crate::skills::{SkillExecutor, WasmRuntime};

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

    static API_TOKEN: OnceLock<String> = OnceLock::new();

    /// API Token used by tests to authenticate requests
    fn api_token() -> &'static str {
        API_TOKEN.get_or_init(|| {
            drop(dotenv());
            env::var("AA_API_TOKEN").expect("AA_API_TOKEN variable not set")
        })
    }

    #[cfg_attr(not(feature = "test_inference"), ignore)]
    #[tokio::test]
    async fn execute_skill() {
        let api_token = api_token();
        let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
        auth_value.set_sensitive(true);

        let inference = Inference::new();

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
        let inference = Inference::new();

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
    async fn hello_world() {
        let inference = Inference::new();

        let runtime = WasmRuntime::new();
        let http = http(SkillExecutor::new(runtime, inference.api()).api());
        let resp = http
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"Hello, world!");
    }
}
