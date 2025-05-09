use std::{borrow::Cow, future::Future, pin::Pin, sync::Arc, time::Instant};

use futures::{StreamExt, stream::FuturesUnordered};
use serde_json::Value;
use tokio::{
    select,
    sync::{mpsc, oneshot},
    task::JoinHandle,
};
use tracing::{Level, error, info, warn};

use crate::{
    context,
    csi::Csi,
    logging::TracingContext,
    skill_driver::SkillDriver,
    skill_store::{SkillStoreApi, SkillStoreError},
    skills::{AnySkillManifest, Engine, Skill, SkillPath},
};

// It would be nice for users of this module, not to be concerned with the fact that the runtime is
// using the driver. This may indicate that maybe driver and runtime should be part of the same top
// level module. For now I decided to leave it like that due to the fact that I am not sure about
// it. (MK)
pub use crate::skill_driver::{SkillExecutionError, SkillExecutionEvent};

/// An actor which invokes skills concurrently. It is responsible for fetching the skills from the
/// store. Reporting their results back over the API (the shell should be most intersted in it). It
/// also reports metrics and tracing to the operators.
pub struct SkillRuntime {
    send: mpsc::Sender<SkillRuntimeMsg>,
    handle: JoinHandle<()>,
}

impl SkillRuntime {
    /// Create a new skill runtime with the default web assembly runtime
    pub fn new<C>(
        engine: Arc<Engine>,
        csi_apis: C,
        store: impl SkillStoreApi + Send + Sync + 'static,
    ) -> Self
    where
        C: Csi + Clone + Send + Sync + 'static,
    {
        let driver = SkillDriver::new(engine);
        let (send, recv) = mpsc::channel::<SkillRuntimeMsg>(1);
        let handle = tokio::spawn(async {
            SkillRuntimeActor::new(driver, store, recv, csi_apis)
                .run()
                .await;
        });
        SkillRuntime { send, handle }
    }

    /// Retrieve a handle in order to interact with skills. All handles have to be dropped in order
    /// for [`Self::wait_for_shutdown`] to complete.
    pub fn api(&self) -> mpsc::Sender<SkillRuntimeMsg> {
        self.send.clone()
    }

    pub async fn wait_for_shutdown(self) {
        drop(self.send);
        self.handle.await.unwrap();
    }
}

/// The skill runtime API is used to interact with the skill runtime actor.
///
/// Using a trait rather than an mpsc allows for easier and more ergonomic testing, since the
/// implementation of the test double is not required to be an actor.
pub trait SkillRuntimeApi {
    fn run_function(
        &self,
        skill_path: SkillPath,
        input: Value,
        api_token: String,
        tracing_context: TracingContext,
    ) -> impl Future<Output = Result<Value, SkillExecutionError>> + Send;

    fn run_message_stream(
        &self,
        skill_path: SkillPath,
        input: Value,
        api_token: String,
        tracing_context: TracingContext,
    ) -> impl Future<Output = mpsc::Receiver<SkillExecutionEvent>> + Send;

    fn skill_metadata(
        &self,
        skill_path: SkillPath,
        tracing_context: TracingContext,
    ) -> impl Future<Output = Result<AnySkillManifest, SkillExecutionError>> + Send;
}

impl SkillRuntimeApi for mpsc::Sender<SkillRuntimeMsg> {
    async fn run_function(
        &self,
        skill_path: SkillPath,
        input: Value,
        api_token: String,
        tracing_context: TracingContext,
    ) -> Result<Value, SkillExecutionError> {
        let (send, recv) = oneshot::channel();
        let msg = SkillRuntimeMsg::Function(RunFunctionMsg {
            skill_path,
            input,
            send,
            api_token,
            tracing_context,
        });
        self.send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }

