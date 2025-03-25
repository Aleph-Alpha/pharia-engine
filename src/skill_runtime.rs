use std::{borrow::Cow, future::Future, pin::Pin, sync::Arc, time::Instant};

use anyhow::anyhow;
use async_trait::async_trait;
use futures::{StreamExt, stream::FuturesUnordered};
use serde_json::Value;
use thiserror::Error;
use tokio::{
    select,
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use crate::{
    csi::Csi,
    namespace_watcher::Namespace,
    skill_driver::SkillDriver,
    skill_store::{SkillStoreApi, SkillStoreError},
    skills::{AnySkillManifest, Engine, Skill, SkillError, SkillPath},
};

impl From<SkillError> for SkillExecutionError {
    fn from(source: SkillError) -> Self {
        match source {
            SkillError::Any(error) => Self::UserCode(error.to_string()),
            SkillError::InvalidInput(error) => Self::InvalidInput(error),
            SkillError::UserCode(error) => Self::UserCode(error),
            SkillError::InvalidOutput(error) => Self::InvalidOutput(error),
            SkillError::RuntimeError(error) => SkillExecutionError::RuntimeError(error),
            SkillError::IsFunction => SkillExecutionError::IsFunction,
            SkillError::IsMessageStream => SkillExecutionError::IsMessageStream,
        }
    }
}

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
        let runtime = SkillDriver::new(engine);
        let (send, recv) = mpsc::channel::<SkillRuntimeMsg>(1);
        let handle = tokio::spawn(async {
            SkillRuntimeActor::new(runtime, store, recv, csi_apis)
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
#[async_trait]
pub trait SkillRuntimeApi {
    async fn run_function(
        &self,
        skill_path: SkillPath,
        input: Value,
        api_token: String,
    ) -> Result<Value, SkillExecutionError>;

    async fn run_message_stream(
        &self,
        skill_path: SkillPath,
        input: Value,
        api_token: String,
    ) -> mpsc::Receiver<StreamEvent>;

    async fn skill_metadata(
        &self,
        skill_path: SkillPath,
    ) -> Result<AnySkillManifest, SkillExecutionError>;
}

#[async_trait]
impl SkillRuntimeApi for mpsc::Sender<SkillRuntimeMsg> {
    async fn run_function(
        &self,
        skill_path: SkillPath,
        input: Value,
        api_token: String,
    ) -> Result<Value, SkillExecutionError> {
        let (send, recv) = oneshot::channel();
        let msg = SkillRuntimeMsg::Function(RunFunctionMsg {
            skill_path,
            input,
            send,
            api_token,
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
    ) -> mpsc::Receiver<StreamEvent> {
        let (send, recv) = mpsc::channel::<StreamEvent>(1);

        let msg = RunMessageStreamMsg {
            skill_path,
            input,
            send,
            api_token,
        };

        self.send(SkillRuntimeMsg::MessageStream(msg))
            .await
            .expect("all api handlers must be shutdown before actors");
        recv
    }

    async fn skill_metadata(
        &self,
        skill_path: SkillPath,
    ) -> Result<AnySkillManifest, SkillExecutionError> {
        let (send, recv) = oneshot::channel();
        let msg = SkillRuntimeMsg::Metadata(MetadataMsg { skill_path, send });
        self.send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await.unwrap()
    }
}

/// Errors which may prevent a skill from executing to completion successfully.
#[derive(Debug, Error)]
pub enum SkillExecutionError {
    #[error(
        "The metadata function of the invoked skill is bugged. It is forbidden to invoke any CSI \
        functions from the metadata function, yet the skill does precisely this."
    )]
    CsiUseFromMetadata,
    /// A skill name is not mentioned in the namespace and therfore it is not served. This is a
    /// logic error. Yet it does not originate in the skill code itself. It could be an error in the
    /// request by the user, or a missing configuration at the side of the skill developer.
    #[error(
        "Sorry, We could not find the skill you requested in its namespace. This can have three \
        causes:\n\n\
        1. You send the wrong skill name.\n\
        2. You send the wrong namespace.\n\
        3. The skill is not configured in the namespace you requested. You may want to check the \
        namespace configuration."
    )]
    SkillNotConfigured,
    /// Skill Logic errors are logic errors which are reported by the skill code itself. These may
    /// be due to bugs in the skill code, or invalid user input, we will not be able to tell. For
    /// the operater these are both user errors. The skill user and developer are often the same
    /// person so in either case we do well to report it in our answer.
    #[error(
        "The skill you called responded with an error. Maybe you should check your input, if it \
        seems to be correct you may want to contact the skill developer. Error reported by Skill:\n\
        \n{0}"
    )]
    UserCode(String),
    /// Skills are still responsible for parsing the input bytes. Our SDK emits a specific error if
    /// the input is not interpretable.
    #[error("The skill had trouble interpreting the input:\n\n{0}")]
    InvalidInput(String),
    #[error(
        "The skill returned an output the Kernel could not interpret. This hints to a bug in the \
        skill. Please contact its developer. Caused by: \n\n{0}"
    )]
    InvalidOutput(String),
    /// A runtime error is caused by the runtime, i.e. the dependencies and resources we need to be
    /// available in order execute skills. For us this are mostly the drivers behind the CSI. This
    /// does not mean that all CSI errors are runtime errors, because the Inference may e.g. be
    /// available, but the prompt exceeds the maximum length, etc. Yet any error caused by spurious
    /// network errors, inference being to busy, etc. will fall in this category. For runtime errors
    /// it is most important to report them to the operator, so they can take action.
    ///
    /// For our other **users** we should not forward them transparently, but either just tell them
    /// something went wrong, or give appropriate context. E.g. whether it was inference or document
    /// index which caused the error.
    #[error(
        "The skill could not be executed to completion, something in our runtime is currently \n\
        unavailable or misconfigured. You should try again later, if the situation persists you \n\
        may want to contact the operaters. Original error:\n\n{0}"
    )]
    RuntimeError(#[source] anyhow::Error),
    /// This happens if a configuration for an individual namespace is broken. For the user calling
    /// the route to execute a skill, we treat this as a runtime, but make sure he gets all the
    /// context, because it very likely might be the skill developer who misconfigured the
    /// namespace. For the operater team, operating all of Pharia Kernel we treat this as a logic
    /// error, because there is nothing wrong about the kernel installion or inference, or network
    /// or other stuff, which they would be able to fix.
    #[error(
        "The skill could not be executed to completion, the namespace '{namespace}' is \
        misconfigured. If you are the developer who configured the skill, you should probably fix \
        this error. If you are not, there is nothing you can do, until the developer who maintains \
        the list of skills to be served, fixes this. Original Syntax error:\n\n\
        {original_syntax_error}"
    )]
    MisconfiguredNamespace {
        namespace: Namespace,
        original_syntax_error: String,
    },
    #[error(
        "The skill is designed to be executed as a function. Please invoke it via the /run endpoint."
    )]
    IsFunction,
    #[error("The skill is designed to stream output. Please invoke it via the /stream endpoint.")]
    IsMessageStream,
}

