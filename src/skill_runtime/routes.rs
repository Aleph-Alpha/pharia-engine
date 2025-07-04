use std::convert::Infallible;

use async_stream::try_stream;
use axum::{
    Json, Router,
    extract::{FromRef, Path, State},
    response::{Sse, sse::Event},
    routing::{get, post},
};
use axum_extra::{
    TypedHeader,
    headers::{Authorization, authorization::Bearer},
};
use futures::Stream;
use reqwest::StatusCode;
use serde::Serialize;
use serde_json::Value;
use utoipa::{OpenApi, ToSchema};

use crate::{
    FeatureSet,
    logging::TracingContext,
    namespace_watcher::Namespace,
    shell::HttpError,
    skill_driver::{SkillExecutionError, SkillExecutionEvent},
    skill_runtime::SkillRuntimeApi,
    skills::{AnySkillManifest, JsonSchema, Signature, SkillPath},
};

pub fn http_skill_runtime_v1<T>(_feature_set: FeatureSet) -> Router<T>
where
    T: Send + Sync + Clone + SkillRuntimeProvider + 'static,
    T::SkillRuntime: SkillRuntimeApi + Send + Clone,
{
    Router::new()
        .route("/skills/{namespace}/{name}/run", post(run_skill))
        .route(
            "/skills/{namespace}/{name}/message-stream",
            post(message_stream_skill),
        )
        .route("/skills/{namespace}/{name}/metadata", get(skill_metadata))
}

pub fn openapi_skill_runtime_v1(feature_set: FeatureSet) -> utoipa::openapi::OpenApi {
    if feature_set == FeatureSet::Beta {
        SkillRuntimeOpenApiDocBeta::openapi()
    } else {
        SkillRuntimeOpenApiDoc::openapi()
    }
}

#[derive(OpenApi)]
#[openapi(paths(run_skill, message_stream_skill))]
struct SkillRuntimeOpenApiDoc;

#[derive(OpenApi)]
#[openapi(paths(run_skill, message_stream_skill, skill_metadata))]
struct SkillRuntimeOpenApiDocBeta;

pub trait SkillRuntimeProvider {
    type SkillRuntime;
    fn skill_runtime(&self) -> &Self::SkillRuntime;
}

/// Wrapper around Skill Runtime API for the shell. We use this strict alias to enable extracting a
/// reference from the [`AppState`] using a [`FromRef`] implementation.
pub struct SkillRuntimeState<M>(pub M);

impl<T> FromRef<T> for SkillRuntimeState<T::SkillRuntime>
where
    T: SkillRuntimeProvider,
    T::SkillRuntime: Clone,
{
    fn from_ref(app_state: &T) -> SkillRuntimeState<T::SkillRuntime> {
        SkillRuntimeState(app_state.skill_runtime().clone())
    }
}

/// Run
///
/// Run a Skill in the Kernel from one of the available repositories.
#[utoipa::path(
    post,
    operation_id = "run_skill",
    path = "/skills/{namespace}/{name}/run",
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
    let tracing_context = TracingContext::current();
    let skill_path = SkillPath::new(namespace, name);
    let response = skill_runtime_api
        .run_function(
            skill_path,
            input,
            bearer.token().to_owned(),
            tracing_context,
        )
        .await?;
    Ok(Json(response))
}