    async fn run_message_stream(
        &self,
        skill_path: SkillPath,
        input: Value,
        api_token: String,
        tracing_context: TracingContext,
    ) -> mpsc::Receiver<SkillExecutionEvent> {
        let (send, recv) = mpsc::channel::<SkillExecutionEvent>(1);

        let msg = RunMessageStreamMsg {
            skill_path,
            input,
            send,
            api_token,
            tracing_context,
        };

        self.send(SkillRuntimeMsg::MessageStream(msg))
            .await
            .expect("all api handlers must be shutdown before actors");
        recv
    }

    async fn skill_metadata(
        &self,
        skill_path: SkillPath,
        tracing_context: TracingContext,
    ) -> Result<AnySkillManifest, SkillExecutionError> {
        let (send, recv) = oneshot::channel();
        let msg = SkillRuntimeMsg::Metadata(MetadataMsg {
            skill_path,
            send,
            tracing_context,
        });
        self.send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }
}

struct SkillRuntimeActor<C, S> {
    driver: Arc<SkillDriver>,
    store: Arc<S>,
    recv: mpsc::Receiver<SkillRuntimeMsg>,
    csi_apis: C,
    // Can be a skill execution or a skill metadata request
    running_requests: FuturesUnordered<Pin<Box<dyn Future<Output = ()> + Send>>>,
}

impl<C, S> SkillRuntimeActor<C, S>
where
    C: Csi + Clone + Send + Sync + 'static,
    S: SkillStoreApi + Send + Sync + 'static,
{
    fn new(
        driver: SkillDriver,
        store: S,
        recv: mpsc::Receiver<SkillRuntimeMsg>,
        csi_apis: C,
    ) -> Self {
        SkillRuntimeActor {
            driver: Arc::new(driver),
            store: Arc::new(store),
            recv,
            csi_apis,
            running_requests: FuturesUnordered::new(),
        }
    }

    async fn run(&mut self) {
        loop {
            // While there are messages and running skills, poll both.
            // If there is a message, add it to the queue.
            // If there are skills, make progress on them.
            select! {
                msg = self.recv.recv() => match msg {
                    Some(msg) => {
                        let csi_apis = self.csi_apis.clone();
                        let driver = self.driver.clone();
                        let store = self.store.clone();
                        self.running_requests.push(Box::pin(async move {
                            msg.act(csi_apis, driver.as_ref(), store.as_ref()).await;
                        }));
                    },
                    // Senders are gone, break out of the loop for shutdown.
                    None => break
                },
                // FuturesUnordered will let them run in parallel. It will
                // yield once one of them is completed.
                () = self.running_requests.select_next_some(), if !self.running_requests.is_empty()  => {}
            }
        }
    }
}

pub enum SkillRuntimeMetrics {
    ExecutionTotal,
    ExecutionDurationSeconds,
    FetchDurationSeconds,
}

impl From<SkillRuntimeMetrics> for metrics::KeyName {
    fn from(value: SkillRuntimeMetrics) -> Self {
        Self::from_const_str(match value {
            SkillRuntimeMetrics::ExecutionTotal => "kernel_skill_execution_total",
            SkillRuntimeMetrics::ExecutionDurationSeconds => {
                "kernel_skill_execution_duration_seconds"
            }
            SkillRuntimeMetrics::FetchDurationSeconds => "kernel_skill_fetch_duration_seconds",
        })
    }
}

#[derive(Debug)]
pub enum SkillRuntimeMsg {
    MessageStream(RunMessageStreamMsg),
    Function(RunFunctionMsg),
    Metadata(MetadataMsg),
}

impl SkillRuntimeMsg {
    async fn act(
        self,
        csi_apis: impl Csi + Send + Sync + 'static,
        driver: &SkillDriver,
        store: &impl SkillStoreApi,
    ) {
        match self {
            SkillRuntimeMsg::MessageStream(msg) => {
                msg.act(csi_apis, driver, store).await;
            }
            SkillRuntimeMsg::Function(msg) => {
                msg.act(csi_apis, driver, store).await;
            }
            SkillRuntimeMsg::Metadata(msg) => {
                msg.act(driver, store).await;
            }
        }
    }
}

