#![allow(keyword_idents_2024)]

use std::sync::Arc;

use aide::{
    axum::{routing::get, ApiRouter, IntoApiResponse},
    gen::{all_error_responses, extract_schemas, infer_responses, on_error},
    openapi::OpenApi,
    scalar::Scalar,
    transform::TransformOpenApi,
};
use axum::Extension;
use tracing::error;

use super::extractors::Json;

pub fn open_api() -> OpenApi {
    on_error(|error| {
        error!("{error}");
    });
    infer_responses(true);
    extract_schemas(true);
    all_error_responses(true);

    OpenApi::default()
}

pub fn api_docs(api: TransformOpenApi<'_>) -> TransformOpenApi<'_> {
    api.title("Pharia Kernel")
        .version("0.1.0")
        .description("The best place to run serverless AI applications.")
    // .description(include_str!("../../README.md"))
}

pub fn docs_routes() -> ApiRouter {
    ApiRouter::new()
        .route("/", Scalar::new("/docs/api.json").axum_route())
        .route("/api.json", get(serve_docs))
}

async fn serve_docs(Extension(api): Extension<Arc<OpenApi>>) -> impl IntoApiResponse {
    Json(api)
}
