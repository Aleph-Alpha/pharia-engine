//! The agent-results module is concerned with the results of the work of async agents.
//!
//! We want to introduce the concept of scheduled agents, that run periodically and can produce
//! certain outputs. These outputs are provided via http for frontends to consume.

use axum::{Json, Router, routing::get};
use http::StatusCode;
use serde::Serialize;

use crate::FeatureSet;

pub fn http_agent_results_v1<T>(feature_set: FeatureSet) -> Router<T>
where
    T: Send + Sync + Clone + 'static,
{
    if feature_set == FeatureSet::Beta {
        Router::new().route("/agent_results", get(agent_results))
    } else {
        Router::new()
    }
}

/// A agent-result represents the output of an agent.
#[derive(Debug, Serialize)]
struct AgentResult {
    content: String,
}

async fn agent_results() -> Result<Json<Vec<AgentResult>>, (StatusCode, Json<String>)> {
    Ok(Json(vec![AgentResult {
        content: "Why do programmers prefer dark mode? Because light attracts bugs!".to_string(),
    }]))
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn agent_results_in_beta() {
        // Given
        let app = http_agent_results_v1(FeatureSet::Beta);

        // When
        let resp = app
            .oneshot(Request::get("/agent_results").body(Body::empty()).unwrap())
            .await
            .unwrap();

        // Then we get the hardcoded result
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap();
        let body = String::from_utf8(body.to_bytes().to_vec()).unwrap();
        assert_eq!(
            body,
            r#"[{"content":"Why do programmers prefer dark mode? Because light attracts bugs!"}]"#
        );
    }

    #[tokio::test]
    async fn agent_results_is_not_available_in_stable() {
        // Given a non-beta feature set
        let app = http_agent_results_v1(FeatureSet::Stable(1));

        // When
        let resp = app
            .oneshot(Request::get("/agent_results").body(Body::empty()).unwrap())
            .await
            .unwrap();

        // Then
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