#[derive(Debug)]
pub struct MetadataMsg {
    pub skill_path: SkillPath,
    pub send: oneshot::Sender<Result<AnySkillManifest, SkillExecutionError>>,
    pub tracing_context: TracingContext,
}

impl MetadataMsg {
    pub async fn act(self, driver: &SkillDriver, store: &impl SkillStoreApi) {
        let skill_result = fetch_skill(store, &self.skill_path, &self.tracing_context).await;
        let skill = match skill_result {
            Ok(skill) => skill,
            Err(e) => {
                drop(self.send.send(Err(e)));
                return;
            }
        };
        let result = driver.metadata(skill, &self.tracing_context).await;
        drop(self.send.send(result));
    }
}

#[derive(Debug)]
pub struct RunMessageStreamMsg {
    pub skill_path: SkillPath,
    pub input: Value,
    pub send: mpsc::Sender<SkillExecutionEvent>,
    pub api_token: String,
    pub tracing_context: TracingContext,
}

impl RunMessageStreamMsg {
    async fn act(
        self,
        csi_apis: impl Csi + Send + Sync + 'static,
        driver: &SkillDriver,
        store: &impl SkillStoreApi,
    ) {
        let RunMessageStreamMsg {
            skill_path,
            input,
            send,
            api_token,
            tracing_context,
        } = self;

        let skill_result = {
            let load_context = context!(
                &tracing_context,
                "pharia-kernel::skill-runtime",
                "load_skill"
            );
            fetch_skill(store, &skill_path, &load_context).await
        };

        let skill = match skill_result {
            Ok(skill) => skill,
            Err(e) => {
                drop(send.send(SkillExecutionEvent::Error(e)).await);
                return;
            }
        };
        let start = Instant::now();
        let result = {
            let context = context!(tracing_context, "pharia_kernel::skill_runtime", "skill_execution", skill=%skill_path);
            let result = driver
                .run_message_stream(skill, input, csi_apis, api_token, &context, send.clone())
                .await;

            log_skill_result(&context, &skill_path, &result);
            result
        };
        record_skill_execution_metrics(start, skill_path, &result);
    }
}

async fn fetch_skill(
    store: &impl SkillStoreApi,
    skill_path: &SkillPath,
    tracing_context: &TracingContext,
) -> Result<Arc<dyn Skill>, SkillExecutionError> {
    let start = Instant::now();

    let result = match store.fetch(skill_path.clone(), tracing_context).await {
        Ok(Some(skill)) => Ok(skill),
        Ok(None) => Err(SkillExecutionError::SkillNotConfigured),
        Err(e) => Err(e.into()),
    };

    let latency = start.elapsed().as_secs_f64();
    let labels = [
        ("namespace", Cow::from(skill_path.namespace.to_string())),
        ("name", Cow::from(skill_path.name.clone())),
    ];
    metrics::histogram!(SkillRuntimeMetrics::FetchDurationSeconds, &labels).record(latency);

    result
}

impl From<SkillStoreError> for SkillExecutionError {
    fn from(source: SkillStoreError) -> Self {
        match source {
            SkillStoreError::SkillLoaderError(skill_loader_error) => {
                SkillExecutionError::RuntimeError(format!(
                    "Error loading skill: {skill_loader_error}"
                ))
            }
            SkillStoreError::InvalidNamespaceError(namespace, original_syntax_error) => {
                SkillExecutionError::MisconfiguredNamespace {
                    namespace,
                    original_syntax_error,
                }
            }
        }
    }
}