/// Stream
///
/// Stream from a Skill in the Kernel from one of the available repositories.
#[utoipa::path(
    post,
    operation_id = "message_stream_skill",
    path = "/skills/{namespace}/{name}/message-stream",
    request_body(
        content_type = "application/json",
        description = "The expected input for the skill in JSON format.",
        example = json!({})
    ),
    security(("api_token" = [])),
    tag = "skills",
    responses(
        (status = 200,
            description = "A stream of substrings composing a message",
            body=Value,
            content(("text/event-stream", examples(
                ("Tell me a joke" = (
                    summary = "Namespace: `example`, Skill: `tell_me_a_joke`",
                    description = "The `tell_me_a_joke` Skill in the `example` namespace streams a joke.",
                    value = json!("\
                        event: message\n\
                        data: {\"type\":\"begin\"}\n\
                        \n\
                        event: message\n\
                        data: {\"type\":\"append\",\"text\":\"Here's one:\\n\\nWhat do you call a fake noodle?\\n\\nAn impasta!\\n\\nHope that made you laugh! Do you want to hear another one?\"}\n\
                        \n\
                        event: message\n\
                        data: {\"type\":\"end\",\"payload\":\"Stop\"}\n\
                    ")
                )),
                ("Hello" = (
                    summary = "Namespace: `example`, Skill: `hello`",
                    description = "The `hello` Skill in the `example` namespace streams each character of the text \"Hello\".",
                    value = json!("\
                        event: message\n\
                        data: {\"type\":\"begin\"}\n\
                        \n\
                        event: message\n\
                        data: {\"type\":\"append\",\"text\":\"H\"}\n\
                        \n\
                        event: message\n\
                        data: {\"type\":\"append\",\"text\":\"e\"}\n\
                        \n\
                        event: message\n\
                        data: {\"type\":\"append\",\"text\":\"l\"}\n\
                        \n\
                        event: message\n\
                        data: {\"type\":\"append\",\"text\":\"l\"}\n\
                        \n\
                        event: message\n\
                        data: {\"type\":\"append\",\"text\":\"o\"}\n\
                        \n\
                        event: message\n\
                        data: {\"type\":\"end\",\"payload\":null}\n\
                    ")
                )),
                ("Saboteur" = (
                    summary = "Namespace: `example`, Skill: `saboteur`",
                    description = "The `saboteur` Skill in the `example` namespace responds with an error.",
                    value = json!("\
                        event: error\n\
                        data: {\"message\":\"The skill you called responded with an error. Maybe you should check your input, if it seems to be correct you may want to contact the skill developer. Error reported by Skill:\\n\\nSkill is a saboteur\"}\n\
                    ")
                )),
                ("Non-existing Skill" = (
                    summary = "Namespace: `example`, Skill: `non-existing-skill`",
                    description = "The `non-existing-skill` Skill does not exist in the `example` namespace.",
                    value = json!("\
                        event: error\n\
                        data: {\"message\":\"Sorry, we could not find the skill you requested in its namespace. This can have three causes:\\n\n1. You sent the wrong skill name.\\n2. You sent the wrong namespace.\\n3. The skill is not configured in the namespace you requested. You may want to check the namespace configuration.\"}\n\
                    ")
                ))
            )))),
        ),
)]
async fn message_stream_skill<R>(
    State(SkillRuntimeState(skill_runtime_api)): State<SkillRuntimeState<R>>,
    bearer: TypedHeader<Authorization<Bearer>>,
    Path((namespace, name)): Path<(Namespace, String)>,
    Json(input): Json<Value>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>>
where
    R: SkillRuntimeApi,
{
    let path = SkillPath::new(namespace, name);
    let tracing_context = TracingContext::current();
    let mut stream_events = skill_runtime_api
        .run_message_stream(path, input, bearer.token().to_owned(), tracing_context)
        .await;

    // We need to use `try_stream!` instead of `stream!`, because `stream!` does not implement the
    // traits required to be converted into an http body for the response. Since we do wanna rely on
    // the implementation provided by axum for this, we use `try_stream!` with an infallible error
    // type. Please note, that we report the actual errors as "normal" events in the stream.
    let stream = try_stream! {
        while let Some(event) = stream_events.recv().await {
            // Convert stream events to Server Side Events
            yield event.into();
        }
    };

    Sse::new(stream)
}

impl From<SkillExecutionEvent> for Event {
    fn from(value: SkillExecutionEvent) -> Self {
        match value {
            SkillExecutionEvent::MessageBegin => Self::default()
                .event("message")
                .json_data(MessageEvent::Begin)
                .expect("`json_data` must only be called once."),
            SkillExecutionEvent::MessageEnd { payload } => Self::default()
                .event("message")
                .json_data(MessageEvent::End { payload })
                .expect("`json_data` must only be called once."),
            SkillExecutionEvent::MessageAppend { text } => Self::default()
                .event("message")
                .json_data(MessageEvent::Append { text })
                .expect("`json_data` must only be called once."),
            SkillExecutionEvent::ToolBegin { tool } => Self::default()
                .event("tool")
                .json_data(MessageEvent::ToolBegin { tool })
                .expect("`json_data` must only be called once."),
            SkillExecutionEvent::ToolEnd { tool, result } => Self::default()
                .event("tool")
                .json_data(MessageEvent::ToolEnd { tool })
                .expect("`json_data` must only be called once."),
            SkillExecutionEvent::Error(error) => Self::default()
                .event("error")
                .json_data(SseErrorEvent {
                    message: error.to_string(),
                })
                .expect("`json_data` must only be called once."),
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum MessageEvent {
    Begin,
    Append {
        text: String,
    },
    End {
        payload: Value,
    },
    // While the enum variants do not differ in the body, they are distinguished by the event field.
    #[serde(rename = "begin")]
    ToolBegin {
        tool: String,
    },
    #[serde(rename = "end")]
    ToolEnd {
        tool: String,
    },
}

#[derive(Serialize)]
struct SseErrorEvent {
    message: String,
}

impl From<SkillExecutionError> for HttpError {
    fn from(value: SkillExecutionError) -> Self {
        let status_code = match &value {
            // We return 5xx not only for runtime errors, but also for errors we consider Bugs in
            // the deployed skills.
            SkillExecutionError::MisconfiguredNamespace { .. }
            | SkillExecutionError::CsiUseFromMetadata
            | SkillExecutionError::InvalidOutput(_)
            | SkillExecutionError::MessageAppendWithoutMessageBegin
            | SkillExecutionError::MessageBeginWhileMessageActive
            | SkillExecutionError::MessageEndWithoutMessageBegin
            | SkillExecutionError::SkillLoadError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            // Service unavailable indicates better that the situation is temporary and retrying it
            // might be worth it.
            SkillExecutionError::RuntimeError(_) => StatusCode::SERVICE_UNAVAILABLE,
            // 400 for every error we see an error on the client side of HTTP
            SkillExecutionError::UserCode(_)
            | SkillExecutionError::SkillNotConfigured
            | SkillExecutionError::IsFunction
            | SkillExecutionError::IsMessageStream
            | SkillExecutionError::InvalidInput(_) => StatusCode::BAD_REQUEST,
        };
        HttpError::new(value.to_string(), status_code)
    }
}

/// Metadata
///
/// Get the metadata (input schema, output schema, description) for a Skill.
#[utoipa::path(
    get,
    operation_id = "skill_metadata",
    path = "/skills/{namespace}/{name}/metadata",
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
    Path((namespace, name)): Path<(Namespace, String)>,
) -> Result<Json<SkillMetadataV1Representation>, HttpError>
where
    R: SkillRuntimeApi,
{
    let tracing_context = TracingContext::current();
    let skill_path = SkillPath::new(namespace, name);
    let response = skill_runtime_api
        .skill_metadata(skill_path, tracing_context)
        .await?;
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

impl From<AnySkillManifest> for SkillMetadataV1Representation {
    fn from(metadata: AnySkillManifest) -> Self {
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

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::Request};
    use double_trait::Dummy;
    use http_body_util::BodyExt as _;
    use mime::{APPLICATION_JSON, TEXT_EVENT_STREAM};
    use reqwest::{
        Method, StatusCode,
        header::{AUTHORIZATION, CONTENT_TYPE},
    };
    use serde_json::{Value, json};
    use tokio::sync::mpsc;
    use tower::ServiceExt as _;

    use crate::{
        FeatureSet,
        feature_set::PRODUCTION_FEATURE_SET,
        logging::TracingContext,
        shell::tests::dummy_auth_value,
        skill_driver::{SkillExecutionError, SkillExecutionEvent},
        skill_runtime::{SkillRuntimeDouble, http_skill_runtime_v1, routes::SkillRuntimeProvider},
        skills::{AnySkillManifest, JsonSchema, Signature, SkillMetadataV0_3, SkillPath},
    };

    #[derive(Clone)]
    struct ProviderStub<T> {
        skill_runtime: T,
    }

    impl<T> ProviderStub<T> {
        fn new(skill_runtime: T) -> Self {
            Self { skill_runtime }
        }
    }

    impl<T> SkillRuntimeProvider for ProviderStub<T> {
        type SkillRuntime = T;

        fn skill_runtime(&self) -> &T {
            &self.skill_runtime
        }
    }

    #[tokio::test]
    async fn run_skill_with_bad_namespace() {
        // Given an invalid namespace
        let bad_namespace = "bad_namespace";
        let app_state = ProviderStub::new(Dummy);
        let http = http_skill_runtime_v1(PRODUCTION_FEATURE_SET).with_state(app_state);

        // When
        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, dummy_auth_value())
                    .uri(format!("/skills/{bad_namespace}/greet_skill/run"))
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
    async fn stream_endpoint_should_send_individual_message_deltas() {
        // Given
        let mut events = vec![SkillExecutionEvent::MessageBegin];
        events.extend("Hello".chars().map(|c| SkillExecutionEvent::MessageAppend {
            text: c.to_string(),
        }));
        events.push(SkillExecutionEvent::MessageEnd {
            payload: json!(null),
        });
        let app_state = ProviderStub::new(EventStreamStub::new(events));
        let http = http_skill_runtime_v1(PRODUCTION_FEATURE_SET).with_state(app_state);

        // When asking for a message stream
        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, dummy_auth_value())
                    .uri("/skills/local/hello/message-stream")
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
            event: message\n\
            data: {\"type\":\"begin\"}\n\n\
            event: message\n\
            data: {\"type\":\"append\",\"text\":\"H\"}\n\n\
            event: message\n\
            data: {\"type\":\"append\",\"text\":\"e\"}\n\n\
            event: message\n\
            data: {\"type\":\"append\",\"text\":\"l\"}\n\n\
            event: message\n\
            data: {\"type\":\"append\",\"text\":\"l\"}\n\n\
            event: message\n\
            data: {\"type\":\"append\",\"text\":\"o\"}\n\n\
            event: message\n\
            data: {\"type\":\"end\",\"payload\":null}\n\n";
        assert_eq!(body_text, expected_body);
    }

    #[tokio::test]
    async fn answer_of_succesfull_run_skill_function() {
        // Given
        #[derive(Clone)]
        struct SkillRuntimeStub;
        impl SkillRuntimeDouble for SkillRuntimeStub {
            async fn run_function(
                &self,
                _skill_path: SkillPath,
                _input: Value,
                _api_token: String,
                _tracing_context: TracingContext,
            ) -> Result<Value, SkillExecutionError> {
                Ok(json!("Result from Skill"))
            }
        }
        let app_state = ProviderStub::new(SkillRuntimeStub);
        let http = http_skill_runtime_v1(PRODUCTION_FEATURE_SET).with_state(app_state);

        // When
        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, dummy_auth_value())
                    .uri("/skills/local/greet_skill/run")
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
        #[derive(Clone)]
        struct SkillRuntimeMock;
        impl SkillRuntimeDouble for SkillRuntimeMock {
            fn run_function(
                &self,
                path: SkillPath,
                input: Value,
                auth: String,
                _: TracingContext,
            ) -> impl Future<Output = Result<Value, SkillExecutionError>> + Send {
                assert_eq!(path, SkillPath::local("greet_skill"));
                assert_eq!(input, json!("Homer"));
                assert_eq!(auth, "dummy auth token".to_owned());
                async move { Ok(json!({})) }
            }
        }
        let app_state = ProviderStub::new(SkillRuntimeMock);
        let http = http_skill_runtime_v1(PRODUCTION_FEATURE_SET).with_state(app_state);

        // When
        let _resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, dummy_auth_value())
                    .uri("/skills/local/greet_skill/run")
                    .body(Body::from(serde_json::to_string(&json!("Homer")).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn not_existing_skill_is_400_error() {
        // Given a skill executer which always replies Skill does not exist
        #[derive(Clone)]
        struct SkillRuntimeStub;
        impl SkillRuntimeDouble for SkillRuntimeStub {
            async fn run_function(
                &self,
                _: SkillPath,
                _: Value,
                _: String,
                _: TracingContext,
            ) -> Result<Value, SkillExecutionError> {
                Err(SkillExecutionError::SkillNotConfigured)
            }
        }
        let app_state = ProviderStub::new(SkillRuntimeStub);

        // When executing a skill
        let http = http_skill_runtime_v1(PRODUCTION_FEATURE_SET).with_state(app_state);
        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, dummy_auth_value())
                    .uri("/skills/any-namespace/any_skill/run")
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
            "Sorry, we could not find the skill you requested in its namespace. This can have \
            three causes:\n\n1. You sent the wrong skill name.\n2. You sent the wrong namespace.\
            \n3. The skill is not configured in the namespace you requested. You may want to \
            check the namespace configuration.",
            body_str
        );
    }

    #[tokio::test]
    async fn skill_metadata() {
        // Given
        #[derive(Clone)]
        struct SkillRuntimeStub;
        impl SkillRuntimeDouble for SkillRuntimeStub {
            async fn skill_metadata(
                &self,
                _: SkillPath,
                _: TracingContext,
            ) -> Result<AnySkillManifest, SkillExecutionError> {
                let metadata = AnySkillManifest::V0_3(SkillMetadataV0_3 {
                    description: Some("dummy description".to_owned()),
                    signature: Signature::Function {
                        input_schema: JsonSchema::dummy(),
                        output_schema: JsonSchema::dummy(),
                    },
                });
                Ok(metadata)
            }
        }
        let app_state = ProviderStub::new(SkillRuntimeStub);

        // When
        let http = http_skill_runtime_v1(PRODUCTION_FEATURE_SET).with_state(app_state);
        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .header(AUTHORIZATION, dummy_auth_value())
                    .uri("/skills/local/greet_skill/metadata")
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
    async fn stream_endpoint_for_saboteur_skill() {
        // Given
        let stream_events = vec![SkillExecutionEvent::Error(
            SkillExecutionError::RuntimeError("Skill is a saboteur".to_string()),
        )];
        let skill_runtime = EventStreamStub::new(stream_events);
        let app_state = ProviderStub::new(skill_runtime);
        let http = http_skill_runtime_v1(FeatureSet::Beta).with_state(app_state);

        // When asking for a message stream from a skill that does not exist
        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, dummy_auth_value())
                    .uri("/skills/local/saboteur/message-stream")
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
            data: {\"message\":\"The skill could not be executed to completion, something in our \
                runtime is currently unavailable or misconfigured. You should try again later, if \
                the situation persists you may want to contact the operators. Original error:\\n\\n\
                Skill is a saboteur\"}\n\n";
        assert_eq!(body_text, expected_body);
    }

    #[derive(Clone)]
    struct EventStreamStub {
        events: Vec<SkillExecutionEvent>,
    }

    impl EventStreamStub {
        fn new(events: Vec<SkillExecutionEvent>) -> Self {
            Self { events }
        }
    }

    impl SkillRuntimeDouble for EventStreamStub {
        async fn run_message_stream(
            &self,
            _skill_path: SkillPath,
            _input: Value,
            _api_token: String,
            _tracing_context: TracingContext,
        ) -> mpsc::Receiver<SkillExecutionEvent> {
            let (send, recv) = mpsc::channel(self.events.len());
            for ce in &self.events {
                send.send(ce.clone()).await.unwrap();
            }
            recv
        }
    }

    #[tokio::test]
    async fn stream_endpoint_tool_calling_skill() {
        // Given
        let events = vec![
            SkillExecutionEvent::ToolBegin {
                tool: "add".to_string(),
            },
            SkillExecutionEvent::ToolEnd {
                tool: "add".to_string(),
                result: Ok(()),
            },
        ];
        let skill_runtime = EventStreamStub::new(events);
        let app_state = ProviderStub::new(skill_runtime);
        let http = http_skill_runtime_v1(FeatureSet::Beta).with_state(app_state);

        // When asking for a message stream
        let resp = http
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, APPLICATION_JSON.as_ref())
                    .header(AUTHORIZATION, dummy_auth_value())
                    .uri("/skills/local/saboteur/message-stream")
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
            event: tool\n\
            data: {\"type\":\"begin\",\"tool\":\"add\"}\n\n\
            event: tool\n\
            data: {\"type\":\"end\",\"tool\":\"add\"}\n\n";
        assert_eq!(body_text, expected_body);
    }
}
