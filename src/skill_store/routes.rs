use axum::{
    Json, Router,
    extract::{FromRef, Path, Query, State},
    routing::{delete, get},
};
use serde::Deserialize;
use utoipa::{OpenApi, ToSchema};

use crate::{
    FeatureSet, namespace_watcher::Namespace, skill_common::SkillPath,
    skill_loader::SkillDescriptionFilterType, skill_store::SkillStoreApi,
};

pub fn http_skill_store_v0<T>(_feature_set: FeatureSet) -> Router<T>
where
    T: Send + Sync + Clone + SkillStoreProvider + 'static,
    T::SkillStore: SkillStoreApi + Send + Clone,
{
    // Hidden routes for cache for internal use
    Router::new()
        .route("/cached_skills", get(cached_skills))
        .route(
            "/cached_skills/{namespace}/{name}",
            delete(drop_cached_skill),
        )
}

pub fn http_skill_store_v1<T>(feature_set: FeatureSet) -> Router<T>
where
    T: Send + Sync + Clone + SkillStoreProvider + 'static,
    T::SkillStore: SkillStoreApi + Send + Clone,
{
    // Additional routes for beta features, please keep them in sync with the API documentation.
    if feature_set == FeatureSet::Beta {
        Router::new().route("/skills", get(skills_beta))
    } else {
        Router::new().route("/skills", get(skills))
    }
}

pub fn openapi_skill_store_v1(feature_set: FeatureSet) -> utoipa::openapi::OpenApi {
    if feature_set == FeatureSet::Beta {
        SkillStoreOpenApiDocBeta::openapi()
    } else {
        SkillStoreOpenApiDoc::openapi()
    }
}

#[derive(OpenApi)]
#[openapi(paths(skills))]
struct SkillStoreOpenApiDoc;

#[derive(OpenApi)]
#[openapi(paths(skills_beta))]
struct SkillStoreOpenApiDocBeta;

pub trait SkillStoreProvider {
    type SkillStore;
    fn skill_store(&self) -> &Self::SkillStore;
}

/// Wrapper around Skill Store API for the shell. We use this strict alias to enable extracting a
/// reference from the [`AppState`] using a [`FromRef`] implementation.
pub struct SkillStoreState<M>(pub M);

impl<T> FromRef<T> for SkillStoreState<T::SkillStore>
where
    T: SkillStoreProvider,
    T::SkillStore: Clone,
{
    fn from_ref(app_state: &T) -> SkillStoreState<T::SkillStore> {
        SkillStoreState(app_state.skill_store().clone())
    }
}

/// List of configured Skills
#[utoipa::path(
    get,
    operation_id = "skills",
    path = "/skills",
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

/// List of configured Skills
#[utoipa::path(
    get,
    operation_id = "skills",
    path = "/skills",
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

#[derive(Deserialize, ToSchema)]
struct SkillListParams {
    #[serde(rename = "type")]
    skill_type: Option<SkillDescriptionSchemaType>,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
enum SkillDescriptionSchemaType {
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

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt as _;
    use reqwest::Method;
    use tower::ServiceExt as _;

    use crate::{
        feature_set::PRODUCTION_FEATURE_SET,
        namespace_watcher::Namespace,
        skill_common::SkillPath,
        skill_loader::SkillDescriptionFilterType,
        skill_store::{
            SkillStoreApiDouble, SkillStoreProvider, http_skill_store_v0, http_skill_store_v1,
        },
    };

    #[derive(Clone)]
    struct ProviderStub<T> {
        skill_store: T,
    }

    impl<T> ProviderStub<T> {
        fn new(skill_store: T) -> Self {
            Self { skill_store }
        }
    }

    impl<T> SkillStoreProvider for ProviderStub<T> {
        type SkillStore = T;

        fn skill_store(&self) -> &T {
            &self.skill_store
        }
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
        let app_state = ProviderStub::new(SkillStoreStub);
        let http = http_skill_store_v1(PRODUCTION_FEATURE_SET).with_state(app_state);

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
        let expected = "[\"ns-one/one\",\"ns-two/two\"]";
        assert_eq!(actual, expected);
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
        let app_state = ProviderStub::new(SkillStoreStub);
        let http = http_skill_store_v0(PRODUCTION_FEATURE_SET).with_state(app_state);

        // When
        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/cached_skills")
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
    async fn drop_non_cached_skill() {
        // Given an empty skill cache
        #[derive(Clone)]
        struct SkillStoreMock;

        impl SkillStoreApiDouble for SkillStoreMock {
            async fn invalidate_cache(&self, path: SkillPath) -> bool {
                assert_eq!(
                    SkillPath::new(Namespace::new("my-test-namespace").unwrap(), "test-skill"),
                    path
                );
                false
            }
        }
        let app_state = ProviderStub::new(SkillStoreMock);
        let namespace = Namespace::new("my-test-namespace").unwrap();
        let http = http_skill_store_v0(PRODUCTION_FEATURE_SET).with_state(app_state);

        // When the skill is deleted
        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri(format!("/cached_skills/{namespace}/test-skill"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then we get an ok and a message indicating the skill was not present in cache
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let answer = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(answer, "\"Skill was not present in cache\"");
    }

    #[tokio::test]
    async fn drop_cached_skill() {
        // Given a provider which answers invalidate cache with `true`
        #[derive(Clone)]
        struct SkillStoreMock;

        impl SkillStoreApiDouble for SkillStoreMock {
            async fn invalidate_cache(&self, path: SkillPath) -> bool {
                assert_eq!(
                    SkillPath::new(Namespace::new("my-test-namespace").unwrap(), "test-skill"),
                    path
                );
                true
            }
        }
        let app_state = ProviderStub::new(SkillStoreMock);
        let namespace = Namespace::new("my-test-namespace").unwrap();
        let http = http_skill_store_v0(PRODUCTION_FEATURE_SET).with_state(app_state);

        // When the skill is deleted
        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri(format!("/cached_skills/{namespace}/test-skill"))
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
}