fn record_skill_execution_metrics<T>(
    start: Instant,
    skill_path: SkillPath,
    result: &Result<T, SkillExecutionError>,
) {
    let status = match result {
        Ok(_) => "ok",
        Err(
            SkillExecutionError::UserCode(_)
            | SkillExecutionError::CsiUseFromMetadata
            | SkillExecutionError::SkillNotConfigured
            | SkillExecutionError::InvalidInput(_)
            | SkillExecutionError::InvalidOutput(_)
            | SkillExecutionError::MisconfiguredNamespace { .. }
            | SkillExecutionError::IsFunction
            | SkillExecutionError::IsMessageStream
            | SkillExecutionError::MessageAppendWithoutMessageBegin
            | SkillExecutionError::MessageEndWithoutMessageBegin
            | SkillExecutionError::MessageBeginWhileMessageActive,
        ) => "logic_error",
        Err(SkillExecutionError::RuntimeError(_) | SkillExecutionError::SkillLoadError(_)) => {
            "runtime_error"
        }
    };

    let latency = start.elapsed().as_secs_f64();
    let labels = [
        ("namespace", Cow::from(skill_path.namespace.to_string())),
        ("name", Cow::from(skill_path.name)),
        ("status", status.into()),
    ];
    metrics::counter!(SkillRuntimeMetrics::ExecutionTotal, &labels).increment(1);
    metrics::histogram!(SkillRuntimeMetrics::ExecutionDurationSeconds, &labels).record(latency);
}

fn log_skill_result<T>(
    tracing_context: &TracingContext,
    skill_path: &SkillPath,
    result: &Result<T, SkillExecutionError>,
) {
    match result {
        Ok(_) => {
            info!(
                target: "pharia_kernel::skill_execution",
                parent: tracing_context.span(),
                skill=%skill_path,
                message="Skill executed successfully"
            );
        }
        Err(error) => match error.tracing_level() {
            Level::ERROR => error!(
                target: "pharia_kernel::skill_execution",
                parent: tracing_context.span(),
                skill=%skill_path,
                message="Skill invocation failed"
            ),
            Level::WARN => warn!(
                target: "pharia_kernel::skill_execution",
                parent: tracing_context.span(),
                skill=%skill_path,
                message="Skill invocation failed"
            ),
            Level::INFO => info!(
                target: "pharia_kernel::skill_execution",
                parent: tracing_context.span(),
                skill=%skill_path,
                message="Skill invocation failed"
            ),
            _ => {}
        },
    }
}

/// Message type used to transfer the input and output of a function skill execution
#[derive(Debug)]
pub struct RunFunctionMsg {
    pub skill_path: SkillPath,
    pub input: Value,
    pub send: oneshot::Sender<Result<Value, SkillExecutionError>>,
    pub api_token: String,
    pub tracing_context: TracingContext,
}

impl RunFunctionMsg {
    async fn act(
        self,
        csi_apis: impl Csi + Send + Sync + 'static,
        driver: &SkillDriver,
        store: &impl SkillStoreApi,
    ) {
        let RunFunctionMsg {
            skill_path,
            input,
            send,
            api_token,
            tracing_context,
        } = self;

        let skill_result = {
            let load_context = context!(
                &tracing_context,
                "pharia-kernel::skill-runtime",
                "load_skill"
            );
            fetch_skill(store, &skill_path, &load_context).await
        };
        let skill = match skill_result {
            Ok(skill) => skill,
            Err(e) => {
                drop(send.send(Err(e)));
                return;
            }
        };

        let start = Instant::now();
        let result = {
            let context = context!(tracing_context, "pharia_kernel::skill_runtime", "skill_execution", skill=%skill_path);
            let result = driver
                .run_function(skill, input, csi_apis, api_token, &context)
                .await;

            log_skill_result(&context, &skill_path, &result);
            result
        };
        record_skill_execution_metrics(start, skill_path, &result);

        // Error is expected to happen during shutdown. Ignore result.
        drop(send.send(result));
    }
}

#[cfg(test)]
pub mod tests {
    use std::time::Duration;