struct SkillRuntimeActor<C, S> {
    runtime: Arc<SkillDriver>,
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
        runtime: SkillDriver,
        store: S,
        recv: mpsc::Receiver<SkillRuntimeMsg>,
        csi_apis: C,
    ) -> Self {
        SkillRuntimeActor {
            runtime: Arc::new(runtime),
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
                        let runtime = self.runtime.clone();
                        let store = self.store.clone();
                        self.running_requests.push(Box::pin(async move {
                            msg.act(csi_apis, runtime.as_ref(), store.as_ref()).await;
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
    SkillExecutionTotal,
    SkillExecutionDurationSeconds,
}

impl From<SkillRuntimeMetrics> for metrics::KeyName {
    fn from(value: SkillRuntimeMetrics) -> Self {
        Self::from_const_str(match value {
            SkillRuntimeMetrics::SkillExecutionTotal => "kernel_skill_execution_total",
            SkillRuntimeMetrics::SkillExecutionDurationSeconds => {
                "kernel_skill_execution_duration_seconds"
            }
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
        runtime: &SkillDriver,
        store: &impl SkillStoreApi,
    ) {
        match self {
            SkillRuntimeMsg::MessageStream(msg) => {
                msg.act(csi_apis, runtime, store).await;
            }
            SkillRuntimeMsg::Function(msg) => {
                msg.act(csi_apis, runtime, store).await;
            }
            SkillRuntimeMsg::Metadata(msg) => {
                msg.act(runtime, store).await;
            }
        }
    }
}

#[derive(Debug)]
pub struct MetadataMsg {
    pub skill_path: SkillPath,
    pub send: oneshot::Sender<Result<AnySkillManifest, SkillExecutionError>>,
}

impl MetadataMsg {
    pub async fn act(self, runtime: &SkillDriver, store: &impl SkillStoreApi) {
        let skill_result = fetch_skill(store, &self.skill_path).await;
        let skill = match skill_result {
            Ok(skill) => skill,
            Err(e) => {
                drop(self.send.send(Err(e)));
                return;
            }
        };
        let result = runtime.metadata(skill).await;
        drop(self.send.send(result));
    }
}

#[derive(Debug)]
pub struct RunMessageStreamMsg {
    pub skill_path: SkillPath,
    pub input: Value,
    pub send: mpsc::Sender<StreamEvent>,
    pub api_token: String,
}

impl RunMessageStreamMsg {
    async fn act(
        self,
        csi_apis: impl Csi + Send + Sync + 'static,
        runtime: &SkillDriver,
        store: &impl SkillStoreApi,
    ) {
        let RunMessageStreamMsg {
            skill_path,
            input,
            send,
            api_token,
        } = self;

        let skill_result = fetch_skill(store, &skill_path).await;
        let skill = match skill_result {
            Ok(skill) => skill,
            Err(e) => {
                drop(send.send(StreamEvent::Error(e.to_string())).await);
                return;
            }
        };

        let start = Instant::now();
        let result = runtime
            .run_message_stream(skill, input, csi_apis, api_token, send.clone())
            .await;
        let label = status_label(result.as_ref().err());

        record_skill_metrics(start, skill_path, label);
    }
}

async fn fetch_skill(
    store: &impl SkillStoreApi,
    skill_path: &SkillPath,
) -> Result<Arc<dyn Skill>, SkillExecutionError> {
    match store.fetch(skill_path.clone()).await {
        Ok(Some(skill)) => Ok(skill),
        Ok(None) => Err(SkillExecutionError::SkillNotConfigured),
        Err(e) => Err(e.into()),
    }
}

impl From<SkillStoreError> for SkillExecutionError {
    fn from(source: SkillStoreError) -> Self {
        match source {
            SkillStoreError::SkillLoaderError(skill_loader_error) => {
                SkillExecutionError::RuntimeError(anyhow!(
                    "Error loading skill: {}",
                    skill_loader_error
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

fn record_skill_metrics(start: Instant, skill_path: SkillPath, status: String) {
    let latency = start.elapsed().as_secs_f64();
    let labels = [
        ("namespace", Cow::from(skill_path.namespace.to_string())),
        ("name", Cow::from(skill_path.name)),
        ("status", status.into()),
    ];
    metrics::counter!(SkillRuntimeMetrics::SkillExecutionTotal, &labels).increment(1);
    metrics::histogram!(SkillRuntimeMetrics::SkillExecutionDurationSeconds, &labels)
        .record(latency);
}

fn status_label(result: Option<&SkillExecutionError>) -> String {
    match result {
        None => "ok",
        Some(
            SkillExecutionError::UserCode(_)
            | SkillExecutionError::CsiUseFromMetadata
            | SkillExecutionError::SkillNotConfigured
            | SkillExecutionError::InvalidInput(_)
            | SkillExecutionError::InvalidOutput(_)
            | SkillExecutionError::MisconfiguredNamespace { .. }
            | SkillExecutionError::IsFunction
            | SkillExecutionError::IsMessageStream,
        ) => "logic_error",
        Some(SkillExecutionError::RuntimeError(_)) => "runtime_error",
    }
    .to_owned()
}

/// An event emitted by a streaming skill
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamEvent {
    /// Send at the beginning of each message, currently carries no information. May be used in the
    /// future to communicate the role. Can also be useful to the UI to communicate that its about
    /// time to start rendering that speech bubble.
    MessageBegin,
    /// Send at the end of each message. Can carry an arbitrary payload, to make messages more of a
    /// dropin for classical functions. Might be refined in the future. We anticipate the stop
    /// reason to be very useful for end appliacations. We also introduce end messages to keep the
    /// door open for multiple messages in a stream.
    MessageEnd { payload: Value },
    /// Append the internal string to the current message
    MessageAppend { text: String },
    /// An error occurred during skill execution. This kind of error can happen after streaming has
    /// started
    Error(String),
}

/// Message type used to transfer the input and output of a function skill execution
#[derive(Debug)]
pub struct RunFunctionMsg {
    pub skill_path: SkillPath,
    pub input: Value,
    pub send: oneshot::Sender<Result<Value, SkillExecutionError>>,
    pub api_token: String,
}

impl RunFunctionMsg {
    async fn act(
        self,
        csi_apis: impl Csi + Send + Sync + 'static,
        runtime: &SkillDriver,
        store: &impl SkillStoreApi,
    ) {
        let RunFunctionMsg {
            skill_path,
            input,
            send,
            api_token,
        } = self;

        let skill_result = fetch_skill(store, &skill_path).await;
        let skill = match skill_result {
            Ok(skill) => skill,
            Err(e) => {
                drop(send.send(Err(e)));
                return;
            }
        };

        let start = Instant::now();
        let response = runtime
            .run_function(skill, input, csi_apis, api_token)
            .await;

        let status = status_label(response.as_ref().err());
        // Error is expected to happen during shutdown. Ignore result.
        drop(send.send(response));
        record_skill_metrics(start, skill_path, status);
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
        skills::{AnySkillManifest, Skill},
    };
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
        let skill_actor = SkillRuntime::new(Arc::new(Engine::new(false).unwrap()), CsiDummy, store);

        // When asking the skill actor to run the skill
        let result = skill_actor
            .api()
            .run_function(SkillPath::dummy(), json!(""), "dummy_token".to_owned())
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
        let engine = Arc::new(Engine::new(false).unwrap());
        let registry_config = RegistryConfig::empty();
        let skill_loader = SkillLoader::from_config(engine.clone(), registry_config).api();

        let skill_store = SkillStore::new(skill_loader, Duration::from_secs(10));
        let csi_apis = CsiDummy;
        let executer = SkillRuntime::new(engine, csi_apis, skill_store.api());
        let api = executer.api();

        // When a skill is requested, but it is not listed in the namespace
        let result = api
            .run_function(
                SkillPath::local("my_skill"),
                json!("Any input"),
                "Dummy api token".to_owned(),
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
        let engine = Arc::new(Engine::new(false).unwrap());
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
            ) -> Result<AnySkillManifest, SkillError> {
                panic!("Dummy metadata implementation of Assert concurrency skill")
            }

            async fn run_as_message_stream(
                &self,
                _engine: &Engine,
                _ctx: Box<dyn CsiForSkills + Send>,
                _input: Value,
                _mpsc: mpsc::Sender<StreamEvent>,
            ) -> Result<(), SkillError> {
                panic!("Dummy message stream implementation of Assert concurrency skill")
            }
        }

        let (send, _recv) = broadcast::channel(2);
        let skill = SkillAssertConcurrent { send: send.clone() };
        let engine = Arc::new(Engine::new(false).unwrap());
        let mut store = SkillStoreStub::new();
        store.with_fetch_response(Some(Arc::new(skill)));
        let runtime = SkillRuntime::new(engine, CsiDummy, store);

        // When invoking two skills in parallel
        let token = "TOKEN_NOT_REQUIRED";

        let api_first = runtime.api();
        let first = tokio::spawn(async move {
            api_first
                .run_function(SkillPath::local("any_path"), json!({}), token.to_owned())
                .await
        });
        let api_second = runtime.api();
        let second = tokio::spawn(async move {
            api_second
                .run_function(SkillPath::local("any_path"), json!({}), token.to_owned())
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
        let engine = Arc::new(Engine::new(false).unwrap());
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
            )
            .await;

        // Then
        assert_eq!(recv.recv().await.unwrap(), StreamEvent::MessageBegin);
        assert_eq!(
            recv.recv().await.unwrap(),
            StreamEvent::MessageAppend {
                text: "H".to_string()
            }
        );
        assert_eq!(
            recv.recv().await.unwrap(),
            StreamEvent::MessageAppend {
                text: "e".to_string()
            }
        );
        assert_eq!(
            recv.recv().await.unwrap(),
            StreamEvent::MessageAppend {
                text: "l".to_string()
            }
        );
        assert_eq!(
            recv.recv().await.unwrap(),
            StreamEvent::MessageAppend {
                text: "l".to_string()
            }
        );
        assert_eq!(
            recv.recv().await.unwrap(),
            StreamEvent::MessageAppend {
                text: "o".to_string()
            }
        );
        assert_eq!(
            recv.recv().await.unwrap(),
            StreamEvent::MessageEnd {
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
        let engine = Arc::new(Engine::new(false).unwrap());
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
            )
            .await;

        // Then
        let expected_error_msg = "The skill you called responded with an error. Maybe you should \
            check your input, if it seems to be correct you may want to contact the skill \
            developer. Error reported by Skill:\n\nSkill is a saboteur";
        assert_eq!(
            recv.recv().await.unwrap(),
            StreamEvent::Error(expected_error_msg.to_string())
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
        let engine = Arc::new(Engine::new(false).unwrap());
        let (send, _) = oneshot::channel();
        let msg = RunFunctionMsg {
            skill_path: skill_path.clone(),
            input: json!("Hello"),
            send,
            api_token: "dummy".to_owned(),
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
        let engine = Arc::new(Engine::new(false).unwrap());
        let mut store = SkillStoreStub::new();
        store.with_fetch_response(Some(Arc::new(SkillTellMeAJoke)));
        let runtime = SkillRuntime::new(engine, CsiSaboteur, store);
        let skill_path = SkillPath::new(Namespace::new("test-beta").unwrap(), "tell_me_a_joke");

        // When
        let mut recv = runtime
            .api()
            .run_message_stream(skill_path, json!({}), "dumm_token".to_owned())
            .await;

        // Then
        let event = recv.recv().await.unwrap();
        let expected_error_msg = "The skill could not be executed to completion, something in our \
            runtime is currently \nunavailable or misconfigured. You should try again later, if \
            the situation persists you \nmay want to contact the operaters. Original error:\n\n\
            Test error";
        assert_eq!(event, StreamEvent::Error(expected_error_msg.to_string()));
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
        ) -> Result<Value, SkillError> {
            Ok(json!("Hello"))
        }

        async fn manifest(
            &self,
            _engine: &Engine,
            _ctx: Box<dyn CsiForSkills + Send>,
        ) -> Result<AnySkillManifest, SkillError> {
            panic!("Dummy metadata implementation of Greet Skill")
        }

        async fn run_as_message_stream(
            &self,
            _engine: &Engine,
            _ctx: Box<dyn CsiForSkills + Send>,
            _input: Value,
            _sender: mpsc::Sender<StreamEvent>,
        ) -> Result<(), SkillError> {
            Err(SkillError::IsFunction)
        }
    }
}