    use crate::{
        csi::{
            CsiForSkills,
            tests::{CsiDummy, CsiSaboteur},
        },
        hardcoded_skills::{SkillHello, SkillSaboteur, SkillTellMeAJoke},
        namespace_watcher::Namespace,
        skill_loader::{RegistryConfig, SkillLoader},
        skill_store::{SkillStore, tests::SkillStoreStub},
        skills::{AnySkillManifest, Skill, SkillError, SkillEvent},
    };
    use async_trait::async_trait;
    use bytesize::ByteSize;
    use metrics::Label;
    use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshot};
    use serde_json::json;
    use tokio::sync::broadcast;

    use super::*;

    #[tokio::test]
    async fn errors_for_non_existing_skill() {
        // Given a skill actor connected to an empty skill store
        let mut store = SkillStoreStub::new();
        store.with_fetch_response(None);
        let skill_actor = SkillRuntime::new(Arc::new(Engine::default()), CsiDummy, store);

        // When asking the skill actor to run the skill
        let result = skill_actor
            .api()
            .run_function(
                SkillPath::dummy(),
                json!(""),
                "dummy_token".to_owned(),
                TracingContext::dummy(),
            )
            .await;

        // Then the skill actor should return an error
        assert!(matches!(
            result,
            Err(SkillExecutionError::SkillNotConfigured)
        ));
    }

    #[tokio::test]
    async fn dedicated_error_for_skill_not_found() {
        // Given a skill executer with no skills
        let engine = Arc::new(Engine::default());
        let registry_config = RegistryConfig::empty();
        let skill_loader = SkillLoader::from_config(engine.clone(), registry_config).api();

        let skill_store =
            SkillStore::new(skill_loader, Duration::from_secs(10), ByteSize(u64::MAX));
        let csi_apis = CsiDummy;
        let executer = SkillRuntime::new(engine, csi_apis, skill_store.api());
        let api = executer.api();

        // When a skill is requested, but it is not listed in the namespace
        let result = api
            .run_function(
                SkillPath::local("my_skill"),
                json!("Any input"),
                "Dummy api token".to_owned(),
                TracingContext::dummy(),
            )
            .await;

        drop(api);
        executer.wait_for_shutdown().await;
        skill_store.wait_for_shutdown().await;

        // Then result indicates that the skill is missing
        assert!(matches!(
            result,
            Err(SkillExecutionError::SkillNotConfigured)
        ));
    }

    #[tokio::test]
    async fn greeting_skill_should_output_hello() {
        // Given
        let skill = GreetSkill;
        let csi = CsiDummy;
        let engine = Arc::new(Engine::default());
        let mut store = SkillStoreStub::new();
        store.with_fetch_response(Some(Arc::new(skill)));

        // When
        let runtime = SkillRuntime::new(engine, csi, store);
        let result = runtime
            .api()
            .run_function(
                SkillPath::local("greet"),
                json!(""),
                "TOKEN_NOT_REQUIRED".to_owned(),
                TracingContext::dummy(),
            )
            .await;
        runtime.wait_for_shutdown().await;

        // Then
        assert_eq!(result.unwrap(), "Hello");
    }

    #[tokio::test]
    async fn concurrent_skill_execution() {
        // Given
        struct SkillAssertConcurrent {
            send: broadcast::Sender<()>,
        }

        #[async_trait]
        impl Skill for SkillAssertConcurrent {
            async fn run_as_function(
                &self,
                _engine: &Engine,
                _ctx: Box<dyn CsiForSkills + Send>,
                _input: Value,
                _tracing_context: &TracingContext,
            ) -> Result<Value, SkillError> {
                let mut recv = self.send.subscribe();
                self.send.send(()).unwrap();
                // Send once await two responses. This way we can only finish any skill if we
                // actually execute them concurrently. Two for the two invocations in this test
                recv.recv().await.unwrap();
                recv.recv().await.unwrap();
                // We finished, lets unblock our counterpart, in case it missed a broadcast
                self.send.send(()).unwrap();
                Ok(json!("Hello"))
            }

            async fn manifest(
                &self,
                _engine: &Engine,
                _ctx: Box<dyn CsiForSkills + Send>,
                _tracing_context: &TracingContext,
            ) -> Result<AnySkillManifest, SkillError> {
                panic!("Dummy metadata implementation of Assert concurrency skill")
            }

            async fn run_as_message_stream(
                &self,
                _engine: &Engine,
                _ctx: Box<dyn CsiForSkills + Send>,
                _input: Value,
                _mpsc: mpsc::Sender<SkillEvent>,
                _tracing_context: &TracingContext,
            ) -> Result<(), SkillError> {
                panic!("Dummy message stream implementation of Assert concurrency skill")
            }
        }

        let (send, _recv) = broadcast::channel(2);
        let skill = SkillAssertConcurrent { send: send.clone() };
        let engine = Arc::new(Engine::default());
        let mut store = SkillStoreStub::new();
        store.with_fetch_response(Some(Arc::new(skill)));
        let runtime = SkillRuntime::new(engine, CsiDummy, store);

        // When invoking two skills in parallel
        let token = "TOKEN_NOT_REQUIRED";

        let api_first = runtime.api();
        let first = tokio::spawn(async move {
            api_first
                .run_function(
                    SkillPath::local("any_path"),
                    json!({}),
                    token.to_owned(),
                    TracingContext::dummy(),
                )
                .await
        });
        let api_second = runtime.api();
        let second = tokio::spawn(async move {
            api_second
                .run_function(
                    SkillPath::local("any_path"),
                    json!({}),
                    token.to_owned(),
                    TracingContext::dummy(),
                )
                .await
        });
        let result_first = tokio::time::timeout(Duration::from_secs(1), first).await;
        let result_second = tokio::time::timeout(Duration::from_secs(1), second).await;

        assert!(result_first.is_ok());
        assert!(result_second.is_ok());

        runtime.wait_for_shutdown().await;
    }

    #[tokio::test]
    async fn stream_hello_test() {
        // Given
        let engine = Arc::new(Engine::default());
        let mut store = SkillStoreStub::new();
        store.with_fetch_response(Some(Arc::new(SkillHello)));
        let runtime = SkillRuntime::new(engine, CsiDummy, store);

        // When
        let mut recv = runtime
            .api()
            .run_message_stream(
                SkillPath::new(Namespace::new("test-beta").unwrap(), "hello"),
                json!(""),
                "TOKEN_NOT_REQUIRED".to_owned(),
                TracingContext::dummy(),
            )
            .await;

        // Then
        assert_eq!(
            recv.recv().await.unwrap(),
            SkillExecutionEvent::MessageBegin
        );
        assert_eq!(
            recv.recv().await.unwrap(),
            SkillExecutionEvent::MessageAppend {
                text: "H".to_string()
            }
        );
        assert_eq!(
            recv.recv().await.unwrap(),
            SkillExecutionEvent::MessageAppend {
                text: "e".to_string()
            }
        );
        assert_eq!(
            recv.recv().await.unwrap(),
            SkillExecutionEvent::MessageAppend {
                text: "l".to_string()
            }
        );
        assert_eq!(
            recv.recv().await.unwrap(),
            SkillExecutionEvent::MessageAppend {
                text: "l".to_string()
            }
        );
        assert_eq!(
            recv.recv().await.unwrap(),
            SkillExecutionEvent::MessageAppend {
                text: "o".to_string()
            }
        );
        assert_eq!(
            recv.recv().await.unwrap(),
            SkillExecutionEvent::MessageEnd {
                payload: json!(null)
            }
        );
        assert!(recv.recv().await.is_none());

        // Cleanup
        runtime.wait_for_shutdown().await;
    }

    #[tokio::test]
    async fn stream_saboteur_test() {
        // Given
        let engine = Arc::new(Engine::default());
        let mut store = SkillStoreStub::new();
        store.with_fetch_response(Some(Arc::new(SkillSaboteur)));
        let runtime = SkillRuntime::new(engine, CsiDummy, store);

        // When
        let mut recv = runtime
            .api()
            .run_message_stream(
                SkillPath::new(Namespace::new("test-beta").unwrap(), "saboteur"),
                json!(""),
                "TOKEN_NOT_REQUIRED".to_owned(),
                TracingContext::dummy(),
            )
            .await;

        // Then
        assert_eq!(
            recv.recv().await.unwrap(),
            SkillExecutionEvent::Error(SkillExecutionError::UserCode(
                "Skill is a saboteur".to_owned()
            ))
        );
        assert!(recv.recv().await.is_none());

        // Cleanup
        runtime.wait_for_shutdown().await;
    }

    #[test]
    fn skill_runtime_metrics_emitted() {
        let skill_path = SkillPath::local("greet");
        let mut store = SkillStoreStub::new();
        store.with_fetch_response(Some(Arc::new(GreetSkill)));
        let engine = Arc::new(Engine::default());
        let (send, _) = oneshot::channel();
        let msg = RunFunctionMsg {
            skill_path: skill_path.clone(),
            input: json!("Hello"),
            send,
            api_token: "dummy".to_owned(),
            tracing_context: TracingContext::dummy(),
        };

        // Metrics requires sync, so all of the async parts are moved into this closure.
        let snapshot = metrics_snapshot(async || {
            let runtime = SkillDriver::new(engine);
            msg.act(CsiDummy, &runtime, &store).await;
            drop(runtime);
        });

        let metrics = snapshot.into_vec();
        let expected_labels = [
            &Label::new("namespace", skill_path.namespace.to_string()),
            &Label::new("name", skill_path.name),
            &Label::new("status", "ok"),
        ];
        assert!(metrics.iter().any(|(key, _, _, value)| {
            let key = key.key();
            let labels = key.labels().collect::<Vec<_>>();
            key.name() == "kernel_skill_execution_total"
                && labels == expected_labels
                && value == &DebugValue::Counter(1)
        }));
        assert!(metrics.iter().any(|(key, _, _, _)| {
            let key = key.key();
            let labels = key.labels().collect::<Vec<_>>();
            key.name() == "kernel_skill_execution_duration_seconds" && labels == expected_labels
        }));
    }

    #[tokio::test]
    async fn stream_skill_should_emit_error_in_case_of_runtime_error_in_csi() {
        // Given
        let engine = Arc::new(Engine::default());
        let mut store = SkillStoreStub::new();
        store.with_fetch_response(Some(Arc::new(SkillTellMeAJoke)));
        let runtime = SkillRuntime::new(engine, CsiSaboteur, store);
        let skill_path = SkillPath::new(Namespace::new("test-beta").unwrap(), "tell_me_a_joke");

        // When
        let mut recv = runtime
            .api()
            .run_message_stream(
                skill_path,
                json!({}),
                "dumm_token".to_owned(),
                TracingContext::dummy(),
            )
            .await;

        // Then
        let event = recv.recv().await.unwrap();
        assert_eq!(
            event,
            SkillExecutionEvent::Error(SkillExecutionError::RuntimeError("Test error".to_owned()))
        );
    }

    fn metrics_snapshot<F: Future<Output = ()>>(f: impl FnOnce() -> F) -> Snapshot {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        metrics::with_local_recorder(&recorder, || runtime.block_on(f()));
        snapshotter.snapshot()
    }

    /// A skill implementation for testing purposes. It sends a greeting to the user.
    struct GreetSkill;

    #[async_trait]
    impl Skill for GreetSkill {
        async fn run_as_function(
            &self,
            _engine: &Engine,
            _ctx: Box<dyn CsiForSkills + Send>,
            _input: Value,
            _tracing_context: &TracingContext,
        ) -> Result<Value, SkillError> {
            Ok(json!("Hello"))
        }

        async fn manifest(
            &self,
            _engine: &Engine,
            _ctx: Box<dyn CsiForSkills + Send>,
            _tracing_context: &TracingContext,
        ) -> Result<AnySkillManifest, SkillError> {
            panic!("Dummy metadata implementation of Greet Skill")
        }

        async fn run_as_message_stream(
            &self,
            _engine: &Engine,
            _ctx: Box<dyn CsiForSkills + Send>,
            _input: Value,
            _sender: mpsc::Sender<SkillEvent>,
            _tracing_context: &TracingContext,
        ) -> Result<(), SkillError> {
            Err(SkillError::IsFunction)
        }
    }
}
